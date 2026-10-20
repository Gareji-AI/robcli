//! Public charts, experience snapshots and published music/commerce metadata.
//! These reads never resolve Firefox or expand into private configuration/assets.
use clap::{Args, ValueEnum};
use reqwest::Url;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::error::AppError;
use crate::traffic::{Clock, TrafficGuard};
use crate::transport::{Request, Transport};

fn invalid() -> AppError {
    AppError::new(
        "discovery_query_invalid",
        "The public discovery query is invalid.",
        "Use exactly one positive Place/Universe ID, a bounded page, a supported chart/device/country, and a short opaque page token. No credentials were accessed.",
    )
}
fn bad_reply() -> AppError {
    AppError::network(
        "discovery_response_invalid",
        "The public response did not match the expected schema or target; no raw response body is printed.",
    )
}
fn valid_id(id: u64) -> bool {
    id > 0 && id <= i64::MAX as u64
}
fn text_ok(s: &str, max: usize) -> bool {
    !s.trim().is_empty() && s.len() <= max && !s.chars().any(char::is_control)
}
fn token_ok(token: &Option<String>) -> bool {
    token.as_ref().is_none_or(|s| text_ok(s, 4096))
}

#[derive(Debug, Args)]
#[group(required = true, multiple = false)]
pub(crate) struct Target {
    /// Experience Universe ID, not a Place ID.
    #[arg(long, value_parser=clap::value_parser!(u64).range(1..=9223372036854775807))]
    pub universe: Option<u64>,
    /// Resolve this Place to its Universe with one extra public request.
    #[arg(long, value_parser=clap::value_parser!(u64).range(1..=9223372036854775807))]
    pub place: Option<u64>,
}
impl Target {
    pub fn validate(&self) -> Result<(), AppError> {
        match (self.universe, self.place) {
            (Some(id), None) | (None, Some(id)) if valid_id(id) => Ok(()),
            _ => Err(invalid()),
        }
    }
}

#[derive(Debug, Args)]
pub(crate) struct Page {
    #[arg(long, default_value="10", value_parser=clap::value_parser!(u32).range(1..=30))]
    pub limit: u32,
    /// Explicit next page only. No automatic pagination or per-item fan-out.
    #[arg(long)]
    pub page_token: Option<String>,
}
impl Page {
    pub fn validate(&self) -> Result<(), AppError> {
        if self.limit == 0 || self.limit > 30 || !token_ok(&self.page_token) {
            return Err(invalid());
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default, ValueEnum)]
pub(crate) enum Device {
    #[default]
    Computer,
    Phone,
    Tablet,
}
impl Device {
    fn api(self) -> &'static str {
        match self {
            Self::Computer => "computer",
            Self::Phone => "phone",
            Self::Tablet => "tablet",
        }
    }
}
#[derive(Debug, Args)]
pub(crate) struct Chart {
    #[arg(long, value_enum, default_value = "computer")]
    pub device: Device,
    /// all, or a two-letter uppercase country code such as JP/US.
    #[arg(long, default_value = "all")]
    pub country: String,
    #[arg(long)]
    pub page_token: Option<String>,
}
impl Chart {
    pub fn validate(&self) -> Result<(), AppError> {
        if !(self.country == "all"
            || (self.country.len() == 2 && self.country.bytes().all(|b| b.is_ascii_uppercase())))
            || !token_ok(&self.page_token)
        {
            return Err(invalid());
        }
        Ok(())
    }
    pub fn url(&self, sort: Option<&str>) -> Result<Url, AppError> {
        self.validate()?;
        if sort.is_some_and(|s| {
            !text_ok(s, 128)
                || !s
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        }) {
            return Err(invalid());
        }
        let path = if sort.is_some() {
            "get-sort-content"
        } else {
            "get-sorts"
        };
        let mut url = api_url(&format!("/explore-api/v1/{path}"));
        // Correlation ID only, never an account/session credential or supplied token.
        let session = correlation_id()?;
        let mut q = url.query_pairs_mut();
        q.append_pair("sessionId", &session)
            .append_pair("device", self.device.api())
            .append_pair("country", &self.country);
        if let Some(s) = sort {
            q.append_pair("sortId", s);
        }
        if let Some(t) = &self.page_token {
            q.append_pair(
                if sort.is_some() {
                    "pageToken"
                } else {
                    "sortsPageToken"
                },
                t,
            );
        }
        drop(q);
        Ok(url)
    }
}
fn correlation_id() -> Result<String, AppError> {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let mut bytes = (SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| invalid())?
        .as_nanos()
        ^ ((SEQ.fetch_add(1, Ordering::Relaxed) as u128) << 64))
        .to_be_bytes();
    bytes[6] = (bytes[6] & 15) | 64;
    bytes[8] = (bytes[8] & 63) | 128;
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    Ok(format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    ))
}
fn api_url(path: &str) -> Url {
    Url::parse(&format!("https://apis.roblox.com{path}")).unwrap()
}
fn games_url(path: &str, id: u64) -> Url {
    let mut u = Url::parse(&format!("https://games.roblox.com{path}")).unwrap();
    u.query_pairs_mut()
        .append_pair("universeIds", &id.to_string());
    u
}
fn game_link(place: u64) -> Value {
    if valid_id(place) {
        json!(format!("https://www.roblox.com/games/{place}"))
    } else {
        Value::Null
    }
}
fn opt_string(v: &Value, key: &str) -> Result<Option<String>, AppError> {
    match v.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        _ => Err(bad_reply()),
    }
}
fn next(v: &Value, key: &str) -> Result<Option<String>, AppError> {
    let t = opt_string(v, key)?.filter(|s| !s.is_empty());
    if !token_ok(&t) {
        return Err(bad_reply());
    }
    Ok(t)
}
fn array<'a>(v: &'a Value, key: &str) -> Result<&'a Vec<Value>, AppError> {
    v.get(key)
        .and_then(Value::as_array)
        .filter(|a| a.len() <= 500)
        .ok_or_else(bad_reply)
}
fn forbidden_control(c: char) -> bool {
    c.is_control() && c != '\n' && c != '\r' && c != '\t'
}
fn opt_bool(v: &Value, key: &str) -> Result<Option<bool>, AppError> {
    match v.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(value)) => Ok(Some(*value)),
        _ => Err(bad_reply()),
    }
}
fn opt_u64(v: &Value, key: &str) -> Result<Option<u64>, AppError> {
    match v.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value.as_u64().map(Some).ok_or_else(bad_reply),
    }
}
fn opt_f64(v: &Value, key: &str) -> Result<Option<f64>, AppError> {
    match v.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value.as_f64().map(Some).ok_or_else(bad_reply),
    }
}
fn opt_id(v: &Value, key: &str) -> Result<Option<u64>, AppError> {
    match v.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_u64()
            .filter(|id| valid_id(*id))
            .map(Some)
            .ok_or_else(bad_reply),
    }
}
const ICON_SIZES: &[&str] = &["50x50", "128x128", "150x150", "256x256", "512x512"];
const THUMB_SIZES: &[&str] = &["768x432", "576x324", "480x270", "384x216", "256x144"];

