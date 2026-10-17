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
            Some("groups.roblox.com") => numeric("/v2/users/", "/groups/roles"),
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

fn blocked() -> AppError {
    AppError::new(
        "destination_blocked",
        "The request is not a supported Dashboard route.",
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
}
