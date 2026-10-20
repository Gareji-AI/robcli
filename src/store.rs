use clap::{Args, ValueEnum};
use reqwest::Url;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};

use crate::error::AppError;
use crate::traffic::{Clock, TrafficGuard};
use crate::transport::{Request, Transport};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, ValueEnum)]
pub(crate) enum AssetKind {
    #[default]
    Audio,
    #[value(alias = "models")]
    Model,
    #[value(alias = "meshes", alias = "mesh-part")]
    Mesh,
    #[value(alias = "image", alias = "images", alias = "decals")]
    Decal,
    #[value(alias = "plugins")]
    Plugin,
    #[value(alias = "videos")]
    Video,
    #[value(alias = "fonts", alias = "font-family")]
    Font,
    #[value(alias = "places")]
    Place,
}

impl AssetKind {
    fn api(self) -> &'static str {
        match self {
            Self::Audio => "Audio",
            Self::Model => "Model",
            Self::Mesh => "MeshPart",
            Self::Decal => "Decal",
            Self::Plugin => "Plugin",
            Self::Video => "Video",
            Self::Font => "FontFamily",
            Self::Place => "Place",
        }
    }
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub(crate) enum AudioKind {
    Music,
    #[value(alias = "sound-effect")]
    Sfx,
}
impl AudioKind {
    fn api(self) -> &'static str {
        match self {
            Self::Music => "Music",
            Self::Sfx => "SoundEffect",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, ValueEnum)]
pub(crate) enum Sort {
    #[default]
    Relevance,
    Trending,
    Top,
    Duration,
    Created,
    Updated,
    Rating,
}
impl Sort {
    fn api(self) -> &'static str {
        match self {
            Self::Relevance => "Relevance",
            Self::Trending => "Trending",
            Self::Top => "Top",
            Self::Duration => "AudioDuration",
            Self::Created => "CreateTime",
            Self::Updated => "UpdatedTime",
            Self::Rating => "Ratings",
        }
    }
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub(crate) enum Direction {
    Asc,
    Desc,
}
impl Direction {
    fn api(self) -> &'static str {
        match self {
            Self::Asc => "Ascending",
            Self::Desc => "Descending",
        }
    }
}

#[derive(Debug, Default, Args)]
pub(crate) struct Search {
    /// Creator Store category. Avatar Marketplace is a different catalog.
    #[arg(long = "type", value_enum)]
    pub kind: AssetKind,
    #[arg(long, short = 'q')]
    pub query: String,
    /// Music for BGM; sfx for sound effects. Audio category only.
    #[arg(long, value_enum)]
    pub audio_type: Option<AudioKind>,
    #[arg(long, value_parser = clap::value_parser!(u32).range(..=2147483647))]
    pub min_duration: Option<u32>,
    #[arg(long, value_parser = clap::value_parser!(u32).range(1..=2147483647))]
    pub max_duration: Option<u32>,
    #[arg(long)]
    pub artist: Option<String>,
    #[arg(long)]
    pub album: Option<String>,
    #[arg(long, conflicts_with = "creator_group", value_parser = clap::value_parser!(u64).range(1..=9223372036854775807))]
    pub creator_user: Option<u64>,
    #[arg(long, conflicts_with = "creator_user", value_parser = clap::value_parser!(u64).range(1..=9223372036854775807))]
    pub creator_group: Option<u64>,
    /// Publisher badge filter, not a license or script-safety guarantee.
    #[arg(long)]
    pub verified_only: bool,
    #[arg(long, conflicts_with = "max_price_cents")]
    pub free: bool,
    #[arg(long, value_parser = clap::value_parser!(u32).range(..=2147483647))]
    pub max_price_cents: Option<u32>,
    #[arg(long, value_enum, default_value = "relevance")]
    pub sort: Sort,
    #[arg(long, value_enum)]
    pub order: Option<Direction>,
    /// One explicit next page only; no automatic pagination or detail fan-out.
    #[arg(long)]
    pub page_token: Option<String>,
    #[arg(long, default_value = "10", value_parser = clap::value_parser!(u32).range(1..=100))]
    pub limit: u32,
}

fn invalid() -> AppError {
    AppError::new(
        "store_search_invalid",
        "The Creator Store search filters are invalid.",
        "Provide a short nonempty query, a supported type, compatible audio filters, an ordered duration range, one creator and a page size from 1 to 100. No credential lookup was performed.",
    )
}
fn text_ok(value: &str, max: usize) -> bool {
    !value.trim().is_empty() && value.len() <= max && !value.chars().any(char::is_control)
}

