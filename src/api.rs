use std::collections::HashSet;

use reqwest::Url;
use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::auth::SessionCookie;
use crate::error::AppError;
use crate::traffic::{Clock, TrafficGuard};
use crate::transport::{ReadRequest, Transport};

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
}

pub(crate) struct ListOptions<'a> {
    pub all: bool,
    pub archived: bool,
    pub limit: &'a str,
}

pub(crate) struct Api<T: Transport> {
    pub transport: T,
    cookie: SessionCookie,
}

impl<T: Transport> Api<T> {
    pub fn new(transport: T, cookie: SessionCookie) -> Self {
        Self { transport, cookie }
    }

    fn read<R: DeserializeOwned, C: Clock>(
        &mut self,
        url: Url,
        guard: &mut TrafficGuard<C>,
    ) -> Result<R, AppError> {
        let request = ReadRequest::new(url, self.cookie.header()?)?;
        guard.reserve()?;
        let response = self.transport.get(request)?;
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
            let mut url = Url::parse("https://develop.roblox.com/v1/user/universes").unwrap();
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
            scope: "user-owned",
            archived: options.archived,
            count: experiences.len(),
            has_more: cursor.is_some(),
            experiences,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use serde_json::json;

    use super::*;
    use crate::traffic::tests::FakeClock;
    use crate::transport::HttpResponse;

    #[derive(Default)]
    struct FakeTransport {
        replies: VecDeque<HttpResponse>,
        requests: Vec<Url>,
    }

    impl Transport for FakeTransport {
        fn get(&mut self, request: ReadRequest) -> Result<HttpResponse, AppError> {
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
                body,
            }]);
            let error = api.whoami(&mut guard).unwrap_err();
            assert!(!format!("{error:?}").contains("synthetic-sensitive-body"));
            assert_eq!(api.transport.requests.len(), 1);
        }
    }
}
