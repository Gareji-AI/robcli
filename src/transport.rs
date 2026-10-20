use std::io::Read;
use std::time::Duration;

use reqwest::blocking::Client;
use reqwest::header::{ACCEPT, CONTENT_TYPE, COOKIE, HeaderValue, RETRY_AFTER};
use reqwest::{Method, Url};
use zeroize::Zeroize;

use crate::error::AppError;

const MAX_BODY_BYTES: u64 = 2 * 1024 * 1024;

pub(crate) struct Request {
    url: Url,
    cookie: Option<HeaderValue>,
    method: Method,
    body: Option<serde_json::Value>,
    csrf: Option<HeaderValue>,
}

fn groups_roles_request(url: &Url) -> bool {
    let Some(id) = url
        .path()
        .strip_prefix("/v2/users/")
        .and_then(|path| path.strip_suffix("/groups/roles"))
        .and_then(|id| id.parse::<u64>().ok())
    else {
        return false;
    };
    if id == 0 || id > i64::MAX as u64 {
        return false;
    }
    let pairs: Vec<_> = url.query_pairs().collect();
    if pairs.len() != 3 {
        return false;
    }
    let mut seen = [false; 3];
    for (key, value) in &pairs {
        let index = match key.as_ref() {
            "includeLocked" if value == "false" => 0,
            "includeNotificationPreferences" if value == "false" => 1,
            "discoveryType" if value == "0" => 2,
            _ => return false,
        };
        if seen[index] {
            return false;
        }
        seen[index] = true;
    }
    seen == [true, true, true]
}

impl Request {
    pub fn new(url: Url, cookie: HeaderValue) -> Result<Self, AppError> {
        Self::validate(&url)?;
        let path = url.path();
        let numeric = |prefix: &str, suffix: &str| {
            path.strip_prefix(prefix)
                .and_then(|value| value.strip_suffix(suffix))
                .and_then(|value| value.parse::<u64>().ok())
                .is_some_and(|id| id > 0)
        };
        let allowed = match url.host_str() {
            Some("users.roblox.com") => path == "/v1/users/authenticated" && url.query().is_none(),
            Some("groups.roblox.com") => groups_roles_request(&url),
            Some("develop.roblox.com") => {
                path == "/v1/user/universes"
                    || numeric("/v1/groups/", "/universes")
                    || numeric("/v1/universes/", "")
                    || numeric("/v1/universes/", "/configuration")
            }
            _ => false,
        };
        if !allowed {
            return Err(blocked());
        }
        Ok(Self {
            url,
            cookie: Some(cookie),
            method: Method::GET,
            body: None,
            csrf: None,
        })
    }

    pub fn key_post(
        create: bool,
        body: serde_json::Value,
        cookie: HeaderValue,
        csrf: Option<HeaderValue>,
    ) -> Result<Self, AppError> {
        let path = if create { "apiKey" } else { "apiKeys" };
        let url = Url::parse(&format!(
            "https://apis.roblox.com/cloud-authentication/v1/{path}"
        ))
        .unwrap();
        Self::validate(&url)?;
        Ok(Self {
            url,
            cookie: Some(cookie),
            method: Method::POST,
            body: Some(body),
            csrf,
        })
    }

    /// Public Creator Store reads deliberately have no authentication headers.
    /// This constructor cannot address Dashboard/private APIs or make a write.
    pub fn creator_store(url: Url) -> Result<Self, AppError> {
        Self::validate(&url)?;
        let search = url.path() == "/toolbox-service/v2/assets:search";
        let detail = url
            .path()
            .strip_prefix("/toolbox-service/v2/assets/")
            .and_then(|id| id.parse::<u64>().ok())
            .is_some_and(|id| id > 0 && id <= i64::MAX as u64);
        const KEYS: &[&str] = &[
            "searchCategoryType",
            "query",
            "maxPageSize",
            "searchView",
            "sortCategory",
            "sortDirection",
            "includeOnlyVerifiedCreators",
            "audioTypes",
            "audioMinDurationSeconds",
            "audioMaxDurationSeconds",
            "audioArtist",
            "audioAlbum",
            "userId",
            "groupId",
            "maxPriceCents",
            "pageToken",
        ];
        if url.host_str() != Some("apis.roblox.com")
            || (!search && !detail)
            || (detail && url.query().is_some())
            || url
                .query_pairs()
                .any(|(key, _)| !KEYS.contains(&key.as_ref()))
        {
            return Err(blocked());
        }
        Ok(Self {
            url,
            cookie: None,
            method: Method::GET,
            body: None,
            csrf: None,
        })
    }