impl Search {
    pub fn url(&self) -> Result<Url, AppError> {
        let has_audio_filter = self.audio_type.is_some()
            || self.min_duration.is_some()
            || self.max_duration.is_some()
            || self.artist.is_some()
            || self.album.is_some()
            || matches!(self.sort, Sort::Duration);
        if !text_ok(&self.query, 512)
            || !(1..=100).contains(&self.limit)
            || (self.kind != AssetKind::Audio && has_audio_filter)
            || self
                .min_duration
                .zip(self.max_duration)
                .is_some_and(|(min, max)| min > max)
            || self.max_duration == Some(0)
            || [self.min_duration, self.max_duration, self.max_price_cents]
                .into_iter()
                .flatten()
                .any(|value| value > i32::MAX as u32)
            || self.artist.as_ref().is_some_and(|s| !text_ok(s, 200))
            || self.album.as_ref().is_some_and(|s| !text_ok(s, 200))
            || self.page_token.as_ref().is_some_and(|s| !text_ok(s, 4096))
            || (self.creator_user.is_some() && self.creator_group.is_some())
            || [self.creator_user, self.creator_group]
                .into_iter()
                .flatten()
                .any(|id| id == 0 || id > i64::MAX as u64)
            || (self.free && self.max_price_cents.is_some())
        {
            return Err(invalid());
        }
        let mut url =
            Url::parse("https://apis.roblox.com/toolbox-service/v2/assets:search").unwrap();
        {
            let mut q = url.query_pairs_mut();
            q.append_pair("searchCategoryType", self.kind.api());
            q.append_pair("query", &self.query);
            q.append_pair("maxPageSize", &self.limit.to_string());
            q.append_pair("searchView", "Full");
            q.append_pair("sortCategory", self.sort.api());
            q.append_pair(
                "includeOnlyVerifiedCreators",
                if self.verified_only { "true" } else { "false" },
            );
            if let Some(value) = self.audio_type {
                q.append_pair("audioTypes", value.api());
            }
            if let Some(value) = self.min_duration {
                q.append_pair("audioMinDurationSeconds", &value.to_string());
            }
            if let Some(value) = self.max_duration {
                q.append_pair("audioMaxDurationSeconds", &value.to_string());
            }
            if let Some(value) = &self.artist {
                q.append_pair("audioArtist", value);
            }
            if let Some(value) = &self.album {
                q.append_pair("audioAlbum", value);
            }
            if let Some(value) = self.creator_user {
                q.append_pair("userId", &value.to_string());
            }
            if let Some(value) = self.creator_group {
                q.append_pair("groupId", &value.to_string());
            }
            if self.free {
                q.append_pair("maxPriceCents", "0");
            }
            if let Some(value) = self.max_price_cents {
                q.append_pair("maxPriceCents", &value.to_string());
            }
            if let Some(value) = self.order {
                q.append_pair("sortDirection", value.api());
            }
            if let Some(value) = &self.page_token {
                q.append_pair("pageToken", value);
            }
        }
        Ok(url)
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SearchReply {
    creator_store_assets: Option<Vec<AssetReply>>,
    total_results: u64,
    next_page_token: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AssetReply {
    asset: Option<Asset>,
    creator: Option<Creator>,
    creator_store_product: Option<Product>,
    voting: Option<Voting>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Asset {
    id: u64,
    name: String,
    description: Option<String>,
    asset_type_id: Option<u32>,
    duration_seconds: Option<u32>,
    audio_type: Option<String>,
    artist: Option<String>,
    title: Option<String>,
    album: Option<String>,
    genre: Option<String>,
    category: Option<String>,
    subcategory: Option<String>,
    has_scripts: Option<bool>,
    script_count: Option<u32>,
    preview_assets: Option<Previews>,
    create_time: Option<String>,
    update_time: Option<String>,
    mesh_id: Option<u64>,
    texture_id: Option<u64>,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Creator {
    user_id: Option<u64>,
    group_id: Option<u64>,
    name: Option<String>,
    verified: Option<bool>,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Product {
    purchase_price: Option<Money>,
    purchasable: Option<bool>,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Money {
    currency_code: Option<String>,
    quantity: Option<Decimal>,
}
#[derive(Deserialize, Serialize)]
struct Decimal {
    significand: i64,
    exponent: i32,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Previews {
    image_preview_assets: Option<Vec<u64>>,
    video_preview_assets: Option<Vec<u64>>,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Voting {
    show_votes: Option<bool>,
    up_votes: Option<u64>,
    down_votes: Option<u64>,
    vote_count: Option<u64>,
    up_vote_percent: Option<u32>,
}

fn bad_reply() -> AppError {
    AppError::network(
        "store_response_invalid",
        "The public Creator Store response did not match the expected schema. No raw body is printed.",
    )
}

pub(crate) fn links(asset: u64) -> Result<Value, AppError> {
    if asset == 0 || asset > i64::MAX as u64 {
        return Err(invalid());
    }
    Ok(
        json!({ "assetId": asset, "storeUrl": format!("https://create.roblox.com/store/asset/{asset}"),
        "previewPageUrl": format!("https://create.roblox.com/store/asset/{asset}"), "studioAssetId": format!("rbxassetid://{asset}"),
        "existsChecked": false, "usagePermissionVerified": false, "browserOpened": false, "automaticPlayback": false }),
    )
}

impl AssetReply {
    fn project(self) -> Result<Value, AppError> {
        let asset = self.asset.ok_or_else(bad_reply)?;
        if asset.id == 0 || asset.id > i64::MAX as u64 || asset.name.trim().is_empty() {
            return Err(bad_reply());
        }
        let mut card = links(asset.id)?;
        card["existsChecked"] = true.into();
        card["asset"] = serde_json::to_value(asset).map_err(|_| bad_reply())?;
        card["creator"] = serde_json::to_value(self.creator).map_err(|_| bad_reply())?;
        card["product"] =
            serde_json::to_value(self.creator_store_product).map_err(|_| bad_reply())?;
        card["voting"] = serde_json::to_value(self.voting).map_err(|_| bad_reply())?;
        Ok(card)
    }
}

pub(crate) struct Store<T: Transport> {
    pub transport: T,
}
impl<T: Transport> Store<T> {
    pub fn new(transport: T) -> Self {
        Self { transport }
    }
    fn read<R: DeserializeOwned, C: Clock>(
        &mut self,
        url: Url,
        guard: &mut TrafficGuard<C>,
    ) -> Result<R, AppError> {
        let request = Request::creator_store(url)?;
        guard.reserve()?;
        let response = self.transport.send(request)?;
        if response.challenge || !(200..300).contains(&response.status) {
            let code = if response.challenge {
                "store_verification_required"
            } else {
                match response.status {
                    300..=399 => "redirect_refused",
                    401 | 403 => "store_access_refused",
                    404 => "store_asset_not_found",
                    429 => "rate_limited",
                    500..=599 => "server_error",
                    _ => "store_request_failed",
                }
            };
            return Err(AppError::http(
                code,
                "The public Creator Store request was refused; no retry or credential fallback was attempted.",
                "Stop. Do not add cookies/API keys, switch accounts or probe alternate endpoints to bypass a refusal.",
                response.status,
                response.retry_after_seconds,
            ));
        }
        serde_json::from_slice(&response.body).map_err(|_| bad_reply())
    }

    pub fn search<C: Clock>(
        &mut self,
        query: &Search,
        guard: &mut TrafficGuard<C>,
    ) -> Result<Value, AppError> {
        let page: SearchReply = self.read(query.url()?, guard)?;
        let items = match page.creator_store_assets {
            Some(items) => items,
            None if page.total_results == 0 => Vec::new(),
            None => return Err(bad_reply()),
        };
        let next = page.next_page_token.filter(|s| !s.is_empty());
        if items.len() > query.limit as usize || next.as_ref().is_some_and(|s| !text_ok(s, 4096)) {
            return Err(bad_reply());
        }
        let cards: Vec<Value> = items
            .into_iter()
            .map(AssetReply::project)
            .collect::<Result<_, _>>()?;
        Ok(
            json!({ "query": query.query, "category": query.kind.api(), "requestedAudioType": query.audio_type.map(AudioKind::api),
            "count": cards.len(), "totalResults": page.total_results, "hasMore": next.is_some(), "nextPageToken": next,
            "assets": cards, "automaticPagination": false, "detailFanout": false }),
        )
    }

    pub fn get<C: Clock>(
        &mut self,
        asset: u64,
        guard: &mut TrafficGuard<C>,
    ) -> Result<Value, AppError> {
        links(asset)?;
        let item: AssetReply = self.read(
            Url::parse(&format!(
                "https://apis.roblox.com/toolbox-service/v2/assets/{asset}"
            ))
            .unwrap(),
            guard,
        )?;
        if item.asset.as_ref().map(|a| a.id) != Some(asset) {
            return Err(bad_reply());
        }
        item.project()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::traffic::tests::FakeClock;
    use crate::transport::{HttpResponse, Request, Transport};
    use serde_json::{Value, json};

    struct FakeTransport {
        body: Value,
        records: Vec<Value>,
        status: u16,
        challenge: bool,
    }
    impl Transport for FakeTransport {
        fn send(&mut self, request: Request) -> Result<HttpResponse, crate::error::AppError> {
            let mut record = request.snapshot();
            record["url"] = request.url().as_str().into();
            self.records.push(record);
            Ok(HttpResponse {
                status: self.status,
                body: serde_json::to_vec(&self.body).unwrap(),
                retry_after_seconds: None,
                csrf: None,
                challenge: self.challenge,
            })
        }
    }

    #[test]
    fn music_search_is_one_anonymous_full_view_request_with_no_fanout() {
        let root = tempfile::tempdir().unwrap();
        let mut guard =
            crate::traffic::TrafficGuard::acquire(root.path(), FakeClock::default()).unwrap();
        let search = Search {
            query: "ambient & private=not-a-parameter".into(),
            kind: AssetKind::Audio,
            audio_type: Some(AudioKind::Music),
            min_duration: Some(60),
            max_duration: Some(180),
            limit: 3,
            ..Search::default()
        };
        let mut store = Store::new(FakeTransport {
            body: json!({ "creatorStoreAssets": [{
            "asset": { "id": 77, "name": "Synthetic Ambient", "assetTypeId": 3, "durationSeconds": 120,
                "audioType": "Music", "artist": "Synthetic Artist", "apikeySecret": "must-not-echo" },
            "creator": { "name": "Synthetic Creator", "userId": 1, "verified": true }
        }], "totalResults": 20, "nextPageToken": "next-page" }),
            records: Vec::new(),
            status: 200,
            challenge: false,
        });
        let output = store.search(&search, &mut guard).unwrap();
        assert_eq!(store.transport.records.len(), 1);
        assert_eq!(store.transport.records[0]["cookieAttached"], false);
        assert_eq!(store.transport.records[0]["method"], "GET");
        let url = reqwest::Url::parse(store.transport.records[0]["url"].as_str().unwrap()).unwrap();
        assert_eq!(url.host_str(), Some("apis.roblox.com"));
        assert_eq!(url.path(), "/toolbox-service/v2/assets:search");
        let params: std::collections::HashMap<_, _> = url.query_pairs().collect();
        assert_eq!(params.get("audioTypes").unwrap(), "Music");
        assert_eq!(params.get("searchView").unwrap(), "Full");
        assert_eq!(params.get("audioMinDurationSeconds").unwrap(), "60");
        assert_eq!(
            params.get("query").unwrap(),
            "ambient & private=not-a-parameter"
        );
        assert!(!params.contains_key("private"));
        assert_eq!(output["hasMore"], true);
        assert_eq!(output["count"], 1);
        assert_eq!(output["assets"][0]["studioAssetId"], "rbxassetid://77");
        assert_eq!(output["assets"][0]["asset"]["durationSeconds"], 120);
        assert!(!output.to_string().contains("must-not-echo"));
    }

    #[test]
    fn model_detail_reports_scripts_and_exact_money_without_fetching_source_or_media() {
        let root = tempfile::tempdir().unwrap();
        let mut guard =
            crate::traffic::TrafficGuard::acquire(root.path(), FakeClock::default()).unwrap();
        let mut store = Store::new(FakeTransport {
            status: 200,
            challenge: false,
            records: Vec::new(),
            body: json!({
                "asset": { "id": 77, "name": "Synthetic Tree", "assetTypeId": 10, "hasScripts": true, "scriptCount": 2,
                    "downloadUrl": "https://untrusted.invalid/must-not-fetch", "apikeySecret": "must-not-echo" },
                "creator": { "groupId": 42, "name": "Synthetic Studio", "verified": false },
                "creatorStoreProduct": { "purchasable": true, "purchasePrice": { "currencyCode": "USD", "quantity": { "significand": 499, "exponent": -2 } } }
            }),
        });
        let result = store.get(77, &mut guard).unwrap();
        assert_eq!(result["asset"]["hasScripts"], true);
        assert_eq!(
            result["product"]["purchasePrice"]["quantity"]["significand"],
            499
        );
        assert_eq!(result["product"]["purchasePrice"]["currencyCode"], "USD");
        assert_eq!(result["usagePermissionVerified"], false);
        assert_eq!(result["creator"]["groupId"], 42);
        assert!(!result.to_string().contains("must-not-echo"));
        assert!(!result.to_string().contains("untrusted.invalid"));
        assert_eq!(store.transport.records.len(), 1);
        assert_eq!(store.transport.records[0]["cookieAttached"], false);
        assert_eq!(
            store.transport.records[0]["path"],
            "/toolbox-service/v2/assets/77"
        );
    }

    #[test]
    fn detail_id_mismatch_never_claims_the_requested_asset_was_read() {
        let root = tempfile::tempdir().unwrap();
        let mut guard =
            crate::traffic::TrafficGuard::acquire(root.path(), FakeClock::default()).unwrap();
        let mut store = Store::new(FakeTransport {
            status: 200,
            challenge: false,
            records: Vec::new(),
            body: json!({
                "asset": { "id": 88, "name": "Another asset" }
            }),
        });
        assert_eq!(
            store.get(77, &mut guard).unwrap_err().code,
            "store_response_invalid"
        );
        assert_eq!(store.transport.records.len(), 1);
    }

    #[test]
    fn public_refusals_never_retry_attach_credentials_or_echo_error_bodies() {
        for (status, challenge) in [
            (302, false),
            (401, false),
            (403, false),
            (404, false),
            (429, false),
            (500, false),
            (200, true),
        ] {
            let root = tempfile::tempdir().unwrap();
            let mut guard =
                crate::traffic::TrafficGuard::acquire(root.path(), FakeClock::default()).unwrap();
            let mut store = Store::new(FakeTransport {
                status,
                challenge,
                records: Vec::new(),
                body: json!({ "error": "must-not-echo" }),
            });
            let error = store.get(77, &mut guard).unwrap_err();
            assert_eq!(error.exit_code, 3);
            assert!(
                !serde_json::to_string(&error)
                    .unwrap()
                    .contains("must-not-echo")
            );
            assert_eq!(store.transport.records.len(), 1);
            assert_eq!(store.transport.records[0]["cookieAttached"], false);
            assert_eq!(store.transport.records[0]["csrfAttached"], false);
        }
    }

    #[test]
    fn all_eight_categories_and_sound_effect_filters_use_published_v2_parameters() {
        for (kind, expected) in [
            (AssetKind::Audio, "Audio"),
            (AssetKind::Model, "Model"),
            (AssetKind::Mesh, "MeshPart"),
            (AssetKind::Decal, "Decal"),
            (AssetKind::Plugin, "Plugin"),
            (AssetKind::Video, "Video"),
            (AssetKind::Font, "FontFamily"),
            (AssetKind::Place, "Place"),
        ] {
            let query = Search {
                kind,
                query: "synthetic".into(),
                limit: 5,
                ..Search::default()
            };
            let url = query.url().unwrap();
            assert!(
                url.query_pairs()
                    .any(|(key, value)| key == "searchCategoryType" && value == expected)
            );
        }
        let query = Search {
            query: "coin".into(),
            limit: 5,
            audio_type: Some(AudioKind::Sfx),
            max_duration: Some(5),
            creator_group: Some(42),
            free: true,
            ..Search::default()
        };
        let url = query.url().unwrap();
        let params: std::collections::HashMap<_, _> = url.query_pairs().collect();
        assert_eq!(params.get("audioTypes").unwrap(), "SoundEffect");
        assert_eq!(params.get("groupId").unwrap(), "42");
        assert_eq!(params.get("maxPriceCents").unwrap(), "0");
    }

    #[test]
    fn incompatible_or_unbounded_filters_stop_before_any_request() {
        let valid = || Search {
            query: "synthetic".into(),
            limit: 10,
            ..Search::default()
        };
        let invalid_queries = [
            Search {
                kind: AssetKind::Model,
                audio_type: Some(AudioKind::Music),
                ..valid()
            },
            Search {
                min_duration: Some(60),
                max_duration: Some(5),
                ..valid()
            },
            Search {
                limit: 101,
                ..valid()
            },
            Search {
                query: " ".into(),
                ..valid()
            },
            Search {
                creator_user: Some(1),
                creator_group: Some(42),
                ..valid()
            },
            Search {
                free: true,
                max_price_cents: Some(100),
                ..valid()
            },
            Search {
                page_token: Some("next\nprivate".into()),
                ..valid()
            },
        ];
        for query in invalid_queries {
            let error = query.url().unwrap_err();
            assert_eq!(error.code, "store_search_invalid");
            assert_eq!(error.exit_code, 2);
        }
    }
}
