use clap::{Args, ValueEnum};
use reqwest::Url;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};

use crate::error::AppError;
use crate::traffic::{Clock, TrafficGuard};
use crate::transport::{Request, Transport};

#[derive(Clone, Copy, Debug, Default, ValueEnum)]
pub(crate) enum SearchType {
    #[default]
    Clothing,
    All,
    ClassicShirt,
    ClassicTshirt,
    ClassicPants,
    Shirt,
    Tshirt,
    Pants,
    Jacket,
    Sweater,
    Shorts,
    DressSkirt,
    LeftShoe,
    RightShoe,
    Accessories,
    Hat,
    Hair,
    FaceAccessory,
    NeckAccessory,
    ShoulderAccessory,
    FrontAccessory,
    BackAccessory,
    WaistAccessory,
    #[value(alias = "bundle")]
    Bundles,
}
impl SearchType {
    fn filters(self) -> (&'static [u32], &'static [u32]) {
        match self {
            Self::All => (&[], &[]),
            Self::Clothing => (&[2, 11, 12, 64, 65, 66, 67, 68, 69, 70, 71, 72], &[]),
            Self::ClassicShirt => (&[11], &[]),
            Self::ClassicTshirt => (&[2], &[]),
            Self::ClassicPants => (&[12], &[]),
            Self::Shirt => (&[65], &[]),
            Self::Tshirt => (&[64], &[]),
            Self::Pants => (&[66], &[]),
            Self::Jacket => (&[67], &[]),
            Self::Sweater => (&[68], &[]),
            Self::Shorts => (&[69], &[]),
            Self::DressSkirt => (&[72], &[]),
            Self::LeftShoe => (&[70], &[]),
            Self::RightShoe => (&[71], &[]),
            Self::Accessories => (&[8, 41, 42, 43, 44, 45, 46, 47], &[]),
            Self::Hat => (&[8], &[]),
            Self::Hair => (&[41], &[]),
            Self::FaceAccessory => (&[42], &[]),
            Self::NeckAccessory => (&[43], &[]),
            Self::ShoulderAccessory => (&[44], &[]),
            Self::FrontAccessory => (&[45], &[]),
            Self::BackAccessory => (&[46], &[]),
            Self::WaistAccessory => (&[47], &[]),
            Self::Bundles => (&[], &[1, 2, 3, 4]),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, ValueEnum)]
pub(crate) enum Sort {
    #[default]
    Relevance,
    Favorites,
    Sales,
    Updated,
    PriceAsc,
    PriceDesc,
}
impl Sort {
    fn api(self) -> u8 {
        match self {
            Self::Relevance => 0,
            Self::Favorites => 1,
            Self::Sales => 2,
            Self::Updated => 3,
            Self::PriceAsc => 4,
            Self::PriceDesc => 5,
        }
    }
}
#[derive(Clone, Copy, Debug, Default, ValueEnum)]
pub(crate) enum Period {
    Day,
    Week,
    Month,
    #[default]
    All,
}
impl Period {
    fn api(self) -> u8 {
        match self {
            Self::Day => 1,
            Self::Week => 3,
            Self::Month => 4,
            Self::All => 5,
        }
    }
}

#[derive(Debug, Default, Args)]
pub(crate) struct Search {
    /// Avatar Marketplace item types, not Creator Store models/audio.
    #[arg(long = "type", value_enum, default_value = "clothing")]
    pub kind: SearchType,
    #[arg(long, short = 'q')]
    pub query: String,
    #[arg(long, conflicts_with="creator_group", value_parser=clap::value_parser!(u64).range(1..=9223372036854775807))]
    pub creator_user: Option<u64>,
    #[arg(long, conflicts_with="creator_user", value_parser=clap::value_parser!(u64).range(1..=9223372036854775807))]
    pub creator_group: Option<u64>,
    /// Price in Robux, not Creator Store USD cents.
    #[arg(long, value_parser=clap::value_parser!(u32).range(..=2147483647))]
    pub min_price: Option<u32>,
    #[arg(long, conflicts_with="free", value_parser=clap::value_parser!(u32).range(..=2147483647))]
    pub max_price: Option<u32>,
    #[arg(long, conflicts_with = "max_price")]
    pub free: bool,
    #[arg(long, value_enum, default_value = "relevance")]
    pub sort: Sort,
    #[arg(long, value_enum, default_value = "all")]
    pub period: Period,
    #[arg(long)]
    pub include_offsale: bool,
    /// One page of 10, 28 or 30 results. No automatic pagination/fan-out.
    #[arg(long, default_value="10", value_parser=clap::value_parser!(u32).range(1..=30))]
    pub limit: u32,
    #[arg(long)]
    pub cursor: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ItemKind {
    Asset,
    Bundle,
}
impl ItemKind {
    fn label(self) -> &'static str {
        match self {
            Self::Asset => "Asset",
            Self::Bundle => "Bundle",
        }
    }
}

#[derive(Debug, Args)]
#[group(required = true, multiple = false)]
pub(crate) struct Target {
    #[arg(long, value_parser=clap::value_parser!(u64).range(1..=9223372036854775807))]
    pub asset: Option<u64>,
    #[arg(long, value_parser=clap::value_parser!(u64).range(1..=9223372036854775807))]
    pub bundle: Option<u64>,
}
impl Target {
    pub fn selection(&self) -> Result<(ItemKind, u64), AppError> {
        let target = match (self.asset, self.bundle) {
            (Some(id), None) => (ItemKind::Asset, id),
            (None, Some(id)) => (ItemKind::Bundle, id),
            _ => return Err(invalid()),
        };
        if target.1 == 0 || target.1 > i64::MAX as u64 {
            return Err(invalid());
        }
        Ok(target)
    }
}

fn invalid() -> AppError {
    AppError::new(
        "avatar_query_invalid",
        "The avatar catalog query or typed target is invalid.",
        "Use a short query, a supported type, one creator, an ordered Robux price range, page size 10/28/30, and exactly one asset or bundle ID where required. No credentials were accessed.",
    )
}
fn text_ok(value: &str, max: usize) -> bool {
    !value.trim().is_empty() && value.len() <= max && !value.chars().any(char::is_control)
}
fn bad_reply() -> AppError {
    AppError::network(
        "avatar_response_invalid",
        "The avatar catalog response did not match the expected item, type or schema. No raw response body is printed.",
    )
}

impl Search {
    pub fn url(&self) -> Result<Url, AppError> {
        let max = if self.free { Some(0) } else { self.max_price };
        if !text_ok(&self.query, 512)
            || ![10, 28, 30].contains(&self.limit)
            || self.cursor.as_ref().is_some_and(|v| !text_ok(v, 4096))
            || (self.creator_user.is_some() && self.creator_group.is_some())
            || [self.creator_user, self.creator_group]
                .into_iter()
                .flatten()
                .any(|id| id == 0 || id > i64::MAX as u64)
            || (self.free && self.max_price.is_some())
            || self.min_price.zip(max).is_some_and(|(a, b)| a > b)
            || [self.min_price, max]
                .into_iter()
                .flatten()
                .any(|n| n > i32::MAX as u32)
        {
            return Err(invalid());
        }
        let mut url = Url::parse("https://catalog.roblox.com/v2/search/items/details").unwrap();
        {
            let mut q = url.query_pairs_mut();
            q.append_pair("Keyword", &self.query)
                .append_pair("limit", &self.limit.to_string());
            q.append_pair("SortType", &self.sort.api().to_string())
                .append_pair("SortAggregation", &self.period.api().to_string());
            q.append_pair(
                "IncludeNotForSale",
                if self.include_offsale {
                    "true"
                } else {
                    "false"
                },
            );
            let (assets, bundles) = self.kind.filters();
            for id in assets {
                q.append_pair("AssetTypeIds", &id.to_string());
            }
            for id in bundles {
                q.append_pair("BundleTypeIds", &id.to_string());
            }
            if let Some(id) = self.creator_user {
                q.append_pair("CreatorType", "1")
                    .append_pair("CreatorTargetId", &id.to_string());
            }
            if let Some(id) = self.creator_group {
                q.append_pair("CreatorType", "2")
                    .append_pair("CreatorTargetId", &id.to_string());
            }
            if let Some(value) = self.min_price {
                q.append_pair("MinPrice", &value.to_string());
            }
            if let Some(value) = max {
                q.append_pair("MaxPrice", &value.to_string());
            }
            if let Some(value) = &self.cursor {
                q.append_pair("cursor", value);
            }
        }
        Ok(url)
    }
}

#[derive(Deserialize, Serialize)]
#[serde(untagged)]
enum ApiEnum {
    Number(u32),
    Name(String),
}
impl ApiEnum {
    fn item_kind(&self) -> Result<ItemKind, AppError> {
        match self {
            Self::Number(1) => Ok(ItemKind::Asset),
            Self::Number(2) => Ok(ItemKind::Bundle),
            Self::Name(s) if s == "Asset" => Ok(ItemKind::Asset),
            Self::Name(s) if s == "Bundle" => Ok(ItemKind::Bundle),
            _ => Err(bad_reply()),
        }
    }
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Reply {
    data: Vec<Item>,
    next_page_cursor: Option<String>,
    previous_page_cursor: Option<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Item {
    id: u64,
    item_type: ApiEnum,
    name: String,
    description: Option<String>,
    asset_type: Option<u32>,
    bundle_type: Option<ApiEnum>,
    creator_type: Option<ApiEnum>,
    creator_target_id: Option<u64>,
    creator_name: Option<String>,
    creator_has_verified_badge: Option<bool>,
    price: Option<i64>,
    lowest_price: Option<i64>,
    price_status: Option<String>,
    is_off_sale: Option<bool>,
    favorite_count: Option<u64>,
    item_status: Option<Vec<ApiEnum>>,
    item_restrictions: Option<Vec<ApiEnum>>,
    bundled_items: Option<Vec<BundleItem>>,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct BundleItem {
    id: u64,
    name: Option<String>,
    #[serde(rename = "type")]
    kind: Option<ApiEnum>,
    asset_type: Option<u32>,
}

pub(crate) fn links(kind: ItemKind, id: u64) -> Result<Value, AppError> {
    if id == 0 || id > i64::MAX as u64 {
        return Err(invalid());
    }
    let path = match kind {
        ItemKind::Asset => "catalog",
        ItemKind::Bundle => "bundles",
    };
    Ok(
        json!({"id":id,"itemType":kind.label(),"catalogUrl":format!("https://www.roblox.com/{path}/{id}"),
        "previewPageUrl":format!("https://www.roblox.com/{path}/{id}"), "existsChecked":false,
        "ownershipVerified":false,"previewEligibilityVerified":false,"appearanceApplied":false,"accountAvatarChanged":false,
        "appearanceHint":appearance_hint(kind,None,id),"browserOpened":false}),
    )
}

fn appearance_hint(kind: ItemKind, asset_type: Option<u32>, id: u64) -> Value {
    if kind == ItemKind::Bundle {
        return json!({"mechanism":"resolve-bundle-items-or-outfit-first","bundleId":id,"canUseAsSingleAssetId":false,"validatedInStudio":false});
    }
    let property = match asset_type {
        Some(2) => Some("GraphicTShirt"),
        Some(11) => Some("Shirt"),
        Some(12) => Some("Pants"),
        _ => None,
    };
    if let Some(property) = property {
        return json!({"mechanism":"HumanoidDescription property","property":property,"assetId":id,
        "requiresCatalogClothingAssetId":true,"notADecalOrImageTextureId":true,"validatedInStudio":false});
    }
    let accessory = match asset_type {
        Some(8) => Some(("Hat", false)),
        Some(41) => Some(("Hair", false)),
        Some(42) => Some(("Face", false)),
        Some(43) => Some(("Neck", false)),
        Some(44) => Some(("Shoulder", false)),
        Some(45) => Some(("Front", false)),
        Some(46) => Some(("Back", false)),
        Some(47) => Some(("Waist", false)),
        Some(64) => Some(("TShirt", true)),
        Some(65) => Some(("Shirt", true)),
        Some(66) => Some(("Pants", true)),
        Some(67) => Some(("Jacket", true)),
        Some(68) => Some(("Sweater", true)),
        Some(69) => Some(("Shorts", true)),
        Some(70) => Some(("LeftShoe", true)),
        Some(71) => Some(("RightShoe", true)),
        Some(72) => Some(("DressSkirt", true)),
        _ => None,
    };
    match accessory {
        Some((accessory_type, layered)) => {
            json!({"mechanism":"HumanoidDescription:SetAccessories","assetId":id,"accessoryType":accessory_type,
        "isLayered":layered,"mergeExistingAccessories":true,"orderRequired":layered,"validatedInStudio":false})
        }
        None => {
            json!({"mechanism":"inspect-asset-type-first","assetId":id,"validatedInStudio":false})
        }
    }
}

impl Item {
    fn project(self) -> Result<Value, AppError> {
        if self.id == 0 || self.id > i64::MAX as u64 || self.name.trim().is_empty() {
            return Err(bad_reply());
        }
        let kind = self.item_type.item_kind()?;
        let mut card = links(kind, self.id)?;
        card["existsChecked"] = true.into();
        card["name"] = self.name.into();
        card["description"] = self.description.into();
        card["assetTypeId"] = self.asset_type.into();
        card["bundleType"] = serde_json::to_value(self.bundle_type).map_err(|_| bad_reply())?;
        card["creator"] = json!({"type":self.creator_type,"id":self.creator_target_id,"name":self.creator_name,"hasVerifiedBadge":self.creator_has_verified_badge});
        card["priceInRobux"] = self.price.into();
        card["lowestPriceInRobux"] = self.lowest_price.into();
        card["priceStatus"] = self.price_status.into();
        card["isOffSale"] = self.is_off_sale.into();
        card["favoriteCount"] = self.favorite_count.into();
        card["itemStatus"] = serde_json::to_value(self.item_status).map_err(|_| bad_reply())?;
        card["itemRestrictions"] =
            serde_json::to_value(self.item_restrictions).map_err(|_| bad_reply())?;
        card["bundledItems"] = serde_json::to_value(self.bundled_items).map_err(|_| bad_reply())?;
        card["appearanceHint"] = appearance_hint(kind, self.asset_type, self.id);
        Ok(card)
    }
}

pub(crate) struct Avatar<T: Transport> {
    pub transport: T,
}
impl<T: Transport> Avatar<T> {
    pub fn new(transport: T) -> Self {
        Self { transport }
    }
    fn read<R: DeserializeOwned, C: Clock>(
        &mut self,
        request: Request,
        guard: &mut TrafficGuard<C>,
    ) -> Result<R, AppError> {
        guard.reserve()?;
        let response = self.transport.send(request)?;
        if response.challenge || !(200..300).contains(&response.status) {
            let code = if response.challenge {
                "avatar_verification_required"
            } else {
                match response.status {
                    300..=399 => "redirect_refused",
                    401 | 403 => "avatar_access_refused",
                    404 => "avatar_not_found",
                    429 => "rate_limited",
                    500..=599 => "server_error",
                    _ => "avatar_request_failed",
                }
            };
            return Err(AppError::http(
                code,
                "The anonymous avatar catalog request was refused; no retry or credential fallback was attempted.",
                "Stop. No Cookie, CSRF-token handshake, account change or alternate endpoint is used to bypass a refusal.",
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
        let page: Reply = self.read(Request::avatar_search(query.url()?)?, guard)?;
        let next = page.next_page_cursor.filter(|s| !s.is_empty());
        let previous = page.previous_page_cursor.filter(|s| !s.is_empty());
        if page.data.len() > query.limit as usize
            || [&next, &previous]
                .into_iter()
                .flatten()
                .any(|s| !text_ok(s, 4096))
        {
            return Err(bad_reply());
        }
        let (asset_types, bundle_types) = query.kind.filters();
        for item in &page.data {
            if !asset_types.is_empty()
                && (item.item_type.item_kind()? != ItemKind::Asset
                    || !item.asset_type.is_some_and(|t| asset_types.contains(&t)))
            {
                return Err(bad_reply());
            }
            if !bundle_types.is_empty() && item.item_type.item_kind()? != ItemKind::Bundle {
                return Err(bad_reply());
            }
        }
        let cards: Vec<Value> = page
            .data
            .into_iter()
            .map(Item::project)
            .collect::<Result<_, _>>()?;
        Ok(
            json!({"query":query.query,"count":cards.len(),"items":cards,"hasMore":next.is_some(),"nextPageCursor":next,
            "previousPageCursor":previous,"automaticPagination":false,"detailFanout":false,"currency":"Robux"}),
        )
    }

    pub fn bundle<C: Clock>(
        &mut self,
        id: u64,
        guard: &mut TrafficGuard<C>,
    ) -> Result<Value, AppError> {
        if id == 0 || id > i64::MAX as u64 {
            return Err(invalid());
        }
        let url = Url::parse(&format!(
            "https://catalog.roblox.com/v1/bundles/{id}/details"
        ))
        .unwrap();
        let body: Value = self.read(Request::avatar_bundle(url)?, guard)?;
        if body.get("id").and_then(Value::as_u64) != Some(id) {
            return Err(bad_reply());
        }
        let name = body
            .get("name")
            .and_then(Value::as_str)
            .filter(|value| text_ok(value, 4096))
            .ok_or_else(bad_reply)?;
        let items = body
            .get("items")
            .and_then(Value::as_array)
            .filter(|items| items.len() <= 100)
            .ok_or_else(bad_reply)?;
        let mut cards = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for item in items {
            let item_id = item
                .get("id")
                .and_then(Value::as_u64)
                .filter(|value| *value > 0 && *value <= i64::MAX as u64)
                .ok_or_else(bad_reply)?;
            let reported = item.get("type").and_then(Value::as_str).unwrap_or("");
            let kind = match reported {
                "Asset" => "Asset",
                "Bundle" => "Bundle",
                "UserOutfit" => "UserOutfit",
                _ => "unsupported",
            };
            if kind != "unsupported" && !seen.insert((kind, item_id)) {
                return Err(bad_reply());
            }
            let reported_type = if kind == "unsupported"
                && reported.len() <= 64
                && reported.chars().all(|c| c.is_ascii_alphanumeric())
            {
                Some(reported)
            } else {
                None
            };
            cards.push(json!({
                "id": item_id,
                "kind": kind,
                "reportedType": reported_type,
                "name": item.get("name").and_then(Value::as_str).filter(|value| text_ok(value, 4096)),
                "usableAsHumanoidDescriptionAssetId": false,
                "outfitExpanded": false
            }));
        }
        Ok(json!({
            "bundleId": id,
            "name": name,
            "bundleType": body.get("bundleType").and_then(Value::as_str).filter(|value| text_ok(value, 64)),
            "items": cards,
            "ownershipVerified": false,
            "purchasePerformed": false,
            "appearanceApplied": false,
            "rigCompatibilityVerified": false,
            "studioApplicationVerified": false,
            "detailFanout": false,
            "humanoidDescriptionAssetIds": false
        }))
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
                csrf: if self.status == 403 {
                    let mut value = reqwest::header::HeaderValue::from_static("synthetic-csrf");
                    value.set_sensitive(true);
                    Some(value)
                } else {
                    None
                },
                challenge: false,
            })
        }
    }

    #[test]
    fn layered_clothing_search_uses_catalog_types_and_reports_appearance_hint_without_ownership() {
        let root = tempfile::tempdir().unwrap();
        let mut guard =
            crate::traffic::TrafficGuard::acquire(root.path(), FakeClock::default()).unwrap();
        let query = Search {
            kind: SearchType::Jacket,
            query: "black & Cookie=not-a-parameter".into(),
            limit: 10,
            ..Search::default()
        };
        let mut client = Avatar::new(FakeTransport {
            status: 200,
            records: Vec::new(),
            body: json!({
                "data": [{ "id": 77, "itemType": "Asset", "assetType": 67, "name": "Synthetic Jacket", "price": 90,
                    "apikeySecret": "must-not-echo" }], "nextPageCursor": "next"
            }),
        });
        let result = client.search(&query, &mut guard).unwrap();
        assert_eq!(client.transport.records.len(), 1);
        assert_eq!(client.transport.records[0]["cookieAttached"], false);
        let url =
            reqwest::Url::parse(client.transport.records[0]["url"].as_str().unwrap()).unwrap();
        assert_eq!(url.host_str(), Some("catalog.roblox.com"));
        assert_eq!(url.path(), "/v2/search/items/details");
        assert!(
            url.query_pairs()
                .any(|(k, v)| k == "AssetTypeIds" && v == "67")
        );
        assert!(
            url.query_pairs()
                .any(|(k, v)| k == "Keyword" && v == "black & Cookie=not-a-parameter")
        );
        assert!(!url.query_pairs().any(|(k, _)| k == "Cookie"));
        let item = &result["items"][0];
        assert_eq!(item["catalogUrl"], "https://www.roblox.com/catalog/77");
        assert_eq!(item["priceInRobux"], 90);
        assert_eq!(item["ownershipVerified"], false);
        assert_eq!(item["appearanceHint"]["accessoryType"], "Jacket");
        assert_eq!(item["appearanceHint"]["isLayered"], true);
        assert_eq!(item["accountAvatarChanged"], false);
        assert_eq!(result["hasMore"], true);
        assert!(!result.to_string().contains("must-not-echo"));
    }

    #[test]
    fn classic_clothing_metadata_uses_clothing_asset_ids_not_template_image_ids() {
        for (asset_type, property) in [(2, "GraphicTShirt"), (11, "Shirt"), (12, "Pants")] {
            let root = tempfile::tempdir().unwrap();
            let mut guard =
                crate::traffic::TrafficGuard::acquire(root.path(), FakeClock::default()).unwrap();
            let mut client = Avatar::new(FakeTransport {
                status: 200,
                records: Vec::new(),
                body: json!({"data":[{
                    "id":77,"itemType":1,"name":"Classic clothing","assetType":asset_type
                }]}),
            });
            let result = client
                .search(
                    &Search {
                        kind: SearchType::All,
                        query: "clothing".into(),
                        limit: 10,
                        ..Search::default()
                    },
                    &mut guard,
                )
                .unwrap();
            let item = &result["items"][0];
            assert_eq!(item["appearanceHint"]["property"], property);
            assert_eq!(item["appearanceHint"]["notADecalOrImageTextureId"], true);
            assert_eq!(item["priceInRobux"], Value::Null);
            assert_eq!(item["ownershipVerified"], false);
            assert_eq!(client.transport.records.len(), 1);
            assert_eq!(client.transport.records[0]["method"], "GET");
            assert_eq!(client.transport.records[0]["body"], Value::Null);
            assert_eq!(client.transport.records[0]["cookieAttached"], false);
            assert_eq!(client.transport.records[0]["csrfAttached"], false);
        }
    }

    #[test]
    fn bundle_metadata_preserves_child_types_and_never_treats_bundle_id_as_an_asset() {
        let root = tempfile::tempdir().unwrap();
        let mut guard =
            crate::traffic::TrafficGuard::acquire(root.path(), FakeClock::default()).unwrap();
        let mut client = Avatar::new(FakeTransport {
            status: 200,
            records: Vec::new(),
            body: json!({"data":[{
                "id":77,"itemType":"Bundle","name":"Synthetic Bundle","bundleType":"Shoes",
                "bundledItems":[{"id":88,"type":"Asset","assetType":70,"owned":true},{"id":99,"type":"UserOutfit"}]
            }]}),
        });
        let result = client
            .search(
                &Search {
                    kind: SearchType::Bundles,
                    query: "shoes".into(),
                    limit: 10,
                    ..Search::default()
                },
                &mut guard,
            )
            .unwrap();
        let item = &result["items"][0];
        assert_eq!(item["catalogUrl"], "https://www.roblox.com/bundles/77");
        assert_eq!(item["appearanceHint"]["canUseAsSingleAssetId"], false);
        assert_eq!(item["appearanceHint"]["validatedInStudio"], false);
        assert_eq!(item["bundledItems"][1]["type"], "UserOutfit");
        assert!(item["bundledItems"][0].get("owned").is_none());
        assert_eq!(client.transport.records[0]["method"], "GET");
    }

    #[test]
    fn bundle_details_keep_typed_ids_and_reject_a_mismatched_bundle() {
        let root = tempfile::tempdir().unwrap();
        let mut guard =
            crate::traffic::TrafficGuard::acquire(root.path(), FakeClock::default()).unwrap();
        let mut client = Avatar::new(FakeTransport {
            status: 200,
            records: Vec::new(),
            body: json!({
                "id": 77,
                "name": "Synthetic Bundle",
                "bundleType": "BodyParts",
                "items": [
                    {"id": 11, "type": "Asset", "name": "Head"},
                    {"id": 22, "type": "UserOutfit", "name": "Look"},
                    {"id": 33, "type": "Mystery", "name": "Unknown"}
                ]
            }),
        });
        let result = client.bundle(77, &mut guard).unwrap();
        assert_eq!(result["items"][0]["kind"], "Asset");
        assert_eq!(result["items"][1]["kind"], "UserOutfit");
        assert_eq!(result["items"][2]["kind"], "unsupported");
        assert_eq!(
            result["items"][0]["usableAsHumanoidDescriptionAssetId"],
            false
        );
        assert_eq!(result["humanoidDescriptionAssetIds"], false);
        assert_eq!(client.transport.records[0]["cookieAttached"], false);
        assert!(
            client.transport.records[0]["url"]
                .as_str()
                .unwrap()
                .ends_with("/v1/bundles/77/details")
        );
        client.transport.body = json!({"id": 88, "name": "Other", "items": []});
        assert_eq!(
            client.bundle(77, &mut guard).unwrap_err().code,
            "avatar_response_invalid"
        );
    }

    #[test]
    fn malformed_ids_or_unknown_item_namespaces_never_fabricate_an_appearance() {
        for (id, kind) in [(0, "Asset"), (77, "UserOutfit")] {
            let root = tempfile::tempdir().unwrap();
            let mut guard =
                crate::traffic::TrafficGuard::acquire(root.path(), FakeClock::default()).unwrap();
            let mut client = Avatar::new(FakeTransport {
                status: 200,
                records: Vec::new(),
                body: json!({"data":[{
                    "id":id,"itemType":kind,"name":"Other item"
                }]}),
            });
            assert_eq!(
                client
                    .search(
                        &Search {
                            kind: SearchType::All,
                            query: "synthetic".into(),
                            limit: 10,
                            ..Search::default()
                        },
                        &mut guard
                    )
                    .unwrap_err()
                    .code,
                "avatar_response_invalid"
            );
            assert_eq!(client.transport.records.len(), 1);
        }
    }

    #[test]
    fn anonymous_catalog_search_never_replays_csrf_errors_or_echoes_server_bodies() {
        for status in [302, 401, 403, 404, 429, 500, 503] {
            let root = tempfile::tempdir().unwrap();
            let mut guard =
                crate::traffic::TrafficGuard::acquire(root.path(), FakeClock::default()).unwrap();
            let mut client = Avatar::new(FakeTransport {
                status,
                records: Vec::new(),
                body: json!({"error":"must-not-echo"}),
            });
            let error = client
                .search(
                    &Search {
                        query: "synthetic".into(),
                        limit: 10,
                        ..Search::default()
                    },
                    &mut guard,
                )
                .unwrap_err();
            assert_eq!(error.exit_code, 3);
            assert!(
                !serde_json::to_string(&error)
                    .unwrap()
                    .contains("must-not-echo")
            );
            assert_eq!(client.transport.records.len(), 1);
            assert_eq!(client.transport.records[0]["csrfAttached"], false);
            assert_eq!(client.transport.records[0]["cookieAttached"], false);
        }
    }

    #[test]
    fn clothing_arrays_use_multi_query_values_and_robux_filters_not_store_categories() {
        let query = Search {
            query: "red".into(),
            limit: 10,
            creator_group: Some(42),
            min_price: Some(5),
            max_price: Some(20),
            ..Search::default()
        };
        let url = query.url().unwrap();
        let ids: Vec<_> = url
            .query_pairs()
            .filter(|(k, _)| k == "AssetTypeIds")
            .map(|(_, v)| v.into_owned())
            .collect();
        assert!(ids.contains(&"11".to_owned()));
        assert!(ids.contains(&"65".to_owned()));
        assert_eq!(ids.len(), 12);
        assert!(
            url.query_pairs()
                .any(|(k, v)| k == "CreatorType" && v == "2")
        );
        assert!(url.query_pairs().any(|(k, v)| k == "MaxPrice" && v == "20"));
        assert!(!url.query_pairs().any(|(k, _)| k == "searchCategoryType"));
        assert!(Search { limit: 11, ..query }.url().is_err());
        assert!(
            Target {
                asset: Some(77),
                bundle: Some(88)
            }
            .selection()
            .is_err()
        );
    }

    #[test]
    fn a_search_response_cannot_silently_substitute_other_asset_types() {
        let root = tempfile::tempdir().unwrap();
        let mut guard =
            crate::traffic::TrafficGuard::acquire(root.path(), FakeClock::default()).unwrap();
        let mut client = Avatar::new(FakeTransport {
            status: 200,
            records: Vec::new(),
            body: json!({"data":[{
                "id":77,"itemType":"Asset","name":"Wrong type","assetType":11
            }]}),
        });
        let query = Search {
            kind: SearchType::Jacket,
            query: "black".into(),
            limit: 10,
            ..Search::default()
        };
        assert_eq!(
            client.search(&query, &mut guard).unwrap_err().code,
            "avatar_response_invalid"
        );
        assert_eq!(client.transport.records.len(), 1);
    }
}