    pub fn avatar_search(url: Url) -> Result<Self, AppError> {
        Self::validate(&url)?;
        const KEYS: &[&str] = &[
            "AssetTypeIds",
            "BundleTypeIds",
            "Keyword",
            "limit",
            "cursor",
            "SortType",
            "SortAggregation",
            "CreatorType",
            "CreatorTargetId",
            "MinPrice",
            "MaxPrice",
            "IncludeNotForSale",
        ];
        if url.host_str() != Some("catalog.roblox.com")
            || url.path() != "/v2/search/items/details"
            || url
                .query_pairs()
                .any(|(key, _)| !KEYS.contains(&key.as_ref()))
        {
            return Err(blocked());
        }
        Ok(Self {
            url,
            cookie: None,
            method: Method::GET,
            body: None,
            csrf: None,
        })
    }

    /// One anonymous bundle-details GET. Not the removed catalog metadata POST.
    pub fn avatar_bundle(url: Url) -> Result<Self, AppError> {
        Self::validate(&url)?;
        let id = url
            .path()
            .strip_prefix("/v1/bundles/")
            .and_then(|path| path.strip_suffix("/details"))
            .and_then(|id| id.parse::<u64>().ok());
        if url.host_str() != Some("catalog.roblox.com")
            || url.query().is_some()
            || !id.is_some_and(|id| id > 0 && id <= i64::MAX as u64)
        {
            return Err(blocked());
        }
        Ok(Self {
            url,
            cookie: None,
            method: Method::GET,
            body: None,
            csrf: None,
        })
    }

    /// Anonymous game icon or thumbnail metadata. Image bytes are not requested.
    pub fn game_thumbnail(url: Url) -> Result<Self, AppError> {
        Self::validate(&url)?;
        let icon = url.path() == "/v1/games/icons";
        let thumbs = url.path() == "/v1/games/multiget/thumbnails";
        let keys: &[&str] = if icon {
            &["universeIds", "size", "format", "isCircular"]
        } else if thumbs {
            &["universeIds", "count", "size", "format", "isCircular"]
        } else {
            return Err(blocked());
        };
        if url.host_str() != Some("thumbnails.roblox.com") || !fixed_query(&url, keys) {
            return Err(blocked());
        }
        let pairs: Vec<_> = url.query_pairs().collect();
        let size = pairs
            .iter()
            .find(|(key, _)| key == "size")
            .map(|(_, value)| value.as_ref());
        let format = pairs
            .iter()
            .find(|(key, _)| key == "format")
            .map(|(_, value)| value.as_ref());
        let circular = pairs
            .iter()
            .find(|(key, _)| key == "isCircular")
            .map(|(_, value)| value.as_ref());
        let universe = pairs
            .iter()
            .find(|(key, _)| key == "universeIds")
            .and_then(|(_, value)| value.parse::<u64>().ok());
        let sizes: &[&str] = if icon {
            &["50x50", "128x128", "150x150", "256x256", "512x512"]
        } else {
            &["768x432", "576x324", "480x270", "384x216", "256x144"]
        };
        let count_ok = if thumbs {
            pairs
                .iter()
                .find(|(key, _)| key == "count")
                .and_then(|(_, value)| value.parse::<u32>().ok())
                .is_some_and(|count| (1..=10).contains(&count))
        } else {
            true
        };
        if circular != Some("false")
            || !matches!(format, Some("Png" | "Jpeg"))
            || !size.is_some_and(|value| sizes.contains(&value))
            || !universe.is_some_and(|id| id > 0 && id <= i64::MAX as u64)
            || pairs.iter().filter(|(key, _)| key == "universeIds").count() != 1
            || !count_ok
        {
            return Err(blocked());
        }
        Ok(Self {
            url,
            cookie: None,
            method: Method::GET,
            body: None,
            csrf: None,
        })
    }