pub(crate) fn icon_url(universe: u64, size: &str, format: &str) -> Result<Url, AppError> {
    media_url(
        "https://thumbnails.roblox.com/v1/games/icons",
        universe,
        size,
        format,
        ICON_SIZES,
        None,
    )
}
pub(crate) fn thumbnail_url(
    universe: u64,
    size: &str,
    format: &str,
    count: u32,
) -> Result<Url, AppError> {
    if !(1..=10).contains(&count) {
        return Err(invalid());
    }
    media_url(
        "https://thumbnails.roblox.com/v1/games/multiget/thumbnails",
        universe,
        size,
        format,
        THUMB_SIZES,
        Some(count),
    )
}
fn media_url(
    base: &str,
    universe: u64,
    size: &str,
    format: &str,
    sizes: &[&str],
    count: Option<u32>,
) -> Result<Url, AppError> {
    if !valid_id(universe) || !sizes.contains(&size) || !matches!(format, "Png" | "Jpeg") {
        return Err(invalid());
    }
    let mut url = Url::parse(base).unwrap();
    {
        let mut query = url.query_pairs_mut();
        query
            .append_pair("universeIds", &universe.to_string())
            .append_pair("size", size)
            .append_pair("format", format)
            .append_pair("isCircular", "false");
        if let Some(count) = count {
            query.append_pair("count", &count.to_string());
        }
    }
    Ok(url)
}
pub(crate) fn badges_url(
    universe: u64,
    limit: &str,
    sort: &str,
    cursor: Option<&str>,
) -> Result<Url, AppError> {
    if !valid_id(universe)
        || !matches!(limit, "10" | "25" | "50" | "100")
        || !matches!(sort, "Asc" | "Desc")
        || cursor.is_some_and(|value| !text_ok(value, 4096))
    {
        return Err(invalid());
    }
    let mut url = Url::parse(&format!(
        "https://badges.roblox.com/v1/universes/{universe}/badges"
    ))
    .unwrap();
    {
        let mut query = url.query_pairs_mut();
        query
            .append_pair("limit", limit)
            .append_pair("sortOrder", sort);
        if let Some(cursor) = cursor {
            query.append_pair("cursor", cursor);
        }
    }
    Ok(url)
}
fn one_thumbnail(value: &Value, universe: u64) -> Result<&Value, AppError> {
    let rows = array(value, "data")?;
    if rows.len() != 1 || rows[0].get("targetId").and_then(Value::as_u64) != Some(universe) {
        return Err(bad_reply());
    }
    Ok(&rows[0])
}
fn thumbnail_card(universe: u64, size: &str, format: &str, row: &Value) -> Result<Value, AppError> {
    if row.get("targetId").and_then(Value::as_u64) != Some(universe) {
        return Err(bad_reply());
    }
    let state = opt_string(row, "state")?
        .filter(|value| text_ok(value, 64))
        .ok_or_else(bad_reply)?;
    let ready = match state.as_str() {
        "Completed" => true,
        "Pending" | "Blocked" | "Error" | "InReview" | "TemporarilyUnavailable" => false,
        _ => return Err(bad_reply()),
    };
    let image = if ready {
        Some(public_image_url(row)?.ok_or_else(bad_reply)?)
    } else {
        None
    };
    Ok(json!({
        "targetId": universe, "state": state, "imageReady": ready, "imageUrl": image,
        "size": size, "format": format, "imageBytesFetched": false, "rightsVerified": false, "ctrEstimated": false
    }))
}
fn public_image_url(row: &Value) -> Result<Option<String>, AppError> {
    match row.get("imageUrl") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(raw)) => {
            let url = Url::parse(raw).map_err(|_| bad_reply())?;
            let host = url.host_str().unwrap_or("");
            if url.scheme() != "https"
                || !url.username().is_empty()
                || url.password().is_some()
                || url.port().is_some()
                || url.fragment().is_some()
                || !(host == "rbxcdn.com" || host.ends_with(".rbxcdn.com"))
            {
                return Err(bad_reply());
            }
            Ok(Some(url.to_string()))
        }
        _ => Err(bad_reply()),
    }
}
fn positive(v: &Value, key: &str) -> Result<u64, AppError> {
    v.get(key)
        .and_then(Value::as_u64)
        .filter(|n| valid_id(*n))
        .ok_or_else(bad_reply)
}
fn field(v: &Value, key: &str) -> Value {
    v.get(key).cloned().unwrap_or(Value::Null)
}
fn select(v: &Value, keys: &[&str]) -> Value {
    let mut card = json!({});
    for k in keys {
        card[*k] = field(v, k);
    }
    card
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Game {
    id: u64,
    root_place_id: u64,
    name: String,
    description: Option<String>,
    playing: Option<u64>,
    visits: Option<u64>,
    favorited_count: Option<u64>,
    max_players: Option<u32>,
    created: Option<String>,
    updated: Option<String>,
    genre: Option<String>,
    #[serde(rename = "genre_l1")]
    genre_l1: Option<String>,
    #[serde(rename = "genre_l2")]
    genre_l2: Option<String>,
    price: Option<u64>,
    is_content_restricted: Option<bool>,
    creator: Option<Creator>,
}
#[derive(Deserialize, serde::Serialize)]
struct Creator {
    id: u64,
    name: Option<String>,
    #[serde(rename = "type")]
    kind: Option<String>,
    #[serde(rename = "hasVerifiedBadge")]
    has_verified_badge: Option<bool>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Votes {
    id: u64,
    up_votes: Option<u64>,
    down_votes: Option<u64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Pass {
    id: u64,
    name: String,
    price: Option<u64>,
    user_base_price_in_robux: Option<u64>,
    is_for_sale: Option<bool>,
    display_name: Option<String>,
    display_description: Option<String>,
    display_icon_image_asset_id: Option<u64>,
    created: Option<String>,
    updated: Option<String>,
    creator: Option<PassCreator>,
}
#[derive(Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct PassCreator {
    creator_id: u64,
    creator_type: Option<String>,
    name: Option<String>,
}
#[derive(Deserialize)]
struct Product {
    #[serde(rename = "ProductId", alias = "productId")]
    id: u64,
    #[serde(rename = "UniverseId", alias = "universeId")]
    universe: Option<u64>,
    #[serde(rename = "Name", alias = "name")]
    name: String,
    #[serde(rename = "Description", alias = "description")]
    description: Option<String>,
    #[serde(rename = "PriceInRobux", alias = "priceInRobux")]
    price: Option<u64>,
    #[serde(rename = "UserBasePriceInRobux", alias = "userBasePriceInRobux")]
    base_price: Option<u64>,
    #[serde(rename = "IsForSale", alias = "isForSale")]
    for_sale: Option<bool>,
    #[serde(rename = "IconImageAssetId", alias = "iconImageAssetId")]
    icon: Option<u64>,
    #[serde(rename = "Created", alias = "created")]
    created: Option<String>,
    #[serde(rename = "Updated", alias = "updated")]
    updated: Option<String>,
    // Lowercase fields coexist with PascalCase display fields in public results.
    #[serde(rename = "displayName")]
    display_name: Option<String>,
    #[serde(rename = "displayDescription")]
    display_description: Option<String>,
}

fn like_percent(
    up: Option<u64>,
    down: Option<u64>,
) -> Result<(Option<u64>, Option<f64>), AppError> {
    let total = up.zip(down).and_then(|(a, b)| a.checked_add(b));
    if up.is_some() && down.is_some() && total.is_none() {
        return Err(bad_reply());
    }
    let percent = total
        .filter(|n| *n > 0)
        .map(|n| (10000.0 * up.unwrap_or(0) as f64 / n as f64).round() / 100.0);
    Ok((total, percent))
}
fn count(v: &Value, key: &str) -> Result<Option<u64>, AppError> {
    match v.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(n) => n.as_u64().map(Some).ok_or_else(bad_reply),
    }
}

pub(crate) struct Discovery<T: Transport> {
    pub transport: T,
}
impl<T: Transport> Discovery<T> {
    pub fn new(transport: T) -> Self {
        Self { transport }
    }
    fn read<C: Clock>(&mut self, url: Url, guard: &mut TrafficGuard<C>) -> Result<Value, AppError> {
        self.send(Request::discovery(url)?, guard)
    }
    fn send<C: Clock>(
        &mut self,
        request: Request,
        guard: &mut TrafficGuard<C>,
    ) -> Result<Value, AppError> {
        guard.reserve()?;
        let r = self.transport.send(request)?;
        if r.challenge || !(200..300).contains(&r.status) {
            let code = if r.challenge {
                "discovery_verification_required"
            } else {
                match r.status {
                    300..=399 => "redirect_refused",
                    401 | 403 => "discovery_access_refused",
                    404 => "discovery_not_found",
                    429 => "rate_limited",
                    500..=599 => "server_error",
                    _ => "discovery_request_failed",
                }
            };
            return Err(AppError::http(
                code,
                "The anonymous public read was refused; no retry or credential fallback was attempted.",
                "Stop. Do not switch accounts, attach Cookie/API-key/CSRF, or try alternate endpoints to bypass this refusal.",
                r.status,
                r.retry_after_seconds,
            ));
        }
        serde_json::from_slice(&r.body).map_err(|_| bad_reply())
    }
    fn resolve<C: Clock>(
        &mut self,
        t: &Target,
        guard: &mut TrafficGuard<C>,
    ) -> Result<u64, AppError> {
        t.validate()?;
        if let Some(id) = t.universe {
            return Ok(id);
        }
        let p = t.place.ok_or_else(invalid)?;
        let v = self.read(
            api_url(&format!("/universes/v1/places/{p}/universe")),
            guard,
        )?;
        positive(&v, "universeId")
    }
    pub fn game<C: Clock>(
        &mut self,
        t: &Target,
        guard: &mut TrafficGuard<C>,
    ) -> Result<Value, AppError> {
        let id = self.resolve(t, guard)?;
        let v = self.read(games_url("/v1/games", id), guard)?;
        let rows = array(&v, "data")?;
        if rows.len() != 1 {
            return Err(bad_reply());
        }
        let g: Game = serde_json::from_value(rows[0].clone()).map_err(|_| bad_reply())?;
        if g.id != id || !valid_id(g.root_place_id) || g.name.trim().is_empty() {
            return Err(bad_reply());
        }
        let v = self.read(games_url("/v1/games/votes", id), guard)?;
        let rows = array(&v, "data")?;
        if rows.len() != 1 {
            return Err(bad_reply());
        }
        let vote: Votes = serde_json::from_value(rows[0].clone()).map_err(|_| bad_reply())?;
        if vote.id != id {
            return Err(bad_reply());
        }
        let (total, pct) = like_percent(vote.up_votes, vote.down_votes)?;
        Ok(
            json!({"universeId":id,"requestedPlaceId":t.place,"rootPlaceId":g.root_place_id,"gameUrl":game_link(g.root_place_id),
            "name":g.name,"description":g.description,"creator":g.creator,"playing":g.playing,"playingScope":"experience/universe, not one server or subplace",
            "visits":g.visits,"favorites":g.favorited_count,"maxPlayersPerServer":g.max_players,"created":g.created,"updated":g.updated,
            "genre":g.genre,"genreL1":g.genre_l1,"genreL2":g.genre_l2,"paidAccessPriceInRobux":g.price,"isContentRestricted":g.is_content_restricted,
            "upVotes":vote.up_votes,"downVotes":vote.down_votes,"totalVotes":total,"likePercent":pct,
            "likePercentFormula":"100 * upVotes / (upVotes + downVotes); null when missing or zero votes",
            "snapshotOnly":true,"playabilityVerified":false,"musicFetched":false,"passesFetched":false,"productsFetched":false}),
        )
    }
    pub fn sorts<C: Clock>(
        &mut self,
        chart: &Chart,
        guard: &mut TrafficGuard<C>,
    ) -> Result<Value, AppError> {
        let v = self.read(chart.url(None)?, guard)?;
        let mut cards = Vec::new();
        let mut seen = HashSet::new();
        let mut rows = array(&v, "sorts")?.clone();
        if let Some(header) = v.get("header").filter(|v| !v.is_null()) {
            rows.extend(array(header, "sorts")?.iter().cloned());
        }
        for row in rows {
            let id = opt_string(&row, "sortId")?
                .filter(|s| text_ok(s, 128))
                .ok_or_else(bad_reply)?;
            if !seen.insert(id.clone()) {
                continue;
            }
            cards.push(json!({"sortId":id,"name":opt_string(&row,"sortDisplayName")?,"contentType":field(&row,"contentType")}));
        }
        let cursor = next(&v, "nextSortsPageToken")?;
        let returned = cards.len();
        cards.truncate(30);
        Ok(
            json!({"sorts":cards,"nextPageToken":cursor,"hasMore":cursor.is_some(),"country":chart.country,"device":chart.device.api(),
                "returnedUniqueSorts":returned,"truncatedLocally":returned>30,"automaticPagination":false}),
        )
    }
    pub fn chart_games<C: Clock>(
        &mut self,
        chart: &Chart,
        sort: &str,
        limit: u32,
        guard: &mut TrafficGuard<C>,
    ) -> Result<Value, AppError> {
        if limit == 0 || limit > 30 {
            return Err(invalid());
        }
        let v = self.read(chart.url(Some(sort))?, guard)?;
        if opt_string(&v, "sortId")?.is_some_and(|s| s != sort) {
            return Err(bad_reply());
        }
        let rows = array(&v, "games")?;
        let mut cards = Vec::new();
        let mut seen = HashSet::new();
        for (i, row) in rows.iter().take(limit as usize).enumerate() {
            let id = positive(row, "universeId")?;
            let p = row
                .get("rootPlaceId")
                .or_else(|| row.get("placeId"))
                .and_then(Value::as_u64)
                .filter(|n| valid_id(*n))
                .ok_or_else(bad_reply)?;
            if !seen.insert(id) {
                return Err(bad_reply());
            }
            let name = opt_string(row, "name")?
                .filter(|s| !s.trim().is_empty())
                .ok_or_else(bad_reply)?;
            let mut card = select(
                row,
                &[
                    "playerCount",
                    "totalUpVotes",
                    "totalDownVotes",
                    "minimumAge",
                    "creatorName",
                    "creatorType",
                    "creatorId",
                ],
            );
            let (votes, percent) =
                like_percent(count(row, "totalUpVotes")?, count(row, "totalDownVotes")?)?;
            count(row, "playerCount")?;
            card["totalVotes"] = votes.into();
            card["likePercent"] = percent.into();
            card["universeId"] = id.into();
            card["rootPlaceId"] = p.into();
            card["name"] = name.into();
            card["gameUrl"] = game_link(p);
            card["positionInPage"] = (i + 1).into();
            cards.push(card);
        }
        let cursor = next(&v, "nextPageToken")?;
        Ok(
            json!({"sortId":sort,"name":opt_string(&v,"sortDisplayName")?,"country":chart.country,"device":chart.device.api(),
            "requestedPageToken":chart.page_token,"requestedLimit":limit,
            "items":cards,"returnedByApi":rows.len(),"truncatedLocally":rows.len()>limit as usize,"omittedFromPage":rows.len().saturating_sub(limit as usize),"nextPageToken":cursor,"hasMorePages":cursor.is_some(),
            "automaticPagination":false,"detailFanout":false,"globalRankVerified":false,"snapshotOnly":true}),
        )
    }
    pub fn top_music<C: Clock>(
        &mut self,
        page: &Page,
        guard: &mut TrafficGuard<C>,
    ) -> Result<Value, AppError> {
        page.validate()?;
        let mut url = api_url("/music-discovery/v1/top-songs");
        url.query_pairs_mut()
            .append_pair("limit", &page.limit.to_string())
            .append_pair("pageToken", page.page_token.as_deref().unwrap_or("0"));
        let v = self.read(url, guard)?;
        music_page(&v, page.limit, false)
    }
    pub fn music<C: Clock>(
        &mut self,
        target: &Target,
        limit: u32,
        guard: &mut TrafficGuard<C>,
    ) -> Result<Value, AppError> {
        if limit == 0 || limit > 30 {
            return Err(invalid());
        }
        let id = self.resolve(target, guard)?;
        let mut url = api_url("/music-discovery/v1/experience-songs");
        url.query_pairs_mut()
            .append_pair("universeId", &id.to_string())
            .append_pair("limit", &limit.to_string());
        let v = self.read(url, guard)?;
        let mut out = music_page(&v, limit, true)?;
        out["universeId"] = id.into();
        out["requestedPlaceId"] = target.place.into();
        Ok(out)
    }
    pub fn commerce<C: Clock>(
        &mut self,
        target: &Target,
        page: &Page,
        passes: bool,
        guard: &mut TrafficGuard<C>,
    ) -> Result<Value, AppError> {
        page.validate()?;
        target.validate()?;
        let id = self.resolve(target, guard)?;
        let path = if passes {
            format!("/game-passes/v1/universes/{id}/game-passes")
        } else {
            format!("/developer-products/v2/universes/{id}/developerproducts")
        };
        let mut url = api_url(&path);
        {
            let mut q = url.query_pairs_mut();
            if passes {
                q.append_pair("passView", "Full")
                    .append_pair("pageSize", &page.limit.to_string());
            } else {
                q.append_pair("limit", &page.limit.to_string());
            }
            if let Some(t) = &page.page_token {
                q.append_pair(if passes { "pageToken" } else { "cursor" }, t);
            }
        }
        let v = self.read(url, guard)?;
        let key = if passes {
            "gamePasses"
        } else {
            "developerProducts"
        };
        let rows = array(&v, key)?;
        if rows.len() > page.limit as usize {
            return Err(bad_reply());
        }
        let mut cards = Vec::new();
        let mut seen = HashSet::new();
        for row in rows {
            let (item, mut card) = if passes {
                let p: Pass = serde_json::from_value(row.clone()).map_err(|_| bad_reply())?;
                if p.name.trim().is_empty() {
                    return Err(bad_reply());
                }
                (
                    p.id,
                    json!({"passId":p.id,"name":p.name,"priceInRobux":p.price,"userBasePriceInRobux":p.user_base_price_in_robux,
                    "isForSale":p.is_for_sale,"displayName":p.display_name,"displayDescription":p.display_description,
                    "iconImageAssetId":p.display_icon_image_asset_id,"created":p.created,"updated":p.updated,"creator":p.creator}),
                )
            } else {
                let p: Product = serde_json::from_value(row.clone()).map_err(|_| bad_reply())?;
                if p.name.trim().is_empty() || p.universe.is_some_and(|u| u != id) {
                    return Err(bad_reply());
                }
                (
                    p.id,
                    json!({"productId":p.id,"name":p.name,"description":p.description,"priceInRobux":p.price,
                    "userBasePriceInRobux":p.base_price,"isForSale":p.for_sale,"iconImageAssetId":p.icon,
                    "created":p.created,"updated":p.updated,"displayName":p.display_name,"displayDescription":p.display_description}),
                )
            };
            if !valid_id(item) {
                return Err(bad_reply());
            }
            if !seen.insert(item) {
                return Err(bad_reply());
            }
            card["priceCurrency"] = "Robux".into();
            card["ownershipVerified"] = false.into();
            card["purchasePerformed"] = false.into();
            if passes {
                card["passUrl"] = format!("https://www.roblox.com/game-pass/{item}").into();
            }
            cards.push(card);
        }
        let cursor = next(&v, "nextPageToken")?.or(next(&v, "nextPageCursor")?);
        Ok(
            json!({"universeId":id,"requestedPlaceId":target.place,"items":cards,"nextPageToken":cursor,"hasMore":cursor.is_some(),
            "requestedPageToken":page.page_token,"requestedLimit":page.limit,
            "automaticPagination":false,"detailFanout":false,"pricesAreAnonymousSnapshot":true,"personalizedCheckoutPriceVerified":false,
            "salesRevenueAvailable":false,"purchasePerformed":false}),
        )
    }

    pub fn icon<C: Clock>(
        &mut self,
        universe: u64,
        size: &str,
        format: &str,
        guard: &mut TrafficGuard<C>,
    ) -> Result<Value, AppError> {
        let value = self.send(
            Request::game_thumbnail(icon_url(universe, size, format)?)?,
            guard,
        )?;
        let row = one_thumbnail(&value, universe)?;
        thumbnail_card(universe, size, format, row)
    }

    pub fn thumbnails<C: Clock>(
        &mut self,
        universe: u64,
        size: &str,
        format: &str,
        count: u32,
        guard: &mut TrafficGuard<C>,
    ) -> Result<Value, AppError> {
        let value = self.send(
            Request::game_thumbnail(thumbnail_url(universe, size, format, count)?)?,
            guard,
        )?;
        let rows = array(&value, "data")?;
        if rows.len() != 1 || rows[0].get("universeId").and_then(Value::as_u64) != Some(universe) {
            return Err(bad_reply());
        }
        let images = rows[0]
            .get("thumbnails")
            .and_then(Value::as_array)
            .ok_or_else(bad_reply)?;
        if images.len() > count as usize {
            return Err(bad_reply());
        }
        let mut cards = Vec::new();
        for row in images {
            cards.push(thumbnail_card(universe, size, format, row)?);
        }
        Ok(json!({
            "universeId": universe,
            "size": size,
            "format": format,
            "requestedCount": count,
            "images": cards,
            "imageBytesFetched": false,
            "automaticPagination": false,
            "detailFanout": false,
            "rightsVerified": false,
            "ctrEstimated": false
        }))
    }

    pub fn badges<C: Clock>(
        &mut self,
        universe: u64,
        limit: &str,
        sort: &str,
        cursor: Option<&str>,
        guard: &mut TrafficGuard<C>,
    ) -> Result<Value, AppError> {
        let value = self.send(
            Request::badges(badges_url(universe, limit, sort, cursor)?)?,
            guard,
        )?;
        let rows = array(&value, "data")?;
        if rows.len() > limit.parse::<usize>().unwrap_or(0) {
            return Err(bad_reply());
        }
        let mut cards = Vec::new();
        let mut seen = HashSet::new();
        for row in rows {
            let id = positive(row, "id")?;
            if !seen.insert(id) {
                return Err(bad_reply());
            }
            let name = opt_string(row, "name")?
                .filter(|s| text_ok(s, 4096))
                .ok_or_else(bad_reply)?;
            let description = match row.get("description") {
                None | Some(Value::Null) => None,
                Some(Value::String(text))
                    if text.len() <= 4096 && !text.chars().any(forbidden_control) =>
                {
                    Some(text.clone())
                }
                _ => return Err(bad_reply()),
            };
            let awarding = row.get("awardingUniverse").filter(|value| !value.is_null());
            if let Some(awarding) = awarding
                && awarding.get("id").and_then(Value::as_u64) != Some(universe)
            {
                return Err(bad_reply());
            }
            let (past_day, awarded, win_rate) =
                if let Some(stats) = row.get("statistics").filter(|value| !value.is_null()) {
                    (
                        opt_u64(stats, "pastDayAwardedCount")?,
                        opt_u64(stats, "awardedCount")?,
                        opt_f64(stats, "winRatePercentage")?,
                    )
                } else {
                    (None, None, None)
                };
            cards.push(json!({
                "badgeId": id,
                "name": name,
                "description": description,
                "enabled": opt_bool(row, "enabled")?,
                "iconImageId": opt_id(row, "iconImageId")?,
                "displayIconImageId": opt_id(row, "displayIconImageId")?,
                "awardingUniverseId": awarding.and_then(|value| value.get("id")).and_then(Value::as_u64),
                "awardingRootPlaceId": awarding.map(|value| opt_id(value, "rootPlaceId")).transpose()?.flatten(),
                "statistics": {
                    "pastDayAwardedCount": past_day,
                    "awardedCount": awarded,
                    "winRatePercentage": win_rate,
                    "winRateIsNotCompletionOrChurn": true
                },
                "untrustedText": true
            }));
        }
        let next_cursor = next(&value, "nextPageCursor")?;
        Ok(json!({
            "universeId": universe,
            "limit": limit.parse::<u32>().unwrap_or(0),
            "sortOrder": sort,
            "requestedCursor": cursor,
            "items": cards,
            "nextPageCursor": next_cursor,
            "hasMore": next_cursor.is_some(),
            "emptyPageIsNotProofOfNoBadges": true,
            "automaticPagination": false,
            "detailFanout": false,
            "ownershipVerified": false,
            "awardPerformed": false
        }))
    }
}

fn music_page(v: &Value, limit: u32, experience: bool) -> Result<Value, AppError> {
    let rows = array(v, "songs")?;
    if rows.len() > limit as usize {
        return Err(bad_reply());
    }
    let mut cards = Vec::new();
    let mut seen = HashSet::new();
    for (i, row) in rows.iter().enumerate() {
        let id = positive(row, "assetId")?;
        if !seen.insert(id) {
            return Err(bad_reply());
        }
        let mut card = select(
            row,
            &[
                "title",
                "artist",
                "album",
                "albumArtAssetId",
                "duration",
                "rank",
                "previousRank",
            ],
        );
        opt_string(row, "title")?
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(bad_reply)?;
        count(row, "duration")?;
        card["assetId"] = id.into();
        card["positionInPage"] = (i + 1).into();
        card["studioAssetId"] = format!("rbxassetid://{id}").into();
        card["storeUrl"] = format!("https://create.roblox.com/store/asset/{id}").into();
        card["usagePermissionVerified"] = false.into();
        cards.push(card);
    }
    let cursor = next(v, "nextPageToken")?;
    Ok(
        json!({"items":cards,"nextPageToken":cursor,"hasMore":cursor.is_some(),"automaticPagination":false,"detailFanout":false,
        "source":if experience {"Roblox experience Top Songs"} else {"Roblox Music Top 100"},
        "allBgmEnumerated":false,"liveNowPlaying":false,"experienceRankingWindow":if experience {Some("last 7 days; supported licensed/ISRC tracks only")} else {None},
        "playbackPerformed":false,"usagePermissionVerified":false}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::traffic::tests::FakeClock;
    use crate::transport::HttpResponse;
    use std::collections::VecDeque;
    struct Fake {
        replies: VecDeque<Value>,
        records: Vec<Value>,
        status: u16,
        challenge: bool,
    }
    impl Transport for Fake {
        fn send(&mut self, r: Request) -> Result<HttpResponse, AppError> {
            let mut record = r.snapshot();
            record["url"] = r.url().as_str().into();
            self.records.push(record);
            Ok(HttpResponse {
                status: self.status,
                body: serde_json::to_vec(&self.replies.pop_front().unwrap_or(json!({}))).unwrap(),
                retry_after_seconds: None,
                csrf: None,
                challenge: self.challenge,
            })
        }
    }
    fn fake(v: Vec<Value>) -> Discovery<Fake> {
        Discovery::new(Fake {
            replies: v.into(),
            records: vec![],
            status: 200,
            challenge: false,
        })
    }
    fn target() -> Target {
        Target {
            universe: Some(77),
            place: None,
        }
    }
    fn page() -> Page {
        Page {
            limit: 10,
            page_token: None,
        }
    }
    fn chart() -> Chart {
        Chart {
            device: Device::Computer,
            country: "all".into(),
            page_token: None,
        }
    }
    #[test]
    fn anonymous_route_boundaries_and_query_injection_are_blocked() {
        for url in [
            "https://games.roblox.com/v1/games?universeIds=77",
            "https://apis.roblox.com/explore-api/v1/get-sort-content?sortId=top-playing-now",
            "https://apis.roblox.com/music-discovery/v1/top-songs?limit=10&pageToken=0",
            "https://apis.roblox.com/universes/v1/places/77/universe",
            "https://apis.roblox.com/game-passes/v1/universes/77/game-passes?passView=Full&pageSize=10",
            "https://apis.roblox.com/developer-products/v2/universes/77/developerproducts?limit=10",
        ] {
            let u = Url::parse(url).unwrap();
            let r = Request::discovery(u.clone()).unwrap();
            assert_eq!(r.snapshot()["cookieAttached"], false);
            assert_eq!(r.snapshot()["csrfAttached"], false);
            assert_eq!(r.snapshot()["method"], "GET");
            assert!(
                Request::new(u, crate::auth::SessionCookie::synthetic().header().unwrap()).is_err()
            );
        }
        for url in [
            "https://apis.roblox.com/game-passes/v1/universes/77/game-passes/creator",
            "https://apis.roblox.com/developer-products/v2/universes/77/developer-products/creator",
            "https://games.roblox.com/v1/games?Cookie=secret",
            "https://apis.roblox.com/music-discovery/v1/top-songs?url=https://evil.test",
            "https://apis.roblox.com/universes/v1/places/0/universe",
            "https://games.roblox.com/v1/games/77/votes/user",
            "https://apis.roblox.com/developer-products/v1/universes/77/developerproducts",
            "https://apis.roblox.com/game-passes/v1/universes/77/game-passes?userId=88",
            "https://assetdelivery.roblox.com/v1/asset/?id=77",
        ] {
            assert!(Request::discovery(Url::parse(url).unwrap()).is_err());
        }
    }
    #[test]
    fn game_snapshot_matches_universe_joins_votes_and_never_fans_out() {
        let root = tempfile::tempdir().unwrap();
        let mut g = TrafficGuard::acquire(root.path(), FakeClock::default()).unwrap();
        let mut c = fake(vec![
            json!({"data":[{"id":77,"rootPlaceId":88,"name":"Synthetic","playing":123,"visits":999,"favoritedCount":5,"private":"discard"}]}),
            json!({"data":[{"id":77,"upVotes":9,"downVotes":1}]}),
        ]);
        let v = c.game(&target(), &mut g).unwrap();
        assert_eq!(v["playing"], 123);
        assert_eq!(v["likePercent"], 90.0);
        assert_eq!(v["rootPlaceId"], 88);
        assert_eq!(v["musicFetched"], false);
        assert_eq!(v["paidAccessPriceInRobux"], Value::Null);
        assert!(!v.to_string().contains("discard"));
        assert_eq!(c.transport.records.len(), 2);
    }
    #[test]
    fn zero_votes_are_unknown_not_zero_percent_and_wrong_targets_stop() {
        for (response_id, up, down) in [(77, 0, 0), (99, 3, 1)] {
            let root = tempfile::tempdir().unwrap();
            let mut g = TrafficGuard::acquire(root.path(), FakeClock::default()).unwrap();
            let mut c = fake(vec![
                json!({"data":[{"id":77,"rootPlaceId":88,"name":"Game"}]}),
                json!({"data":[{"id":response_id,"upVotes":up,"downVotes":down}]}),
            ]);
            let v = c.game(&target(), &mut g);
            if response_id == 77 {
                assert_eq!(v.unwrap()["likePercent"], Value::Null)
            } else {
                assert_eq!(v.unwrap_err().code, "discovery_response_invalid");
            }
        }
    }
    #[test]
    fn place_resolution_is_explicit_and_ids_are_not_swapped() {
        let root = tempfile::tempdir().unwrap();
        let mut g = TrafficGuard::acquire(root.path(), FakeClock::default()).unwrap();
        let mut c = fake(vec![json!({"universeId":77}), json!({"songs":[]})]);
        let v = c
            .music(
                &Target {
                    universe: None,
                    place: Some(88),
                },
                10,
                &mut g,
            )
            .unwrap();
        assert_eq!(v["universeId"], 77);
        assert_eq!(v["requestedPlaceId"], 88);
        assert_eq!(c.transport.records.len(), 2);
        assert!(
            c.transport.records[0]["url"]
                .as_str()
                .unwrap()
                .contains("/places/88/universe")
        );
    }
    #[test]
    fn chart_order_and_server_tokens_are_preserved_without_enrichment() {
        let root = tempfile::tempdir().unwrap();
        let mut g = TrafficGuard::acquire(root.path(), FakeClock::default()).unwrap();
        let mut c = fake(vec![
            json!({"sortId":"top-playing-now","games":[{"universeId":77,"rootPlaceId":88,"name":"Chart game","playerCount":123}],"nextPageToken":"next"}),
        ]);
        let v = c
            .chart_games(&chart(), "top-playing-now", 10, &mut g)
            .unwrap();
        assert_eq!(v["items"][0]["positionInPage"], 1);
        assert_eq!(v["items"][0]["playerCount"], 123);
        assert_eq!(v["nextPageToken"], "next");
        assert_eq!(v["globalRankVerified"], false);
        assert_eq!(c.transport.records.len(), 1);
        assert!(
            Chart {
                country: "bad&Cookie=private".into(),
                ..chart()
            }
            .url(None)
            .is_err()
        );
        assert!(chart().url(Some("bad/../route")).is_err());
    }
    #[test]
    fn songs_are_not_a_full_bgm_scan_or_playback() {
        let root = tempfile::tempdir().unwrap();
        let mut g = TrafficGuard::acquire(root.path(), FakeClock::default()).unwrap();
        let mut c = fake(vec![
            json!({"songs":[{"assetId":99,"title":"Synthetic","artist":"Artist","duration":80,"sourceUrl":"do-not-follow"}]}),
        ]);
        let v = c.music(&target(), 10, &mut g).unwrap();
        assert_eq!(v["items"][0]["assetId"], 99);
        assert_eq!(v["allBgmEnumerated"], false);
        assert_eq!(v["liveNowPlaying"], false);
        assert!(!v.to_string().contains("do-not-follow"));
        assert_eq!(c.transport.records.len(), 1);
    }
    #[test]
    fn commerce_ids_prices_and_ownership_are_not_conflated() {
        for passes in [true, false] {
            let root = tempfile::tempdir().unwrap();
            let mut g = TrafficGuard::acquire(root.path(), FakeClock::default()).unwrap();
            let reply = if passes {
                json!({"gamePasses":[{"id":99,"productId":555,"name":"Pass","price":10,"isOwned":true}],"nextPageToken":"next"})
            } else {
                json!({"developerProducts":[{"ProductId":99,"DeveloperProductId":555,"Name":"Product","PriceInRobux":null,"UniverseId":77,"displayName":"Localized","DisplayName":"Localized"}],"nextPageCursor":"next"})
            };
            let mut c = fake(vec![reply]);
            let v = c.commerce(&target(), &page(), passes, &mut g).unwrap();
            assert_eq!(
                v["items"][0][if passes { "passId" } else { "productId" }],
                99
            );
            assert_eq!(v["items"][0]["ownershipVerified"], false);
            assert!(v["items"][0].get("isOwned").is_none());
            assert!(v["items"][0].get("DeveloperProductId").is_none());
            assert_eq!(
                v["items"][0]["priceInRobux"],
                if passes { json!(10) } else { Value::Null }
            );
            assert_eq!(v["hasMore"], true);
            assert_eq!(c.transport.records.len(), 1);
        }
    }
    #[test]
    fn refusals_stop_without_echo_retry_or_authentication_fallback() {
        for status in [200, 302, 401, 403, 404, 429, 500] {
            let root = tempfile::tempdir().unwrap();
            let mut g = TrafficGuard::acquire(root.path(), FakeClock::default()).unwrap();
            let mut c = fake(vec![json!({"private":"must-not-echo"})]);
            c.transport.status = status;
            c.transport.challenge = status == 200;
            let e = c.top_music(&page(), &mut g).unwrap_err();
            assert_eq!(e.exit_code, 3);
            assert!(!serde_json::to_string(&e).unwrap().contains("must-not-echo"));
            assert_eq!(c.transport.records.len(), 1);
            assert_eq!(c.transport.records[0]["cookieAttached"], false);
        }
    }
    #[test]
    fn sorts_list_deduplicates_headers_and_preserves_an_explicit_next_page() {
        let root = tempfile::tempdir().unwrap();
        let mut g = TrafficGuard::acquire(root.path(), FakeClock::default()).unwrap();
        let mut c = fake(vec![
            json!({"sorts":[{"sortId":"top-playing-now","sortDisplayName":"Top Playing Now","contentType":"Games","games":[{"private":"discard"}]}],
            "header":{"sorts":[{"sortId":"top-playing-now"},{"sortId":"other"}]},"nextSortsPageToken":"next"}),
        ]);
        let v = c.sorts(&chart(), &mut g).unwrap();
        assert_eq!(v["sorts"].as_array().unwrap().len(), 2);
        assert_eq!(v["nextPageToken"], "next");
        assert!(!v.to_string().contains("discard"));
        assert_eq!(c.transport.records.len(), 1);
    }
    #[test]
    fn local_chart_truncation_never_fabricates_an_api_cursor_or_global_rank() {
        let root = tempfile::tempdir().unwrap();
        let mut g = TrafficGuard::acquire(root.path(), FakeClock::default()).unwrap();
        let mut c = fake(vec![
            json!({"games":[{"universeId":77,"rootPlaceId":88,"name":"First","totalUpVotes":9,"totalDownVotes":1},{"universeId":99,"rootPlaceId":100,"name":"Second"}]}),
        ]);
        let v = c
            .chart_games(&chart(), "top-playing-now", 1, &mut g)
            .unwrap();
        assert_eq!(v["truncatedLocally"], true);
        assert_eq!(v["omittedFromPage"], 1);
        assert_eq!(v["hasMorePages"], false);
        assert_eq!(v["nextPageToken"], Value::Null);
        assert_eq!(v["items"][0]["likePercent"], 90.0);
        assert!(like_percent(Some(u64::MAX), Some(1)).is_err());
        assert_eq!(like_percent(None, Some(1)).unwrap(), (None, None));
    }
    #[test]
    fn wrong_universe_and_malformed_music_ids_stop_without_additional_reads() {
        let root = tempfile::tempdir().unwrap();
        let mut g = TrafficGuard::acquire(root.path(), FakeClock::default()).unwrap();
        let mut c = fake(vec![
            json!({"data":[{"id":99,"rootPlaceId":88,"name":"Other universe"}]}),
        ]);
        assert_eq!(
            c.game(&target(), &mut g).unwrap_err().code,
            "discovery_response_invalid"
        );
        assert_eq!(c.transport.records.len(), 1);
        let mut c = fake(vec![json!({"songs":[{"assetId":0,"title":"Bad"}]})]);
        assert!(c.top_music(&page(), &mut g).is_err());
        assert_eq!(c.transport.records.len(), 1);
    }
    #[test]
    fn products_wrong_universe_and_unexpected_page_sizes_fail_closed() {
        let root = tempfile::tempdir().unwrap();
        let mut g = TrafficGuard::acquire(root.path(), FakeClock::default()).unwrap();
        let mut c = fake(vec![
            json!({"developerProducts":[{"ProductId":99,"UniverseId":42,"Name":"Other"}]}),
        ]);
        assert!(c.commerce(&target(), &page(), false, &mut g).is_err());
        assert_eq!(c.transport.records.len(), 1);
        let mut c = fake(vec![]);
        assert!(
            c.top_music(
                &Page {
                    limit: 31,
                    ..page()
                },
                &mut g
            )
            .is_err()
        );
        assert!(c.transport.records.is_empty());
    }
    #[test]
    fn explicit_page_tokens_are_encoded_once_not_used_as_urls_or_credentials() {
        let root = tempfile::tempdir().unwrap();
        let mut g = TrafficGuard::acquire(root.path(), FakeClock::default()).unwrap();
        let mut c = fake(vec![
            json!({"developerProducts":[],"nextPageCursor":"next"}),
        ]);
        c.commerce(
            &target(),
            &Page {
                page_token: Some("next&Cookie=not-a-header".into()),
                ..page()
            },
            false,
            &mut g,
        )
        .unwrap();
        let url = Url::parse(c.transport.records[0]["url"].as_str().unwrap()).unwrap();
        assert!(
            url.query_pairs()
                .any(|(k, v)| k == "cursor" && v == "next&Cookie=not-a-header")
        );
        assert!(!url.query_pairs().any(|(k, _)| k == "Cookie"));
        assert_eq!(c.transport.records[0]["cookieAttached"], false);
        assert!(!token_ok(&Some("\n".into())));
    }
    #[test]
    fn sort_catalog_has_a_bounded_output_without_fetching_more_pages() {
        let root = tempfile::tempdir().unwrap();
        let mut g = TrafficGuard::acquire(root.path(), FakeClock::default()).unwrap();
        let rows: Vec<_> = (0..31)
            .map(|n| json!({"sortId":format!("sort-{n}")}))
            .collect();
        let mut c = fake(vec![json!({"sorts":rows})]);
        let v = c.sorts(&chart(), &mut g).unwrap();
        assert_eq!(v["sorts"].as_array().unwrap().len(), 30);
        assert_eq!(v["truncatedLocally"], true);
        assert_eq!(c.transport.records.len(), 1);
    }

    #[test]
    fn icons_thumbnails_and_badges_are_one_anonymous_get() {
        let root = tempfile::tempdir().unwrap();
        let mut guard = TrafficGuard::acquire(root.path(), FakeClock::default()).unwrap();
        let mut client = fake(vec![json!({
            "data": [{ "targetId": 77, "state": "Pending", "imageUrl": "https://tr.rbxcdn.com/hidden" }]
        })]);
        let pending = client.icon(77, "150x150", "Png", &mut guard).unwrap();
        assert_eq!(pending["imageReady"], false);
        assert_eq!(pending["imageUrl"], Value::Null);
        assert_eq!(pending["imageBytesFetched"], false);
        assert_eq!(client.transport.records[0]["cookieAttached"], false);
        assert!(
            client.transport.records[0]["url"]
                .as_str()
                .unwrap()
                .starts_with("https://thumbnails.roblox.com/v1/games/icons?")
        );

        let mut client = fake(vec![json!({
            "data": [{ "targetId": 77, "state": "Completed", "imageUrl": "https://tr.rbxcdn.com/icon.png" }]
        })]);
        let ready = client.icon(77, "150x150", "Png", &mut guard).unwrap();
        assert_eq!(ready["imageUrl"], "https://tr.rbxcdn.com/icon.png");

        let mut client = fake(vec![json!({
            "data": [{ "id": 5, "name": "Other", "awardingUniverse": { "id": 99 } }]
        })]);
        assert_eq!(
            client
                .badges(77, "10", "Asc", None, &mut guard)
                .unwrap_err()
                .code,
            "discovery_response_invalid"
        );

        let mut client = fake(vec![json!({ "data": [], "nextPageCursor": null })]);
        let page = client.badges(77, "10", "Asc", None, &mut guard).unwrap();
        assert_eq!(page["items"].as_array().unwrap().len(), 0);
        assert_eq!(page["emptyPageIsNotProofOfNoBadges"], true);
        assert_eq!(client.transport.records[0]["cookieAttached"], false);
        assert!(
            client.transport.records[0]["url"]
                .as_str()
                .unwrap()
                .contains("badges.roblox.com/v1/universes/77/badges")
        );
    }
}
