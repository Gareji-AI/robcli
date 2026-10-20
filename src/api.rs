use std::collections::HashSet;

use reqwest::Url;
use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::auth::SessionCookie;
use crate::error::AppError;
use crate::traffic::{Clock, TrafficGuard};
use crate::transport::{HttpResponse, Request, Transport};

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct User {
    pub id: u64,
    pub name: String,
    pub display_name: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Universe {
    id: u64,
    name: String,
    root_place_id: Option<u64>,
    is_active: Option<bool>,
    privacy_type: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Page {
    data: Vec<Universe>,
    next_page_cursor: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Experience {
    id: u64,
    name: String,
    root_place_id: Option<u64>,
    is_active: Option<bool>,
    privacy_type: Option<String>,
    dashboard_url: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Collection {
    scope: &'static str,
    archived: bool,
    pub count: usize,
    has_more: bool,
    experiences: Vec<Experience>,
    #[serde(skip_serializing_if = "Option::is_none")]
    group_id: Option<u64>,
}

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct GroupEntry {
    group: GroupInfo,
    role: GroupRole,
}

#[derive(Debug, Deserialize, Serialize)]
struct GroupInfo {
    id: u64,
    name: String,
}

#[derive(Debug, Deserialize, Serialize)]
struct GroupRole {
    id: u64,
    name: String,
    rank: u32,
}

#[derive(Deserialize)]
struct Groups {
    data: Vec<GroupEntry>,
}

pub(crate) struct ListOptions<'a> {
    pub all: bool,
    pub archived: bool,
    pub limit: &'a str,
}

pub(crate) struct Api<T: Transport> {
    pub transport: T,
    cookie: SessionCookie,
    csrf: Option<reqwest::header::HeaderValue>,
    csrf_replayed: bool,
}

impl<T: Transport> Api<T> {
    pub fn into_cookie(self) -> SessionCookie {
        self.cookie
    }

    pub fn new(transport: T, cookie: SessionCookie) -> Self {
        Self {
            transport,
            cookie,
            csrf: None,
            csrf_replayed: false,
        }
    }

    fn read<R: DeserializeOwned, C: Clock>(
        &mut self,
        url: Url,
        guard: &mut TrafficGuard<C>,
    ) -> Result<R, AppError> {
        let request = Request::new(url, self.cookie.header()?)?;
        guard.reserve()?;
        let response = self.transport.send(request)?;
        Self::decode(&response)
    }

    fn decode<R: DeserializeOwned>(response: &HttpResponse) -> Result<R, AppError> {
        if response.challenge {
            return Err(AppError::network(
                "interactive_challenge_required",
                "Roblox requested additional authentication. No challenge bypass was attempted.",
            ));
        }
        if !(200..300).contains(&response.status) {
            let code = match response.status {
                300..=399 => "redirect_refused",
                401 => "session_rejected",
                403 => "permission_or_challenge_required",
                429 => "rate_limited",
                500..=599 => "server_error",
                _ => "api_request_failed",
            };
            let mut error = AppError::network(
                code,
                "Roblox refused the read-only request. No retry or alternate credential was used.",
            );
            error.http_status = Some(response.status);
            error.retry_after_seconds = response.retry_after_seconds;
            return Err(error);
        }
        serde_json::from_slice(&response.body).map_err(|_| {
            AppError::network(
                "invalid_response",
                "The API response did not match the expected JSON shape.",
            )
        })
    }

    pub fn key_post<R: DeserializeOwned, C: Clock>(
        &mut self,
        create: bool,
        body: serde_json::Value,
        allow_csrf: bool,
        guard: &mut TrafficGuard<C>,
    ) -> Result<R, AppError> {
        loop {
            let request = Request::key_post(
                create,
                body.clone(),
                self.cookie.header()?,
                self.csrf.clone(),
            )?;
            guard.reserve()?;
            let response = match self.transport.send(request) {
                Ok(response) => response,
                Err(error) if create => return Err(unknown_creation(error.http_status)),
                Err(error) => return Err(error),
            };
            if response.challenge {
                return Err(AppError::network(
                    "interactive_challenge_required",
                    "Roblox requested additional authentication. No challenge bypass was attempted.",
                ));
            }
            if response.status == 403 && response.csrf.is_some() {
                if allow_csrf && !self.csrf_replayed {
                    self.csrf = response.csrf.clone();
                    self.csrf_replayed = true;
                    continue;
                }
                return Err(AppError::new(
                    "csrf_required",
                    "Roblox refused the request with a CSRF challenge.",
                    "An explicitly requested --allow-csrf-handshake permits one token-validation replay only; no other retry or authentication bypass is allowed.",
                ));
            }
            if create
                && ((300..400).contains(&response.status)
                    || response.status == 408
                    || response.status == 429
                    || response.status >= 500)
            {
                return Err(unknown_creation(Some(response.status)));
            }
            match Self::decode(&response) {
                Ok(value) => return Ok(value),
                Err(_) if create && (200..300).contains(&response.status) => {
                    return Err(unknown_creation(Some(response.status)));
                }
                Err(error) => return Err(error),
            }
        }
    }

    pub fn keys<C: Clock>(
        &mut self,
        group: Option<u64>,
        all: bool,
        allow_csrf: bool,
        guard: &mut TrafficGuard<C>,
    ) -> Result<serde_json::Value, AppError> {
        let mut keys = Vec::new();
        let mut cursor = String::new();
        let mut seen = HashSet::new();
        loop {
            let mut body = serde_json::json!({ "cursor": cursor, "limit": 100, "reverse": false });
            if let Some(group) = group {
                body["groupId"] = group.into();
            }
            let page: KeyPage = self.key_post(false, body, allow_csrf, guard)?;
            for key in page.cloud_auth_info {
                let props = key.cloud_auth_user_configured_properties;
                keys.push(serde_json::json!({ "id": key.id, "ownerId": key.owner_id, "ownerType": key.owner_type,
                    "name": props.as_ref().and_then(|p| p.name.as_deref()), "isEnabled": props.as_ref().and_then(|p| p.is_enabled),
                    "expirationTime": props.as_ref().and_then(|p| p.expiration_time.as_deref()) }));
            }
            cursor = page.next_cursor.unwrap_or_default();
            if !cursor.is_empty() && (!seen.insert(cursor.clone()) || cursor.len() > 4096) {
                return Err(AppError::network(
                    "pagination_stopped",
                    "Key pagination is incomplete or repeated.",
                ));
            }
            if !all || cursor.is_empty() {
                break;
            }
        }
        Ok(serde_json::json!({ "keys": keys, "groupId": group, "hasMore": !cursor.is_empty() }))
    }

    pub fn confirm_account<C: Clock>(
        &mut self,
        expected: u64,
        guard: &mut TrafficGuard<C>,
    ) -> Result<User, AppError> {
        let user = self.whoami(guard)?;
        if user.id != expected {
            return Err(AppError::new(
                "account_confirmation_mismatch",
                "The manually authenticated browser is not the originally verified account.",
                "Stop. No CSV action, owner fallback, or alternate-account retry is attempted.",
            ));
        }
        Ok(user)
    }

    pub fn whoami<C: Clock>(&mut self, guard: &mut TrafficGuard<C>) -> Result<User, AppError> {
        let user: User = self.read(
            Url::parse("https://users.roblox.com/v1/users/authenticated").unwrap(),
            guard,
        )?;
        if user.id == 0 || user.name.is_empty() {
            return Err(AppError::network(
                "invalid_identity",
                "The authenticated-user response was not recognized.",
            ));
        }
        Ok(user)
    }

    pub fn experiences<C: Clock>(
        &mut self,
        options: ListOptions<'_>,
        guard: &mut TrafficGuard<C>,
    ) -> Result<Collection, AppError> {
        self.collection(options, None, guard)
    }

    pub fn groups<C: Clock>(
        &mut self,
        user_id: u64,
        guard: &mut TrafficGuard<C>,
    ) -> Result<Vec<GroupEntry>, AppError> {
        if user_id == 0 || user_id > i64::MAX as u64 {
            return Err(AppError::new(
                "invalid_identity",
                "The signed-in user ID is outside the signed 64-bit contract.",
                "Stop. No group request was sent.",
            ));
        }
        let groups: Groups = self.read(
            Url::parse_with_params(
                &format!("https://groups.roblox.com/v2/users/{user_id}/groups/roles"),
                [
                    ("includeLocked", "false"),
                    ("includeNotificationPreferences", "false"),
                    ("discoveryType", "0"),
                ],
            )
            .unwrap(),
            guard,
        )?;
        Ok(groups.data)
    }

    pub fn verify_experience_owner<C: Clock>(
        &mut self,
        universe: u64,
        user: u64,
        group: Option<u64>,
        guard: &mut TrafficGuard<C>,
    ) -> Result<(), AppError> {
        let info: serde_json::Value = self.read(
            Url::parse(&format!(
                "https://develop.roblox.com/v1/universes/{universe}"
            ))
            .unwrap(),
            guard,
        )?;
        let (kind, owner) = group.map(|id| ("Group", id)).unwrap_or(("User", user));
        if info["id"].as_u64() != Some(universe)
            || info["creatorType"].as_str() != Some(kind)
            || info["creatorTargetId"].as_u64() != Some(owner)
        {
            return Err(AppError::new(
                "experience_owner_mismatch",
                "The experience does not match the explicitly selected owner.",
                "Select the correct group and experience. No settings query, chart action or owner fallback was attempted.",
            ));
        }
        Ok(())
    }

    pub fn settings<C: Clock>(
        &mut self,
        universe: u64,
        user: u64,
        group: Option<u64>,
        guard: &mut TrafficGuard<C>,
    ) -> Result<serde_json::Value, AppError> {
        self.verify_experience_owner(universe, user, group, guard)?;
        let raw: serde_json::Value = self.read(
            Url::parse(&format!(
                "https://develop.roblox.com/v1/universes/{universe}/configuration"
            ))
            .unwrap(),
            guard,
        )?;
        if raw["id"].as_u64() != Some(universe) {
            return Err(AppError::network(
                "configuration_target_mismatch",
                "The configuration did not identify the selected experience.",
            ));
        }
        // Positive projection from the published UniverseSettingsResponse schema.
        // Unknown future fields, including any unexpected secret field, are not echoed.
        const FIELDS: &[&str] = &[
            "id",
            "name",
            "promotionalText",
            "allowPrivateServers",
            "privateServerPrice",
            "isMeshTextureApiAccessAllowed",
            "isRewardedOnDemandAdsAllowed",
            "universeAvatarType",
            "universeScaleType",
            "universeAnimationType",
            "universeCollisionType",
            "universeBodyType",
            "universeJointPositioningType",
            "isArchived",
            "isFriendsOnly",
            "genre",
            "playableDevices",
            "isForSale",
            "price",
            "isStudioAccessToApisAllowed",
            "privacyType",
            "isForSaleInFiat",
            "fiatBasePriceId",
            "fiatModerationStatus",
            "audiences",
            "demoModeEnabled",
            "demoModeChangeableAfter",
        ];
        let settings: serde_json::Map<String, serde_json::Value> = FIELDS
            .iter()
            .filter_map(|key| {
                raw.get(*key)
                    .map(|value| ((*key).to_owned(), value.clone()))
            })
            .collect();
        let mut output = crate::dashboard::links(universe, group);
        output["settings"] = serde_json::Value::Object(settings);
        Ok(output)
    }

    pub fn group_experiences<C: Clock>(
        &mut self,
        group: u64,
        options: ListOptions<'_>,
        guard: &mut TrafficGuard<C>,
    ) -> Result<Collection, AppError> {
        self.collection(options, Some(group), guard)
    }

    fn collection<C: Clock>(
        &mut self,
        options: ListOptions<'_>,
        group: Option<u64>,
        guard: &mut TrafficGuard<C>,
    ) -> Result<Collection, AppError> {
        if !["10", "25", "50", "100"].contains(&options.limit) {
            return Err(AppError::new(
                "invalid_limit",
                "Page size must be 10, 25, 50, or 100.",
                "Use a supported page size. This does not raise the request budget.",
            ));
        }
        let mut experiences = Vec::new();
        let mut cursor: Option<String> = None;
        let mut seen = HashSet::new();
        loop {
            let mut url = match group {
                Some(id) => Url::parse(&format!(
                    "https://develop.roblox.com/v1/groups/{id}/universes"
                ))
                .unwrap(),
                None => Url::parse("https://develop.roblox.com/v1/user/universes").unwrap(),
            };
            {
                let mut query = url.query_pairs_mut();
                query.append_pair(
                    "isArchived",
                    if options.archived { "true" } else { "false" },
                );
                query.append_pair("limit", options.limit);
                query.append_pair("sortOrder", "Desc");
                if let Some(cursor) = cursor.as_deref() {
                    query.append_pair("cursor", cursor);
                }
            }
            let page: Page = self.read(url, guard)?;
            for universe in page.data {
                if universe.id == 0 || universe.name.is_empty() {
                    return Err(AppError::network(
                        "invalid_response",
                        "An experience entry was not recognized.",
                    ));
                }
                experiences.push(Experience {
                    dashboard_url: format!(
                        "https://create.roblox.com/dashboard/creations/experiences/{}/overview",
                        universe.id
                    ),
                    id: universe.id,
                    name: universe.name,
                    root_place_id: universe.root_place_id,
                    is_active: universe.is_active,
                    privacy_type: universe.privacy_type,
                });
            }
            cursor = page.next_page_cursor.filter(|value| !value.is_empty());
            if let Some(next) = cursor.as_ref()
                && (next.len() > 4096 || !seen.insert(next.clone()))
            {
                return Err(AppError::network(
                    "pagination_stopped",
                    "Pagination repeated or returned an oversized cursor. The listing is incomplete.",
                ));
            }
            if !options.all || cursor.is_none() {
                break;
            }
        }
        Ok(Collection {
            scope: if group.is_some() {
                "group-owned"
            } else {
                "user-owned"
            },
            archived: options.archived,
            count: experiences.len(),
            has_more: cursor.is_some(),
            experiences,
            group_id: group,
        })
    }
}

fn unknown_creation(status: Option<u16>) -> AppError {
    let mut error = AppError::network(
        "creation_outcome_unknown",
        "Key creation may have succeeded. No request was retried.",
    );
    error.http_status = status;
    error.hint = "Inspect the Dashboard's key list before doing anything else. Do not resubmit create, rotate credentials, or delete a reserved secret file to force another attempt.";
    error
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct KeyPage {
    cloud_auth_info: Vec<KeyMeta>,
    next_cursor: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct KeyMeta {
    id: String,
    owner_id: Option<u64>,
    owner_type: Option<String>,
    cloud_auth_user_configured_properties: Option<KeyProperties>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct KeyProperties {
    name: Option<String>,
    is_enabled: Option<bool>,
    expiration_time: Option<String>,
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use serde_json::json;

    use super::*;
    use crate::traffic::tests::FakeClock;

    #[derive(Default)]
    struct FakeTransport {
        replies: VecDeque<HttpResponse>,
        requests: Vec<Url>,
        records: Vec<serde_json::Value>,
    }

    impl Transport for FakeTransport {
        fn send(&mut self, request: Request) -> Result<HttpResponse, AppError> {
            self.records.push(request.snapshot());
            self.requests.push(request.url().clone());
            Ok(self
                .replies
                .pop_front()
                .expect("unexpected request in offline test"))
        }
    }

    fn response(value: serde_json::Value) -> HttpResponse {
        HttpResponse {
            status: 200,
            retry_after_seconds: None,
            csrf: None,
            challenge: false,
            body: serde_json::to_vec(&value).unwrap(),
        }
    }

    fn page(id: u64, next: Option<&str>) -> HttpResponse {
        response(
            json!({ "data": [{ "id": id, "name": "synthetic experience", "rootPlaceId": 2, "isActive": false, "privacyType": "Private" }], "nextPageCursor": next }),
        )
    }

    fn api(replies: Vec<HttpResponse>) -> Api<FakeTransport> {
        Api::new(
            FakeTransport {
                replies: replies.into(),
                ..Default::default()
            },
            SessionCookie::synthetic(),
        )
    }

    #[test]
    fn identity_uses_only_the_fixed_authenticated_user_endpoint() {
        let root = tempfile::tempdir().unwrap();
        let mut guard = TrafficGuard::acquire(root.path(), FakeClock::default()).unwrap();
        let mut api = api(vec![response(
            json!({ "id": 1, "name": "mock", "displayName": "Mock" }),
        )]);
        assert_eq!(api.whoami(&mut guard).unwrap().id, 1);
        assert_eq!(
            api.transport.requests[0].as_str(),
            "https://users.roblox.com/v1/users/authenticated"
        );
    }

    #[test]
    fn groups_list_sends_the_required_query_and_rejects_other_shapes() {
        let root = tempfile::tempdir().unwrap();
        let mut guard = TrafficGuard::acquire(root.path(), FakeClock::default()).unwrap();
        let mut api = api(vec![response(json!({ "data": [] }))]);
        assert!(api.groups(77, &mut guard).unwrap().is_empty());
        let pairs: std::collections::HashMap<_, _> =
            api.transport.requests[0].query_pairs().collect();
        assert_eq!(
            pairs.get("includeLocked").map(|v| v.as_ref()),
            Some("false")
        );
        assert_eq!(
            pairs
                .get("includeNotificationPreferences")
                .map(|v| v.as_ref()),
            Some("false")
        );
        assert_eq!(pairs.get("discoveryType").map(|v| v.as_ref()), Some("0"));
        assert_eq!(api.transport.requests.len(), 1);
        let header = SessionCookie::synthetic().header().unwrap();
        let base = "https://groups.roblox.com/v2/users/77/groups/roles";
        let good = format!(
            "{base}?includeLocked=false&includeNotificationPreferences=false&discoveryType=0"
        );
        assert!(Request::new(Url::parse(&good).unwrap(), header.clone()).is_ok());
        for url in [
            base.to_owned(),
            format!("{good}&extra=1"),
            format!("{base}?includeLocked=false&includeLocked=false&includeNotificationPreferences=false&discoveryType=0"),
            format!("{base}?includeLocked=true&includeNotificationPreferences=false&discoveryType=0"),
            format!("{base}?includeLocked=false&includeNotificationPreferences=true&discoveryType=0"),
            format!("{base}?includeLocked=false&includeNotificationPreferences=false&discoveryType=1"),
            "https://groups.roblox.com/v2/users/9223372036854775808/groups/roles?includeLocked=false&includeNotificationPreferences=false&discoveryType=0".to_owned(),
        ] {
            assert!(
                Request::new(Url::parse(&url).unwrap(), header.clone()).is_err(),
                "{url}"
            );
        }
    }

    #[test]
    fn reauthenticated_browser_must_match_the_original_account() {
        for id in [1, 99] {
            let root = tempfile::tempdir().unwrap();
            let mut guard = TrafficGuard::acquire(root.path(), FakeClock::default()).unwrap();
            let mut api = api(vec![response(json!({ "id": id, "name": "synthetic" }))]);
            match api.confirm_account(1, &mut guard) {
                Ok(user) => assert_eq!(user.id, 1),
                Err(error) => {
                    assert_eq!(id, 99);
                    assert_eq!(error.code, "account_confirmation_mismatch");
                }
            }
            assert_eq!(api.transport.requests.len(), 1);
        }
    }

    #[test]
    fn canonical_browser_url_still_requires_a_server_verified_group_owner() {
        for owner in [42, 99] {
            let root = tempfile::tempdir().unwrap();
            let mut guard = TrafficGuard::acquire(root.path(), FakeClock::default()).unwrap();
            let mut api = api(vec![response(
                json!({ "id": 77, "creatorType": "Group", "creatorTargetId": owner }),
            )]);
            let result = api.verify_experience_owner(77, 1, Some(42), &mut guard);
            assert_eq!(result.is_ok(), owner == 42);
            assert_eq!(api.transport.requests.len(), 1);
            assert_eq!(api.transport.requests[0].path(), "/v1/universes/77");
        }
    }

    #[test]
    fn group_selection_uses_the_verified_group_route_and_explicit_scope() {
        let root = tempfile::tempdir().unwrap();
        let mut guard = TrafficGuard::acquire(root.path(), FakeClock::default()).unwrap();
        let mut api = api(vec![page(1, None)]);
        let output = api
            .group_experiences(
                42,
                ListOptions {
                    all: false,
                    archived: true,
                    limit: "25",
                },
                &mut guard,
            )
            .unwrap();
        assert_eq!(output.scope, "group-owned");
        assert_eq!(output.group_id, Some(42));
        assert_eq!(api.transport.requests[0].path(), "/v1/groups/42/universes");
        assert!(
            api.transport.requests[0]
                .query_pairs()
                .any(|(k, v)| k == "isArchived" && v == "true")
        );
    }

    #[test]
    fn settings_are_read_only_after_the_explicit_owner_matches() {
        let root = tempfile::tempdir().unwrap();
        let mut guard = TrafficGuard::acquire(root.path(), FakeClock::default()).unwrap();
        let mut api = api(vec![
            response(
                json!({ "id": 77, "name": "Mock", "creatorType": "Group", "creatorTargetId": 42 }),
            ),
            response(
                json!({ "id": 77, "name": "Mock", "privateServerPrice": 0, "apikeySecret": "must-not-appear" }),
            ),
        ]);
        let output = api.settings(77, 1, Some(42), &mut guard).unwrap();
        assert_eq!(
            api.transport.requests[1].path(),
            "/v1/universes/77/configuration"
        );
        assert_eq!(output["settings"]["privateServerPrice"], 0);
        assert!(output["settings"].get("apikeySecret").is_none());
        assert_eq!(
            output["settingsUrl"],
            "https://create.roblox.com/dashboard/creations/experiences/77/configure?groupId=42"
        );
    }

    #[test]
    fn key_listing_projects_metadata_without_secret_or_preview() {
        let root = tempfile::tempdir().unwrap();
        let mut guard = TrafficGuard::acquire(root.path(), FakeClock::default()).unwrap();
        let mut api = api(vec![response(json!({
            "cloudAuthInfo": [{ "id": "key-id", "apikeySecret": "never-output", "apikeySecretPreview": "never-output", "cloudAuthUserConfiguredProperties": { "name": "analytics-read", "isEnabled": true } }],
            "nextCursor": null
        }))]);
        let output = api.keys(Some(42), false, false, &mut guard).unwrap();
        assert_eq!(output["keys"][0]["id"], "key-id");
        assert!(!output.to_string().contains("never-output"));
        assert_eq!(
            api.transport.requests[0].path(),
            "/cloud-authentication/v1/apiKeys"
        );
        assert_eq!(api.transport.records[0]["method"], "POST");
        assert_eq!(
            api.transport.records[0]["body"],
            json!({ "cursor": "", "limit": 100, "reverse": false, "groupId": 42 })
        );
    }

    fn csrf() -> HttpResponse {
        let mut value = reqwest::header::HeaderValue::from_static("synthetic-csrf");
        value.set_sensitive(true);
        HttpResponse {
            status: 403,
            retry_after_seconds: None,
            csrf: Some(value),
            challenge: false,
            body: Vec::new(),
        }
    }

    #[test]
    fn csrf_exchange_is_explicit_and_capped_at_one_replay() {
        let root = tempfile::tempdir().unwrap();
        let mut guard = TrafficGuard::acquire(root.path(), FakeClock::default()).unwrap();
        let mut refused = api(vec![csrf()]);
        assert_eq!(
            refused
                .keys(None, false, false, &mut guard)
                .unwrap_err()
                .code,
            "csrf_required"
        );
        assert_eq!(refused.transport.requests.len(), 1);
        drop(guard);

        let root = tempfile::tempdir().unwrap();
        let mut guard = TrafficGuard::acquire(root.path(), FakeClock::default()).unwrap();
        let mut allowed = api(vec![
            csrf(),
            response(json!({ "cloudAuthInfo": [], "nextCursor": null })),
        ]);
        allowed.keys(None, false, true, &mut guard).unwrap();
        assert_eq!(allowed.transport.requests.len(), 2);
        assert_eq!(allowed.transport.records[0]["csrfAttached"], false);
        assert_eq!(allowed.transport.records[1]["csrfAttached"], true);
        assert_eq!(
            allowed.transport.records[0]["body"],
            allowed.transport.records[1]["body"]
        );

        let root = tempfile::tempdir().unwrap();
        let mut guard = TrafficGuard::acquire(root.path(), FakeClock::default()).unwrap();
        let mut twice = api(vec![csrf(), csrf()]);
        assert_eq!(
            twice.keys(None, false, true, &mut guard).unwrap_err().code,
            "csrf_required"
        );
        assert_eq!(twice.transport.requests.len(), 2);
    }

    #[test]
    fn auth_challenges_are_not_replayed_and_uncertain_creation_is_not_retried() {
        for status in [302, 408, 429, 500, 503] {
            let root = tempfile::tempdir().unwrap();
            let mut guard = TrafficGuard::acquire(root.path(), FakeClock::default()).unwrap();
            let mut reply = response(json!({}));
            reply.status = status;
            let mut api = api(vec![reply]);
            let result: Result<crate::keys::Created, _> =
                api.key_post(true, json!({}), true, &mut guard);
            assert_eq!(result.err().unwrap().code, "creation_outcome_unknown");
            assert_eq!(api.transport.requests.len(), 1);
        }
        let root = tempfile::tempdir().unwrap();
        let mut guard = TrafficGuard::acquire(root.path(), FakeClock::default()).unwrap();
        let mut challenge = csrf();
        challenge.challenge = true;
        let mut api = api(vec![challenge]);
        assert_eq!(
            api.keys(None, false, true, &mut guard).unwrap_err().code,
            "interactive_challenge_required"
        );
        assert_eq!(api.transport.requests.len(), 1);
    }

    #[test]
    fn wrong_group_stops_before_reading_settings() {
        let root = tempfile::tempdir().unwrap();
        let mut guard = TrafficGuard::acquire(root.path(), FakeClock::default()).unwrap();
        let mut api = api(vec![response(
            json!({ "id": 77, "creatorType": "Group", "creatorTargetId": 99 }),
        )]);
        assert_eq!(
            api.settings(77, 1, Some(42), &mut guard).unwrap_err().code,
            "experience_owner_mismatch"
        );
        assert_eq!(api.transport.requests.len(), 1);
    }

    #[test]
    fn pages_with_encoded_cursors_and_preserves_archive_filter() {
        let root = tempfile::tempdir().unwrap();
        let mut guard = TrafficGuard::acquire(root.path(), FakeClock::default()).unwrap();
        let cursor = "x&redirect=https://example.com/?secret=synthetic";
        let mut api = api(vec![page(1, Some(cursor)), page(2, None)]);
        let collection = api
            .experiences(
                ListOptions {
                    all: true,
                    archived: true,
                    limit: "25",
                },
                &mut guard,
            )
            .unwrap();
        assert_eq!(collection.count, 2);
        assert!(!collection.has_more);
        assert!(collection.archived);
        assert_eq!(
            api.transport.requests[1].host_str(),
            Some("develop.roblox.com")
        );
        let query: std::collections::HashMap<_, _> =
            api.transport.requests[1].query_pairs().collect();
        assert_eq!(
            query.get("cursor").map(|value| value.as_ref()),
            Some(cursor)
        );
        assert_eq!(
            query.get("isArchived").map(|value| value.as_ref()),
            Some("true")
        );
    }

    #[test]
    fn one_page_does_not_claim_completion_when_more_results_exist() {
        let root = tempfile::tempdir().unwrap();
        let mut guard = TrafficGuard::acquire(root.path(), FakeClock::default()).unwrap();
        let mut api = api(vec![page(1, Some("next"))]);
        let collection = api
            .experiences(
                ListOptions {
                    all: false,
                    archived: false,
                    limit: "100",
                },
                &mut guard,
            )
            .unwrap();
        assert!(collection.has_more);
        assert_eq!(api.transport.requests.len(), 1);
    }

    #[test]
    fn repeated_cursors_stop_without_an_extra_request() {
        let root = tempfile::tempdir().unwrap();
        let mut guard = TrafficGuard::acquire(root.path(), FakeClock::default()).unwrap();
        let mut api = api(vec![page(1, Some("same")), page(2, Some("same"))]);
        let error = api
            .experiences(
                ListOptions {
                    all: true,
                    archived: false,
                    limit: "100",
                },
                &mut guard,
            )
            .unwrap_err();
        assert_eq!(error.code, "pagination_stopped");
        assert_eq!(api.transport.requests.len(), 2);
    }

    #[test]
    fn endless_pages_cannot_exceed_five_requests_or_claim_success() {
        let root = tempfile::tempdir().unwrap();
        let mut guard = TrafficGuard::acquire(root.path(), FakeClock::default()).unwrap();
        let replies = (1..=5)
            .map(|id| page(id, Some(&format!("cursor-{id}"))))
            .collect();
        let mut api = api(replies);
        let error = api
            .experiences(
                ListOptions {
                    all: true,
                    archived: false,
                    limit: "100",
                },
                &mut guard,
            )
            .unwrap_err();
        assert_eq!(error.code, "traffic_budget_exhausted");
        assert_eq!(api.transport.requests.len(), 5);
    }

    #[test]
    fn error_statuses_stop_once_and_never_echo_server_bodies() {
        for status in [302, 401, 403, 429, 500, 503] {
            let root = tempfile::tempdir().unwrap();
            let mut guard = TrafficGuard::acquire(root.path(), FakeClock::default()).unwrap();
            let mut api = api(vec![HttpResponse {
                status,
                retry_after_seconds: Some(60),
                csrf: None,
                challenge: false,
                body: b"sensitive-server-content".to_vec(),
            }]);
            let error = api.whoami(&mut guard).unwrap_err();
            assert_eq!(error.http_status, Some(status));
            assert_eq!(api.transport.requests.len(), 1);
            assert!(
                !serde_json::to_string(&error)
                    .unwrap()
                    .contains("sensitive-server-content")
            );
            let state: serde_json::Value =
                serde_json::from_slice(&std::fs::read(root.path().join("traffic.json")).unwrap())
                    .unwrap();
            assert_eq!(state["starts"].as_array().unwrap().len(), 1);
        }
    }

    #[test]
    fn malformed_json_and_invalid_identity_fail_without_leaking_body() {
        for body in [
            b"synthetic-sensitive-body".to_vec(),
            br#"{"id":0,"name":""}"#.to_vec(),
        ] {
            let root = tempfile::tempdir().unwrap();
            let mut guard = TrafficGuard::acquire(root.path(), FakeClock::default()).unwrap();
            let mut api = api(vec![HttpResponse {
                status: 200,
                retry_after_seconds: None,
                csrf: None,
                challenge: false,
                body,
            }]);
            let error = api.whoami(&mut guard).unwrap_err();
            assert!(!format!("{error:?}").contains("synthetic-sensitive-body"));
            assert_eq!(api.transport.requests.len(), 1);
        }
    }
}