    /// One anonymous public badge page. No award, inventory or badge-creation route.
    pub fn badges(url: Url) -> Result<Self, AppError> {
        Self::validate(&url)?;
        let id = url
            .path()
            .strip_prefix("/v1/universes/")
            .and_then(|path| path.strip_suffix("/badges"))
            .and_then(|id| id.parse::<u64>().ok());
        if url.host_str() != Some("badges.roblox.com")
            || !id.is_some_and(|id| id > 0 && id <= i64::MAX as u64)
            || !fixed_query(&url, &["limit", "sortOrder", "cursor"])
        {
            return Err(blocked());
        }
        let pairs: Vec<_> = url.query_pairs().collect();
        let limit = pairs
            .iter()
            .find(|(key, _)| key == "limit")
            .map(|(_, value)| value.as_ref());
        let sort = pairs
            .iter()
            .find(|(key, _)| key == "sortOrder")
            .map(|(_, value)| value.as_ref());
        let cursor_ok = match pairs.iter().find(|(key, _)| key == "cursor") {
            None => true,
            Some((_, value)) => {
                !value.is_empty() && value.len() <= 4096 && !value.chars().any(|c| c.is_control())
            }
        };
        if !matches!(limit, Some("10" | "25" | "50" | "100"))
            || !matches!(sort, Some("Asc" | "Desc"))
            || !cursor_ok
        {
            return Err(blocked());
        }
        Ok(Self {
            url,
            cookie: None,
            method: Method::GET,
            body: None,
            csrf: None,
        })
    }

    /// Anonymous discovery only. No creator-management, purchase or asset routes.
    pub fn discovery(url: Url) -> Result<Self, AppError> {
        Self::validate(&url)?;
        let numeric = |prefix: &str, suffix: &str| {
            url.path()
                .strip_prefix(prefix)
                .and_then(|s| s.strip_suffix(suffix))
                .and_then(|s| s.parse::<u64>().ok())
                .is_some_and(|id| id > 0 && id <= i64::MAX as u64)
        };
        let keys: &[&str] = match (url.host_str(), url.path()) {
            (Some("games.roblox.com"), "/v1/games" | "/v1/games/votes") => &["universeIds"],
            (Some("apis.roblox.com"), "/explore-api/v1/get-sorts") => {
                &["sessionId", "device", "country", "sortsPageToken"]
            }
            (Some("apis.roblox.com"), "/explore-api/v1/get-sort-content") => {
                &["sessionId", "device", "country", "sortId", "pageToken"]
            }
            (Some("apis.roblox.com"), "/music-discovery/v1/top-songs") => &["limit", "pageToken"],
            (Some("apis.roblox.com"), "/music-discovery/v1/experience-songs") => {
                &["universeId", "limit"]
            }
            (Some("apis.roblox.com"), _) if numeric("/universes/v1/places/", "/universe") => &[],
            (Some("apis.roblox.com"), _)
                if numeric("/game-passes/v1/universes/", "/game-passes") =>
            {
                &["passView", "pageSize", "pageToken"]
            }
            (Some("apis.roblox.com"), _)
                if numeric("/developer-products/v2/universes/", "/developerproducts") =>
            {
                &["limit", "cursor"]
            }
            _ => return Err(blocked()),
        };
        if (keys.is_empty() && url.query().is_some())
            || url
                .query_pairs()
                .any(|(key, _)| !keys.contains(&key.as_ref()))
        {
            return Err(blocked());
        }
        Ok(Self {
            url,
            cookie: None,
            method: Method::GET,
            body: None,
            csrf: None,
        })
    }

