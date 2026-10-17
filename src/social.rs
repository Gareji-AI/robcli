//! Anonymous profile and community metadata, not a social graph harvester or admin client.
use clap::Args;
use reqwest::Url;
use serde::{Deserialize, de::DeserializeOwned};
use serde_json::{Value, json};
use std::collections::HashSet;

use crate::error::AppError;
use crate::traffic::{Clock, TrafficGuard};
use crate::transport::{Request, Transport};

fn valid_id(id: u64) -> bool {
    id > 0 && id <= i64::MAX as u64
}
fn invalid() -> AppError {
    AppError::new(
        "social_query_invalid",
        "The public profile/community query is invalid.",
        "Use a positive User/Community ID, a short keyword, page size 10/25, and a short opaque cursor. Local unpaged output is capped at 25. No credentials were accessed.",
    )
}
fn bad_reply() -> AppError {
    AppError::network(
        "social_response_invalid",
        "The public response did not match the expected profile, community, owner or schema. No raw body is printed.",
    )
}
fn text_ok(s: &str, max: usize) -> bool {
    !s.trim().is_empty() && s.len() <= max && !s.chars().any(char::is_control)
}

#[derive(Debug, Args)]
pub(crate) struct UserTarget {
    #[arg(long, value_parser=clap::value_parser!(u64).range(1..=9223372036854775807))]
    pub user: u64,
}
#[derive(Debug, Args)]
pub(crate) struct CommunityTarget {
    /// Community ID (called groupId by the public API), not a User ID.
    #[arg(long, alias="group", value_parser=clap::value_parser!(u64).range(1..=9223372036854775807))]
    pub community: u64,
}
#[derive(Debug, Args)]
pub(crate) struct Page {
    /// One API page of 10 or 25 items, never all pages.
    #[arg(long, default_value="10", value_parser=clap::value_parser!(u32).range(1..=25))]
    pub limit: u32,
    #[arg(long)]
    pub cursor: Option<String>,
}
impl Page {
    pub fn validate(&self) -> Result<(), AppError> {
        if ![10, 25].contains(&self.limit)
            || self.cursor.as_ref().is_some_and(|s| !text_ok(s, 4096))
        {
            return Err(invalid());
        }
        Ok(())
    }
    fn append(&self, url: &mut Url) -> Result<(), AppError> {
        self.validate()?;
        let mut q = url.query_pairs_mut();
        q.append_pair("limit", &self.limit.to_string());
        if let Some(s) = &self.cursor {
            q.append_pair("cursor", s);
        }
        Ok(())
    }
}
#[derive(Debug, Args)]
pub(crate) struct Search {
    /// Search is not an exact identity/ownership verification.
    #[arg(long, short = 'q')]
    pub query: String,
    #[command(flatten)]
    pub page: Page,
}
impl Search {
    pub fn validate(&self) -> Result<(), AppError> {
        self.page.validate()?;
        if !text_ok(&self.query, 512) {
            return Err(invalid());
        }
        Ok(())
    }
}
pub(crate) fn validate_target(id: u64) -> Result<(), AppError> {
    if valid_id(id) { Ok(()) } else { Err(invalid()) }
}
pub(crate) fn link(id: u64, community: bool) -> Result<Value, AppError> {
    validate_target(id)?;
    Ok(
        json!({"id":id,"kind":if community {"Community"}else{"User"},"url":public_url(id,community),
        "existsChecked":false,"browserOpened":false,"accountModified":false}),
    )
}
fn public_url(id: u64, community: bool) -> String {
    if community {
        format!("https://www.roblox.com/communities/{id}")
    } else {
        format!("https://www.roblox.com/users/{id}/profile")
    }
}
fn url(host: &str, path: &str) -> Url {
    Url::parse(&format!("https://{host}.roblox.com{path}")).unwrap()
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct User {
    id: u64,
    name: String,
    display_name: Option<String>,
    description: Option<String>,
    created: Option<String>,
    is_banned: Option<bool>,
    has_verified_badge: Option<bool>,
}
impl User {
    fn card(self) -> Result<Value, AppError> {
        if !valid_id(self.id) || self.name.trim().is_empty() {
            return Err(bad_reply());
        }
        Ok(
            json!({"userId":self.id,"name":self.name,"displayName":self.display_name,"description":self.description,
            "created":self.created,"isBanned":self.is_banned,"hasVerifiedBadge":self.has_verified_badge,"profileUrl":public_url(self.id,false),
            "identityVerified":false,"accountModified":false}),
        )
    }
}
#[derive(Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct UserRef {
    user_id: u64,
    username: Option<String>,
    display_name: Option<String>,
    has_verified_badge: Option<bool>,
}
impl UserRef {
    fn validate(&self) -> Result<(), AppError> {
        if valid_id(self.user_id) {
            Ok(())
        } else {
            Err(bad_reply())
        }
    }
}
#[derive(Deserialize, serde::Serialize)]
struct Shout {
    body: Option<String>,
    poster: Option<UserRef>,
    created: Option<String>,
    updated: Option<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Community {
    id: u64,
    name: String,
    description: Option<String>,
    member_count: Option<u64>,
    owner: Option<UserRef>,
    shout: Option<Shout>,
    has_verified_badge: Option<bool>,
    public_entry_allowed: Option<bool>,
    is_locked: Option<bool>,
    is_deactivated: Option<bool>,
    has_social_modules: Option<bool>,
    created: Option<String>,
    updated: Option<String>,
}
impl Community {
    fn card(self) -> Result<Value, AppError> {
        if !valid_id(self.id) || self.name.trim().is_empty() {
            return Err(bad_reply());
        }
        if let Some(owner) = &self.owner {
            owner.validate()?;
        }
        if let Some(poster) = self.shout.as_ref().and_then(|s| s.poster.as_ref()) {
            poster.validate()?;
        }
        Ok(
            json!({"communityId":self.id,"name":self.name,"description":self.description,"memberCount":self.member_count,
            "audienceLabel":self.has_social_modules.map(|social|if social {"followers"}else{"members"}),"hasSocialModules":self.has_social_modules,
            "owner":self.owner,"shout":self.shout,"hasVerifiedBadge":self.has_verified_badge,"publicEntryAllowed":self.public_entry_allowed,
            "isLocked":self.is_locked,"isDeactivated":self.is_deactivated,"created":self.created,"updated":self.updated,
            "communityUrl":public_url(self.id,true),"managementPermissionsVerified":false,"accountModified":false}),
        )
    }
}
#[derive(Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct Role {
    id: u64,
    name: String,
    description: Option<String>,
    rank: Option<u32>,
    member_count: Option<u64>,
    is_base: Option<bool>,
    is_private: Option<bool>,
}
impl Role {
    fn validate(&self) -> Result<(), AppError> {
        // The public Guest roleset legitimately has id=0, rank=0.
        if self.id > i64::MAX as u64
            || (self.id == 0 && self.rank != Some(0))
            || self.name.trim().is_empty()
            || self.is_private == Some(true)
        {
            return Err(bad_reply());
        }
        Ok(())
    }
}
#[derive(Deserialize)]
struct Membership {
    group: Community,
    role: Option<Role>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Roles {
    group_id: u64,
    roles: Vec<Role>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct List<T> {
    data: Vec<T>,
    next_page_cursor: Option<String>,
    previous_page_cursor: Option<String>,
}
#[derive(Deserialize)]
#[serde(untagged)]
enum EntityKind {
    Number(u32),
    Name(String),
}
impl EntityKind {
    fn owner_matches(&self, community: bool) -> bool {
        match self {
            Self::Number(n) => *n == if community { 1 } else { 0 },
            Self::Name(s) => s == if community { "Group" } else { "User" },
        }
    }
    fn is_place(&self) -> bool {
        match self {
            Self::Number(n) => *n == 9,
            Self::Name(s) => s == "Place",
        }
    }
}
#[derive(Deserialize)]
struct Entity {
    id: u64,
    #[serde(rename = "type")]
    kind: Option<EntityKind>,
    name: Option<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Game {
    id: u64,
    name: String,
    description: Option<String>,
    creator: Option<Entity>,
    root_place: Option<Entity>,
    created: Option<String>,
    updated: Option<String>,
    place_visits: Option<u64>,
}
impl Game {
    fn card(self, owner: u64, community: bool) -> Result<Value, AppError> {
        if !valid_id(self.id) || self.name.trim().is_empty() {
            return Err(bad_reply());
        }
        let matches = if let Some(c) = &self.creator {
            if c.id != owner || !c.kind.as_ref().is_some_and(|k| k.owner_matches(community)) {
                return Err(bad_reply());
            }
            Some(true)
        } else {
            None
        };
        let place = self.root_place.as_ref().map(|p| p.id);
        if place.is_some_and(|p| !valid_id(p))
            || self
                .root_place
                .as_ref()
                .and_then(|p| p.kind.as_ref())
                .is_some_and(|kind| !kind.is_place())
        {
            return Err(bad_reply());
        }
        Ok(
            json!({"universeId":self.id,"rootPlaceId":place,"name":self.name,"description":self.description,
            "creatorName":self.creator.and_then(|c|c.name),"creatorMatchesTarget":matches,"created":self.created,"updated":self.updated,
            "placeVisits":self.place_visits,"gameUrl":place.map(|p|format!("https://www.roblox.com/games/{p}")),
            "playing":null,"votes":null,"detailFetched":false,"playabilityVerified":false}),
        )
    }
}

fn cursors(
    next: Option<String>,
    previous: Option<String>,
) -> Result<(Option<String>, Option<String>), AppError> {
    let next = next.filter(|s| !s.is_empty());
    let previous = previous.filter(|s| !s.is_empty());
    if [&next, &previous]
        .into_iter()
        .flatten()
        .any(|s| !text_ok(s, 4096))
    {
        return Err(bad_reply());
    }
    Ok((next, previous))
}
fn paged<T>(
    page: List<T>,
    limit: u32,
    project: impl Fn(T) -> Result<Value, AppError>,
    id_key: &str,
) -> Result<Value, AppError> {
    if page.data.len() > limit as usize {
        return Err(bad_reply());
    }
    let (next, previous) = cursors(page.next_page_cursor, page.previous_page_cursor)?;
    let mut seen = HashSet::new();
    let mut items = Vec::new();
    for row in page.data {
        let card = project(row)?;
        let id = card[id_key]
            .as_u64()
            .filter(|n| valid_id(*n))
            .ok_or_else(bad_reply)?;
        if !seen.insert(id) {
            return Err(bad_reply());
        }
        items.push(card);
    }
    Ok(
        json!({"count":items.len(),"items":items,"nextPageCursor":next,"previousPageCursor":previous,"hasMore":next.is_some(),
        "automaticPagination":false,"detailFanout":false}),
    )
}
fn local_limit(limit: u32) -> Result<(), AppError> {
    if !(1..=25).contains(&limit) {
        return Err(invalid());
    }
    Ok(())
}

pub(crate) struct Social<T: Transport> {
    pub transport: T,
}
impl<T: Transport> Social<T> {
    pub fn new(transport: T) -> Self {
        Self { transport }
    }
    fn read<R: DeserializeOwned, C: Clock>(
        &mut self,
        u: Url,
        g: &mut TrafficGuard<C>,
    ) -> Result<R, AppError> {
        let request = Request::social(u)?;
        g.reserve()?;
        let r = self.transport.send(request)?;
        if r.challenge || !(200..300).contains(&r.status) {
            let code = if r.challenge {
                "social_verification_required"
            } else {
                match r.status {
                    300..=399 => "redirect_refused",
                    401 | 403 => "social_access_refused",
                    404 => "social_not_found",
                    429 => "rate_limited",
                    500..=599 => "server_error",
                    _ => "social_request_failed",
                }
            };
            let mut e = AppError::new(
                code,
                "The anonymous public profile/community read was refused; no retry or credential fallback was attempted.",
                "Stop. Do not attach credentials, rotate accounts, bypass privacy settings or probe alternate endpoints to overcome a refusal.",
            );
            e.http_status = Some(r.status);
            e.retry_after_seconds = r.retry_after_seconds;
            return Err(e);
        }
        serde_json::from_slice(&r.body).map_err(|_| bad_reply())
    }
    pub fn user<C: Clock>(&mut self, id: u64, g: &mut TrafficGuard<C>) -> Result<Value, AppError> {
        validate_target(id)?;
        let v: User = self.read(url("users", &format!("/v1/users/{id}")), g)?;
        if v.id != id {
            return Err(bad_reply());
        }
        v.card()
    }
    pub fn community<C: Clock>(
        &mut self,
        id: u64,
        g: &mut TrafficGuard<C>,
    ) -> Result<Value, AppError> {
        validate_target(id)?;
        let v: Community = self.read(url("groups", &format!("/v1/groups/{id}")), g)?;
        if v.id != id {
            return Err(bad_reply());
        }
        v.card()
    }
    pub fn search<C: Clock>(
        &mut self,
        q: &Search,
        community: bool,
        g: &mut TrafficGuard<C>,
    ) -> Result<Value, AppError> {
        q.validate()?;
        let mut u = if community {
            url("groups", "/v1/groups/search")
        } else {
            url("users", "/v1/users/search")
        };
        q.page.append(&mut u)?;
        u.query_pairs_mut().append_pair("keyword", &q.query);
        let mut out = if community {
            let v: List<Community> = self.read(u, g)?;
            paged(v, q.page.limit, Community::card, "communityId")?
        } else {
            let v: List<User> = self.read(u, g)?;
            paged(v, q.page.limit, User::card, "userId")?
        };
        out["query"] = q.query.clone().into();
        out["exactIdentityVerified"] = false.into();
        Ok(out)
    }
    pub fn memberships<C: Clock>(
        &mut self,
        id: u64,
        limit: u32,
        g: &mut TrafficGuard<C>,
    ) -> Result<Value, AppError> {
        validate_target(id)?;
        local_limit(limit)?;
        let mut u = url("groups", &format!("/v2/users/{id}/groups/roles"));
        u.query_pairs_mut()
            .append_pair("includeLocked", "false")
            .append_pair("includeNotificationPreferences", "false")
            .append_pair("discoveryType", "0");
        let page: List<Membership> = self.read(u, g)?;
        if page.data.len() > 1000
            || page
                .next_page_cursor
                .as_ref()
                .is_some_and(|s| !s.is_empty())
            || page
                .previous_page_cursor
                .as_ref()
                .is_some_and(|s| !s.is_empty())
        {
            return Err(bad_reply());
        }
        let returned = page.data.len();
        let mut seen = HashSet::new();
        let mut items = Vec::new();
        for row in page.data.into_iter().take(limit as usize) {
            if let Some(role) = &row.role {
                role.validate()?;
            }
            let community = row.group.card()?;
            let id = community["communityId"].as_u64().ok_or_else(bad_reply)?;
            if !seen.insert(id) {
                return Err(bad_reply());
            }
            items.push(json!({"community":community,"role":row.role,"managementPermissionsVerified":false}));
        }
        Ok(
            json!({"userId":id,"items":items,"returnedByApi":returned,"truncatedLocally":returned>limit as usize,
            "omittedFromResponse":returned.saturating_sub(limit as usize),"apiHasNoCursor":true,"automaticPagination":false,"allMembershipsVerified":false}),
        )
    }
    pub fn roles<C: Clock>(
        &mut self,
        id: u64,
        limit: u32,
        g: &mut TrafficGuard<C>,
    ) -> Result<Value, AppError> {
        validate_target(id)?;
        local_limit(limit)?;
        let mut u = url("groups", &format!("/v1/groups/{id}/roles"));
        u.query_pairs_mut().append_pair("includePrivate", "false");
        let page: Roles = self.read(u, g)?;
        if page.group_id != id || page.roles.len() > 1000 {
            return Err(bad_reply());
        }
        let returned = page.roles.len();
        let mut items = Vec::new();
        let mut seen = HashSet::new();
        for r in page.roles {
            r.validate()?;
            if !seen.insert(r.id) {
                return Err(bad_reply());
            }
            if items.len() < limit as usize {
                items.push(r);
            }
        }
        Ok(
            json!({"communityId":id,"roles":items,"returnedByApi":returned,"truncatedLocally":returned>limit as usize,
            "includePrivate":false,"apiHasNoCursor":true,"automaticPagination":false,"memberRosterFetched":false,"managementPermissionsVerified":false}),
        )
    }
    pub fn games<C: Clock>(
        &mut self,
        id: u64,
        community: bool,
        page: &Page,
        g: &mut TrafficGuard<C>,
    ) -> Result<Value, AppError> {
        validate_target(id)?;
        page.validate()?;
        let mut u = url(
            "games",
            &format!(
                "/v2/{}/{id}/games",
                if community { "groups" } else { "users" }
            ),
        );
        page.append(&mut u)?;
        u.query_pairs_mut()
            .append_pair("accessFilter", "Public")
            .append_pair("sortOrder", "Asc");
        let v: List<Game> = self.read(u, g)?;
        let mut out = paged(v, page.limit, |row| row.card(id, community), "universeId")?;
        out["ownerId"] = id.into();
        out["ownerType"] = if community { "Group" } else { "User" }.into();
        out["accessFilter"] = "Public".into();
        out["allCreationsVerified"] = false.into();
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::traffic::tests::FakeClock;
    use crate::transport::HttpResponse;
    struct Fake {
        body: Value,
        status: u16,
        challenge: bool,
        records: Vec<Value>,
    }
    impl Transport for Fake {
        fn send(&mut self, r: Request) -> Result<HttpResponse, AppError> {
            let mut v = r.snapshot();
            v["url"] = r.url().as_str().into();
            self.records.push(v);
            Ok(HttpResponse {
                body: serde_json::to_vec(&self.body).unwrap(),
                status: self.status,
                challenge: self.challenge,
                csrf: None,
                retry_after_seconds: None,
            })
        }
    }
    fn client(body: Value) -> Social<Fake> {
        Social::new(Fake {
            body,
            status: 200,
            challenge: false,
            records: vec![],
        })
    }
    fn page() -> Page {
        Page {
            limit: 10,
            cursor: None,
        }
    }
    #[test]
    fn public_routes_cannot_request_private_roles_games_or_arbitrary_headers() {
        for s in [
            "https://users.roblox.com/v1/users/77",
            "https://users.roblox.com/v1/users/search?keyword=abc&limit=10",
            "https://groups.roblox.com/v1/groups/42",
            "https://groups.roblox.com/v1/groups/42/roles?includePrivate=false",
            "https://groups.roblox.com/v2/users/77/groups/roles?includeLocked=false&includeNotificationPreferences=false&discoveryType=0",
            "https://games.roblox.com/v2/groups/42/games?accessFilter=Public&sortOrder=Asc&limit=10",
        ] {
            let r = Request::social(Url::parse(s).unwrap()).unwrap();
            assert_eq!(r.snapshot()["method"], "GET");
            assert_eq!(r.snapshot()["cookieAttached"], false);
            assert_eq!(r.snapshot()["csrfAttached"], false);
        }
        for s in [
            "https://users.roblox.com/v1/users/authenticated",
            "https://groups.roblox.com/v1/groups/42/audit-log",
            "https://groups.roblox.com/v1/groups/42/users",
            "https://groups.roblox.com/v1/groups/42/roles?includePrivate=true",
            "https://games.roblox.com/v2/users/77/games?accessFilter=Private&sortOrder=Asc",
            "https://games.roblox.com/v2/users/77/games?sortOrder=Asc",
            "https://users.roblox.com/v1/users/77?Cookie=secret",
            "https://users.roblox.com/v1/users/0",
            "https://groups.roblox.com/v1/groups/42/roles?includePrivate=false&includePrivate=true",
        ] {
            assert!(Request::social(Url::parse(s).unwrap()).is_err());
        }
        for s in [
            "https://users.roblox.com/v1/users/77",
            "https://groups.roblox.com/v1/groups/42",
            "https://games.roblox.com/v2/groups/42/games",
        ] {
            assert!(
                Request::new(
                    Url::parse(s).unwrap(),
                    crate::auth::SessionCookie::synthetic().header().unwrap()
                )
                .is_err()
            );
        }
    }
    #[test]
    fn user_profile_is_target_checked_and_drops_email_credentials_and_history() {
        let root = tempfile::tempdir().unwrap();
        let mut g = TrafficGuard::acquire(root.path(), FakeClock::default()).unwrap();
        let mut c = client(
            json!({"id":77,"name":"Synthetic","displayName":"Display","description":"Bio","email":"must-not-echo","previousUsernames":["old"],"cookie":"must-not-echo"}),
        );
        let v = c.user(77, &mut g).unwrap();
        assert_eq!(v["userId"], 77);
        assert_eq!(v["description"], "Bio");
        assert_eq!(v["isBanned"], Value::Null);
        assert!(!v.to_string().contains("must-not-echo"));
        assert!(v.get("previousUsernames").is_none());
        assert_eq!(c.transport.records.len(), 1);
        let mut c = client(json!({"id":88,"name":"Other"}));
        assert_eq!(
            c.user(77, &mut g).unwrap_err().code,
            "social_response_invalid"
        );
    }
    #[test]
    fn community_snapshot_preserves_null_owner_and_counter_semantics() {
        let root = tempfile::tempdir().unwrap();
        let mut g = TrafficGuard::acquire(root.path(), FakeClock::default()).unwrap();
        let mut c = client(
            json!({"id":42,"name":"Community","owner":null,"memberCount":123,"hasSocialModules":true,"communityTier":{"requirements":[{"ownerAge":"must-not-echo"}]}}),
        );
        let v = c.community(42, &mut g).unwrap();
        assert_eq!(v["audienceLabel"], "followers");
        assert_eq!(v["owner"], Value::Null);
        assert_eq!(v["managementPermissionsVerified"], false);
        assert!(!v.to_string().contains("must-not-echo"));
    }
    #[test]
    fn searches_are_one_page_encoded_and_do_not_fetch_each_profile() {
        let root = tempfile::tempdir().unwrap();
        let mut g = TrafficGuard::acquire(root.path(), FakeClock::default()).unwrap();
        let mut c = client(json!({"data":[{"id":77,"name":"Synthetic"}],"nextPageCursor":"next"}));
        let q = Search {
            query: "name & Cookie=not-a-header".into(),
            page: page(),
        };
        let v = c.search(&q, false, &mut g).unwrap();
        assert_eq!(v["hasMore"], true);
        assert_eq!(v["exactIdentityVerified"], false);
        assert_eq!(c.transport.records.len(), 1);
        let u = Url::parse(c.transport.records[0]["url"].as_str().unwrap()).unwrap();
        assert!(u.query_pairs().any(|(k, v)| k == "keyword" && v == q.query));
        assert!(!u.query_pairs().any(|(k, _)| k == "Cookie"));
        assert!(
            Page {
                limit: 11,
                cursor: None
            }
            .validate()
            .is_err()
        );
        assert!(
            Search {
                query: "\n".into(),
                page: page()
            }
            .validate()
            .is_err()
        );
    }
    #[test]
    fn public_roles_accept_guest_zero_but_stop_on_private_data_or_wrong_community() {
        for (group, private, ok) in [(42, false, true), (99, false, false), (42, true, false)] {
            let root = tempfile::tempdir().unwrap();
            let mut g = TrafficGuard::acquire(root.path(), FakeClock::default()).unwrap();
            let mut c = client(
                json!({"groupId":group,"roles":[{"id":0,"name":"Guest","rank":0,"isPrivate":private}]}),
            );
            let v = c.roles(42, 25, &mut g);
            assert_eq!(v.is_ok(), ok);
            if let Ok(v) = v {
                assert_eq!(v["roles"][0]["id"], 0);
                assert_eq!(v["memberRosterFetched"], false);
            }
            assert_eq!(c.transport.records.len(), 1);
        }
    }
    #[test]
    fn memberships_are_public_bounded_and_not_permission_or_notification_data() {
        let root = tempfile::tempdir().unwrap();
        let mut g = TrafficGuard::acquire(root.path(), FakeClock::default()).unwrap();
        let mut c = client(
            json!({"data":[{"group":{"id":42,"name":"Community"},"role":{"id":123,"name":"Member","rank":1},"notificationPreferences":[{"secret":"must-not-echo"}]}]}),
        );
        let v = c.memberships(77, 25, &mut g).unwrap();
        assert_eq!(v["userId"], 77);
        assert_eq!(v["items"][0]["community"]["communityId"], 42);
        assert_eq!(v["allMembershipsVerified"], false);
        assert!(!v.to_string().contains("must-not-echo"));
        assert!(
            c.transport.records[0]["url"]
                .as_str()
                .unwrap()
                .contains("includeNotificationPreferences=false")
        );
    }
    #[test]
    fn public_games_keep_owner_universe_place_ids_distinct_and_never_enrich() {
        for (kind, community) in [(json!(0), false), (json!("Group"), true)] {
            let root = tempfile::tempdir().unwrap();
            let mut g = TrafficGuard::acquire(root.path(), FakeClock::default()).unwrap();
            let mut c = client(
                json!({"data":[{"id":99,"name":"Game","creator":{"id":42,"type":kind,"name":"Creator"},"rootPlace":{"id":88,"type":9}}]}),
            );
            let v = c.games(42, community, &page(), &mut g).unwrap();
            assert_eq!(v["items"][0]["universeId"], 99);
            assert_eq!(v["items"][0]["rootPlaceId"], 88);
            assert_eq!(v["items"][0]["playing"], Value::Null);
            assert_eq!(v["items"][0]["detailFetched"], false);
            assert_eq!(c.transport.records.len(), 1);
            assert!(
                c.transport.records[0]["url"]
                    .as_str()
                    .unwrap()
                    .contains("accessFilter=Public")
            );
        }
    }
    #[test]
    fn refusal_challenge_and_rate_limit_stop_without_retry_or_body_echo() {
        for status in [200, 302, 401, 403, 404, 429, 500] {
            let root = tempfile::tempdir().unwrap();
            let mut g = TrafficGuard::acquire(root.path(), FakeClock::default()).unwrap();
            let mut c = client(json!({"secret":"must-not-echo"}));
            c.transport.status = status;
            c.transport.challenge = status == 200;
            let e = c.user(77, &mut g).unwrap_err();
            assert!(!serde_json::to_string(&e).unwrap().contains("must-not-echo"));
            assert_eq!(c.transport.records.len(), 1);
            assert_eq!(c.transport.records[0]["cookieAttached"], false);
        }
    }
    #[test]
    fn duplicate_ids_malformed_cursors_and_owner_mismatch_fail_closed() {
        let root = tempfile::tempdir().unwrap();
        let mut g = TrafficGuard::acquire(root.path(), FakeClock::default()).unwrap();
        for body in [
            json!({"data":[{"id":77,"name":"A"},{"id":77,"name":"B"}]}),
            json!({"data":[],"nextPageCursor":"\n"}),
        ] {
            let mut c = client(body);
            assert!(
                c.search(
                    &Search {
                        query: "abc".into(),
                        page: page()
                    },
                    false,
                    &mut g
                )
                .is_err()
            );
        }
        let mut c =
            client(json!({"data":[{"id":99,"name":"Game","creator":{"id":123,"type":"User"}}]}));
        assert!(c.games(42, false, &page(), &mut g).is_err());
        let mut c =
            client(json!({"data":[{"id":99,"name":"Game","rootPlace":{"id":88,"type":1}}]}));
        assert!(c.games(42, false, &page(), &mut g).is_err());
    }
    #[test]
    fn offline_links_preserve_namespace_and_never_claim_existence() {
        let a = link(77, false).unwrap();
        let b = link(77, true).unwrap();
        assert_eq!(a["url"], "https://www.roblox.com/users/77/profile");
        assert_eq!(b["url"], "https://www.roblox.com/communities/77");
        assert_eq!(a["existsChecked"], false);
        assert!(link(0, false).is_err());
    }
    #[test]
    fn community_search_and_unpaged_truncation_never_claim_full_memberships() {
        let root = tempfile::tempdir().unwrap();
        let mut g = TrafficGuard::acquire(root.path(), FakeClock::default()).unwrap();
        let mut c = client(
            json!({"data":[{"id":42,"name":"Studio","memberCount":50}],"nextPageCursor":"next"}),
        );
        let v = c
            .search(
                &Search {
                    query: "Studio".into(),
                    page: page(),
                },
                true,
                &mut g,
            )
            .unwrap();
        assert_eq!(v["items"][0]["communityId"], 42);
        assert_eq!(v["items"][0]["audienceLabel"], Value::Null);
        assert_eq!(v["hasMore"], true);
        let mut c = client(
            json!({"data":[{"group":{"id":42,"name":"First"},"role":{"id":1,"name":"Member","rank":1}},
            {"group":{"id":43,"name":"Second"},"role":{"id":2,"name":"Member","rank":1}}]}),
        );
        let v = c.memberships(77, 1, &mut g).unwrap();
        assert_eq!(v["truncatedLocally"], true);
        assert_eq!(v["omittedFromResponse"], 1);
        assert_eq!(v["returnedByApi"], 2);
        assert!(v.get("nextPageCursor").is_none());
        assert_eq!(c.transport.records.len(), 1);
        let mut c = client(json!({"data":[{"group":{"id":42,"name":"Community"},"role":null}]}));
        let v = c.memberships(77, 25, &mut g).unwrap();
        assert_eq!(v["items"][0]["role"], Value::Null);
    }
}
