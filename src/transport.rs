use std::io::Read;
use std::time::Duration;

use reqwest::Url;
use reqwest::blocking::Client;
use reqwest::header::{ACCEPT, COOKIE, HeaderValue, RETRY_AFTER};

use crate::error::AppError;

const MAX_BODY_BYTES: u64 = 2 * 1024 * 1024;

pub(crate) struct ReadRequest {
    url: Url,
    cookie: HeaderValue,
}

impl ReadRequest {
    pub fn new(url: Url, cookie: HeaderValue) -> Result<Self, AppError> {
        if url.scheme() != "https"
            || !url.username().is_empty()
            || url.password().is_some()
            || url.port().is_some()
            || !matches!(
                url.host_str(),
                Some("users.roblox.com" | "develop.roblox.com")
            )
        {
            return Err(AppError::new(
                "destination_blocked",
                "Only the allowlisted Roblox HTTPS API hosts may receive the session.",
                "Use a supported command. Arbitrary URLs, redirects, and API base-URL overrides are not supported.",
            ));
        }
        Ok(Self { url, cookie })
    }

    #[cfg(test)]
    pub fn url(&self) -> &Url {
        &self.url
    }
}

pub(crate) struct HttpResponse {
    pub status: u16,
    pub retry_after_seconds: Option<u64>,
    pub body: Vec<u8>,
}

pub(crate) trait Transport {
    fn get(&mut self, request: ReadRequest) -> Result<HttpResponse, AppError>;
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
    fn get(&mut self, request: ReadRequest) -> Result<HttpResponse, AppError> {
        let mut response = self
            .client
            .get(request.url)
            .header(ACCEPT, "application/json")
            .header(COOKIE, request.cookie)
            .send()
            .map_err(|_| {
                AppError::network(
                    "network_failed",
                    "The read failed or timed out. No retry was attempted.",
                )
            })?;
        let status = response.status().as_u16();
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
    fn rejects_other_hosts_http_ports_and_embedded_credentials() {
        for url in [
            "http://users.roblox.com/v1/users/authenticated",
            "https://example.com/",
            "https://users.roblox.com.example.com/",
            "https://users.roblox.com:8443/",
            "https://user:pass@users.roblox.com/",
        ] {
            assert!(
                ReadRequest::new(
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
            .get(ReadRequest {
                url,
                cookie: SessionCookie::synthetic().header().unwrap(),
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