    /// Public profiles/communities only; fixed public filters cannot request private roles/games.
    pub fn social(url: Url) -> Result<Self, AppError> {
        Self::validate(&url)?;
        let numeric = |prefix: &str, suffix: &str| {
            url.path()
                .strip_prefix(prefix)
                .and_then(|s| s.strip_suffix(suffix))
                .and_then(|s| s.parse::<u64>().ok())
                .is_some_and(|n| n > 0 && n <= i64::MAX as u64)
        };
        let keys: &[&str] = match (url.host_str(), url.path()) {
            (Some("users.roblox.com"), "/v1/users/search") => &["keyword", "limit", "cursor"],
            (Some("users.roblox.com"), _) if numeric("/v1/users/", "") => &[],
            (Some("groups.roblox.com"), "/v1/groups/search") => &["keyword", "limit", "cursor"],
            (Some("groups.roblox.com"), _) if numeric("/v1/groups/", "") => &[],
            (Some("groups.roblox.com"), _) if numeric("/v1/groups/", "/roles") => {
                &["includePrivate"]
            }
            (Some("groups.roblox.com"), _) if numeric("/v2/users/", "/groups/roles") => &[
                "includeLocked",
                "includeNotificationPreferences",
                "discoveryType",
            ],
            (Some("games.roblox.com"), _)
                if numeric("/v2/users/", "/games") || numeric("/v2/groups/", "/games") =>
            {
                &["accessFilter", "limit", "cursor", "sortOrder"]
            }
            _ => return Err(blocked()),
        };
        let pairs: Vec<_> = url.query_pairs().collect();
        if (keys.is_empty() && url.query().is_some())
            || pairs.iter().any(|(k, _)| !keys.contains(&k.as_ref()))
            || pairs
                .iter()
                .enumerate()
                .any(|(i, (k, _))| pairs[..i].iter().any(|(other, _)| other == k))
        {
            return Err(blocked());
        }
        for (key, expected) in [
            ("includePrivate", "false"),
            ("includeLocked", "false"),
            ("includeNotificationPreferences", "false"),
            ("discoveryType", "0"),
            ("accessFilter", "Public"),
            ("sortOrder", "Asc"),
        ] {
            if keys.contains(&key) && !pairs.iter().any(|(k, v)| k == key && v == expected) {
                return Err(blocked());
            }
        }
        Ok(Self {
            url,
            cookie: None,
            method: Method::GET,
            body: None,
            csrf: None,
        })
    }

    fn validate(url: &Url) -> Result<(), AppError> {
        if url.scheme() != "https"
            || !url.username().is_empty()
            || url.password().is_some()
            || url.port().is_some()
            || url.fragment().is_some()
            || !matches!(
                url.host_str(),
                Some(
                    "users.roblox.com"
                        | "develop.roblox.com"
                        | "groups.roblox.com"
                        | "apis.roblox.com"
                        | "catalog.roblox.com"
                        | "games.roblox.com"
                        | "thumbnails.roblox.com"
                        | "badges.roblox.com"
                )
            )
        {
            return Err(AppError::new(
                "destination_blocked",
                "Only the allowlisted Roblox HTTPS API hosts may be addressed.",
                "Use a supported command. Arbitrary URLs, redirects, and API base-URL overrides are not supported.",
            ));
        }
        Ok(())
    }

    #[cfg(test)]
    pub fn url(&self) -> &Url {
        &self.url
    }

    #[cfg(test)]
    pub fn snapshot(&self) -> serde_json::Value {
        serde_json::json!({ "method": self.method.as_str(), "path": self.url.path(), "body": self.body,
            "csrfAttached": self.csrf.is_some(), "cookieAttached": self.cookie.is_some(),
            "cookieSensitive": self.cookie.as_ref().is_some_and(HeaderValue::is_sensitive) })
    }
}

fn fixed_query(url: &Url, keys: &[&str]) -> bool {
    let pairs: Vec<_> = url.query_pairs().collect();
    if pairs.is_empty() || pairs.iter().any(|(key, _)| !keys.contains(&key.as_ref())) {
        return false;
    }
    let required = if keys.contains(&"cursor") {
        &keys[..keys.len() - 1]
    } else {
        keys
    };
    required
        .iter()
        .all(|key| pairs.iter().any(|(name, _)| name == *key))
        && pairs
            .iter()
            .enumerate()
            .all(|(index, (key, _))| !pairs[..index].iter().any(|(other, _)| other == key))
}

fn blocked() -> AppError {
    AppError::new(
        "destination_blocked",
        "The request is not a supported Roblox route.",
        "Arbitrary endpoints, credentials in URLs, and authentication fallback are not supported.",
    )
}

pub(crate) struct HttpResponse {
    pub status: u16,
    pub retry_after_seconds: Option<u64>,
    pub body: Vec<u8>,
    pub csrf: Option<HeaderValue>,
    pub challenge: bool,
}

impl Drop for HttpResponse {
    fn drop(&mut self) {
        self.body.zeroize();
    }
}

pub(crate) trait Transport {
    fn send(&mut self, request: Request) -> Result<HttpResponse, AppError>;
}

pub(crate) struct HttpsTransport {
    client: Client,
}

impl HttpsTransport {
    pub fn new() -> Result<Self, AppError> {
        let client = client(true)?;
        Ok(Self { client })
    }
}

fn client(https_only: bool) -> Result<Client, AppError> {
    Client::builder()
        .https_only(https_only)
        .redirect(reqwest::redirect::Policy::none())
        .retry(reqwest::retry::never())
        .no_proxy()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|_| {
            AppError::network(
                "http_client_unavailable",
                "Cannot initialize the HTTPS client.",
            )
        })
}

impl Transport for HttpsTransport {
    fn send(&mut self, request: Request) -> Result<HttpResponse, AppError> {
        let mut builder = self
            .client
            .request(request.method, request.url)
            .header(ACCEPT, "application/json");
        if let Some(cookie) = request.cookie {
            builder = builder.header(COOKIE, cookie);
        }
        if let Some(body) = request.body {
            builder = builder.header(CONTENT_TYPE, "application/json").json(&body);
        }
        if let Some(csrf) = request.csrf {
            builder = builder.header("x-csrf-token", csrf);
        }
        let mut response = builder.send().map_err(|_| {
            AppError::network(
                "network_failed",
                "The read failed or timed out. No retry was attempted.",
            )
        })?;
        let status = response.status().as_u16();
        let csrf = response
            .headers()
            .get("x-csrf-token")
            .cloned()
            .map(|mut value| {
                value.set_sensitive(true);
                value
            });
        let challenge = [
            "rblx-challenge-id",
            "rblx-challenge-type",
            "rblx-challenge-metadata",
        ]
        .iter()
        .any(|name| response.headers().contains_key(*name));
        let retry_after_seconds = response
            .headers()
            .get(RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse().ok());
        let mut body = Vec::new();
        if (200..300).contains(&status) {
            response
                .by_ref()
                .take(MAX_BODY_BYTES + 1)
                .read_to_end(&mut body)
                .map_err(|_| {
                    AppError::network("response_read_failed", "Cannot read the API response.")
                })?;
            if body.len() as u64 > MAX_BODY_BYTES {
                return Err(AppError::network(
                    "response_too_large",
                    "The API response exceeded the bounded response size.",
                ));
            }
        }
        // Error bodies and redirect targets are deliberately never read or logged.
        Ok(HttpResponse {
            status,
            retry_after_seconds,
            body,
            csrf,
            challenge,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;

    use super::*;
    use crate::auth::SessionCookie;

    #[test]
    fn avatar_request_is_an_anonymous_get_and_cannot_reach_metadata_posts_or_writes() {
        let url=Url::parse("https://catalog.roblox.com/v2/search/items/details?AssetTypeIds=67&Keyword=black&limit=10").unwrap();
        let request = Request::avatar_search(url.clone()).unwrap();
        assert!(request.cookie.is_none());
        assert!(request.csrf.is_none());
        assert_eq!(request.method, Method::GET);
        assert!(Request::new(url.clone(), SessionCookie::synthetic().header().unwrap()).is_err());
        assert!(Request::creator_store(url).is_err());
        for url in [
            "https://catalog.roblox.com/v1/catalog/items/details",
            "https://avatar.roblox.com/v1/avatar/set",
            "https://catalog.roblox.com/v2/search/items/details?Cookie=secret",
            "https://users.roblox.com/v1/users/authenticated",
        ] {
            assert!(Request::avatar_search(Url::parse(url).unwrap()).is_err());
        }
    }

    #[test]
    fn public_store_requests_cannot_receive_a_cookie_or_access_private_routes() {
        let url = Url::parse("https://apis.roblox.com/toolbox-service/v2/assets:search?searchCategoryType=Audio&query=ambient").unwrap();
        let request = Request::creator_store(url.clone()).unwrap();
        assert!(request.cookie.is_none());
        assert!(request.csrf.is_none());
        assert_eq!(request.method, Method::GET);
        assert!(Request::new(url, SessionCookie::synthetic().header().unwrap()).is_err());
        for url in [
            "https://users.roblox.com/v1/users/authenticated",
            "https://apis.roblox.com/cloud-authentication/v1/apiKey",
            "https://apis.roblox.com/toolbox-service/v1/saves",
            "https://apis.roblox.com/toolbox-service/v2/assets/77?token=private",
            "https://apis.roblox.com/toolbox-service/v2/assets:search?cookie=private",
            "https://apis.roblox.com/toolbox-service/v2/assets/0",
        ] {
            assert!(Request::creator_store(Url::parse(url).unwrap()).is_err());
        }
    }

    #[test]
    fn anonymous_transport_sends_no_authentication_headers_on_the_wire() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut bytes = [0u8; 4096];
            let count = socket.read(&mut bytes).unwrap();
            let request = String::from_utf8_lossy(&bytes[..count]).to_ascii_lowercase();
            assert!(request.starts_with("get / http/1.1"));
            for header in [
                "\r\ncookie:",
                "\r\nauthorization:",
                "\r\nx-api-key:",
                "\r\nx-csrf-token:",
            ] {
                assert!(!request.contains(header));
            }
            write!(
                socket,
                "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}"
            )
            .unwrap();
        });
        // Only this loopback unit test bypasses the fixed production HTTPS routes.
        let mut transport = HttpsTransport {
            client: client(false).unwrap(),
        };
        let response = transport
            .send(Request {
                url,
                cookie: None,
                method: Method::GET,
                body: None,
                csrf: None,
            })
            .unwrap();
        assert_eq!(response.status, 200);
        server.join().unwrap();
    }

    #[test]
    fn rejects_other_hosts_http_ports_and_embedded_credentials() {
        for url in [
            "http://users.roblox.com/v1/users/authenticated",
            "https://example.com/",
            "https://users.roblox.com.example.com/",
            "https://users.roblox.com:8443/",
            "https://user:pass@users.roblox.com/",
        ] {
            assert!(
                Request::new(
                    Url::parse(url).unwrap(),
                    SessionCookie::synthetic().header().unwrap()
                )
                .is_err()
            );
        }
    }

    #[test]
    fn localhost_redirect_is_not_followed_and_cookie_is_not_forwarded() {
        let origin = TcpListener::bind("127.0.0.1:0").unwrap();
        let target = TcpListener::bind("127.0.0.1:0").unwrap();
        target.set_nonblocking(true).unwrap();
        let target_url = format!("http://{}/leak", target.local_addr().unwrap());
        let url = Url::parse(&format!("http://{}/", origin.local_addr().unwrap())).unwrap();
        let server = std::thread::spawn(move || {
            let (mut socket, _) = origin.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut request = [0; 4096];
            let count = socket.read(&mut request).unwrap();
            assert!(String::from_utf8_lossy(&request[..count]).starts_with("GET / HTTP/1.1"));
            write!(socket, "HTTP/1.1 302 Found\r\nLocation: {target_url}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
        });
        // Only this unit test bypasses the production HTTPS destination constructor.
        let mut transport = HttpsTransport {
            client: client(false).unwrap(),
        };
        let response = transport
            .send(Request {
                url,
                cookie: Some(SessionCookie::synthetic().header().unwrap()),
                method: Method::GET,
                body: None,
                csrf: None,
            })
            .unwrap();
        assert_eq!(response.status, 302);
        assert!(response.body.is_empty());
        assert!(
            matches!(target.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock)
        );
        server.join().unwrap();
    }

    #[test]
    fn thumbnail_badge_and_bundle_routes_reject_session_cookies() {
        let header = SessionCookie::synthetic().header().unwrap();
        let icon = crate::discovery::icon_url(77, "150x150", "Png").unwrap();
        assert_eq!(
            Request::game_thumbnail(icon.clone()).unwrap().snapshot()["cookieAttached"],
            false
        );
        assert!(Request::new(icon, header.clone()).is_err());
        let badges = crate::discovery::badges_url(77, "10", "Asc", None).unwrap();
        assert_eq!(
            Request::badges(badges.clone()).unwrap().snapshot()["cookieAttached"],
            false
        );
        assert!(Request::new(badges, header.clone()).is_err());
        let bundle = Url::parse("https://catalog.roblox.com/v1/bundles/77/details").unwrap();
        assert_eq!(
            Request::avatar_bundle(bundle.clone()).unwrap().snapshot()["cookieAttached"],
            false
        );
        assert!(Request::new(bundle, header).is_err());
    }
}
