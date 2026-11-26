use std::{
    future::Future,
    net::{IpAddr, SocketAddr},
    time::Duration,
};

use reqwest::{
    header::{HeaderMap, HeaderValue, LOCATION},
    redirect::Policy,
    Response, StatusCode,
};
use url::{Host, Origin, Url};

pub const MAX_REDIRECTS: usize = 5;
pub const MAX_BODY_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum HttpError {
    #[error("invalid HTTP target: {0}")]
    Target(&'static str),
    #[error("HTTP target resolved to a non-public or empty address set")]
    NonPublic,
    #[error("DNS lookup failed: {0}")]
    Dns(#[from] std::io::Error),
    #[error("HTTP request failed: {}", format_reqwest_error(.0))]
    Request(#[from] reqwest::Error),
    #[error("invalid HTTP redirect location")]
    Redirect,
    #[error("too many HTTP redirects")]
    RedirectLimit,
    #[error("expected HTTP 200, got {0}")]
    Status(StatusCode),
    #[error("HTTP response body exceeded {0} bytes")]
    BodyTooLarge(usize),
    #[error("invalid Bearer credential")]
    Bearer,
}

pub fn redact_url(url: &Url) -> String {
    let mut redacted = url.clone();
    let _ = redacted.set_username("");
    let _ = redacted.set_password(None);
    redacted.set_query(None);
    redacted.set_fragment(None);
    redacted.to_string()
}

fn format_reqwest_error(err: &reqwest::Error) -> String {
    if let Some(url) = err.url() {
        err.to_string().replace(url.as_str(), &redact_url(url))
    } else {
        err.to_string()
    }
}

pub async fn read_bytes_limited(
    mut response: Response,
    limit: usize,
) -> Result<Vec<u8>, HttpError> {
    if let Some(len) = response.content_length() {
        if len > limit as u64 {
            return Err(HttpError::BodyTooLarge(limit));
        }
    }
    let mut buf = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if buf.len().saturating_add(chunk.len()) > limit {
            return Err(HttpError::BodyTooLarge(limit));
        }
        buf.extend_from_slice(&chunk);
    }
    Ok(buf)
}

pub async fn read_text_limited(response: Response, limit: usize) -> Result<String, HttpError> {
    let bytes = read_bytes_limited(response, limit).await?;
    Ok(String::from_utf8(bytes)
        .unwrap_or_else(|e| String::from_utf8_lossy(&e.into_bytes()).into_owned()))
}

pub fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let [a, b, c, d] = v4.octets();
            !matches!(
                (a, b, c, d),
                (0 | 10 | 127 | 224..=255, ..)
                    | (100, 64..=127, ..)
                    | (169, 254, ..)
                    | (172, 16..=31, ..)
                    | (192, 168, ..)
                    | (192, 0, 2, _)
                    | (192, 88, 99, _)
                    | (198, 18..=19, ..)
                    | (198, 51, 100, _)
                    | (203, 0, 113, _)
            ) && (a != 192 || b != 0 || c != 0 || matches!(d, 9 | 10))
        }
        IpAddr::V6(v6) => {
            if let Some(ip) = v6.to_ipv4_mapped() {
                return is_public_ip(ip.into());
            }
            let [a, b, c, ..] = v6.segments();
            // Public IPv6 unicast is currently allocated within 2000::/3. Exclude
            // documentation, transition and special-purpose ranges within it.
            (0x2000..=0x3fff).contains(&a)
                && !matches!(a, 0x2002 | 0x3ffe | 0x3fff)
                && !(a == 0x2001 && (b < 0x0200 || b == 0x0db8))
                && !(a == 0x2620 && b == 0x004f && c == 0x8000)
        }
    }
}

pub fn validate_target(url: &Url) -> Result<(), HttpError> {
    if !matches!(url.scheme(), "http" | "https") {
        return Err(HttpError::Target("scheme must be http or https"));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(HttpError::Target("URL credentials are forbidden"));
    }
    match url.host() {
        None => Err(HttpError::Target("missing host")),
        Some(Host::Domain(host)) => {
            let host = host.trim_end_matches('.').to_ascii_lowercase();
            if host.is_empty()
                || matches!(host.as_str(), "localhost" | "local" | "internal")
                || [".localhost", ".local", ".internal"]
                    .iter()
                    .any(|suffix| host.ends_with(suffix))
            {
                Err(HttpError::NonPublic)
            } else {
                Ok(())
            }
        }
        Some(Host::Ipv4(ip)) if is_public_ip(ip.into()) => Ok(()),
        Some(Host::Ipv6(ip)) if is_public_ip(ip.into()) => Ok(()),
        _ => Err(HttpError::NonPublic),
    }
}

fn validate_addresses(
    addrs: impl IntoIterator<Item = SocketAddr>,
) -> Result<SocketAddr, HttpError> {
    let mut selected = None;
    for addr in addrs {
        if !is_public_ip(addr.ip()) {
            return Err(HttpError::NonPublic);
        }
        selected.get_or_insert(addr);
    }
    selected.ok_or(HttpError::NonPublic)
}

async fn pinned_address(url: &Url, timeout: Duration) -> Result<Option<SocketAddr>, HttpError> {
    validate_target(url)?;
    if let Some(Host::Domain(host)) = url.host() {
        let port = url
            .port_or_known_default()
            .ok_or(HttpError::Target("missing port"))?;
        let addrs = tokio::time::timeout(timeout, tokio::net::lookup_host((host, port)))
            .await
            .map_err(|_| {
                std::io::Error::new(std::io::ErrorKind::TimedOut, "DNS lookup timed out")
            })??;
        return Ok(Some(validate_addresses(addrs)?));
    }
    // Literal IPs have already been checked and cannot trigger DNS.
    Ok(None)
}

#[derive(Clone, Debug)]
pub struct SafeHttpClient {
    timeout: Duration,
    retries: usize,
    user_agent: String,
    fixture_origin: Option<SocketAddr>,
}

impl Default for SafeHttpClient {
    fn default() -> Self {
        Self::new(Duration::from_secs(20), 1, "ai-digest")
    }
}

impl SafeHttpClient {
    pub fn new(timeout: Duration, retries: usize, user_agent: impl Into<String>) -> Self {
        Self {
            timeout,
            retries,
            user_agent: user_agent.into(),
            fixture_origin: None,
        }
    }

    /// Attach an explicitly supplied, local in-process fixture transport.
    ///
    /// This is not read from environment variables or available in the CLI.
    /// It never contacts a logical target or resolves its hostname.
    #[doc(hidden)]
    pub fn with_fixture_origin(mut self, origin: SocketAddr) -> Result<Self, HttpError> {
        if !origin.ip().is_loopback() {
            return Err(HttpError::Target("fixture must be on loopback"));
        }
        self.fixture_origin = Some(origin);
        Ok(self)
    }

    pub async fn get(&self, url: &Url) -> Result<Response, HttpError> {
        self.get_with_bearer(url, None).await
    }

    pub async fn get_with_bearer(
        &self,
        url: &Url,
        bearer: Option<(&Url, &str)>,
    ) -> Result<Response, HttpError> {
        self.get_with_headers(url, bearer, &HeaderMap::new()).await
    }

    pub async fn get_with_headers(
        &self,
        url: &Url,
        bearer: Option<(&Url, &str)>,
        headers: &HeaderMap,
    ) -> Result<Response, HttpError> {
        let bearer = bearer
            .map(|(origin, token)| {
                let mut value = HeaderValue::from_str(&format!("Bearer {token}"))
                    .map_err(|_| HttpError::Bearer)?;
                value.set_sensitive(true);
                Ok::<_, HttpError>((origin.origin(), value))
            })
            .transpose()?;
        self.follow(url, bearer, true, |url, auth| self.send(url, auth, headers))
            .await
    }

    pub async fn fetch_exact_200_no_redirect(&self, url: &Url) -> Result<Response, HttpError> {
        let headers = HeaderMap::new();
        self.follow(url, None, false, |url, auth| self.send(url, auth, &headers))
            .await
    }

    async fn send(
        &self,
        url: Url,
        auth: Option<HeaderValue>,
        headers: &HeaderMap,
    ) -> Result<Response, HttpError> {
        let mut attempt = 0;
        loop {
            let addr = if self.fixture_origin.is_none() {
                pinned_address(&url, self.timeout).await?
            } else {
                None
            };
            let mut client = reqwest::Client::builder()
                .no_proxy()
                .redirect(Policy::none())
                .retry(reqwest::retry::never())
                .timeout(self.timeout)
                .user_agent(&self.user_agent);
            if let Some(addr) = addr {
                client = client.resolve(url.host_str().expect("validated domain"), addr);
            }
            let mut headers = headers.clone();
            headers.remove(reqwest::header::AUTHORIZATION);
            headers.remove(reqwest::header::PROXY_AUTHORIZATION);
            headers.remove(reqwest::header::COOKIE);
            let target = if let Some(origin) = self.fixture_origin {
                Url::parse(&format!("http://{origin}"))
                    .expect("loopback socket URL")
                    .join(&format!(
                        ".{}",
                        &url[url::Position::BeforePath..url::Position::AfterQuery]
                    ))
                    .expect("relative fixture URL")
            } else {
                url.clone()
            };
            let mut request = client.build()?.get(target).headers(headers);
            if let Some(value) = &auth {
                request = request.header(reqwest::header::AUTHORIZATION, value.clone());
            }
            match request.send().await {
                Err(err) if attempt < self.retries && (err.is_connect() || err.is_timeout()) => {
                    attempt += 1
                }
                response => return response.map_err(Into::into),
            }
        }
    }

    async fn follow<F, Fut>(
        &self,
        url: &Url,
        mut bearer: Option<(Origin, HeaderValue)>,
        redirects: bool,
        mut send: F,
    ) -> Result<Response, HttpError>
    where
        F: FnMut(Url, Option<HeaderValue>) -> Fut,
        Fut: Future<Output = Result<Response, HttpError>>,
    {
        let mut current = url.clone();
        for hop in 0..=MAX_REDIRECTS {
            validate_target(&current)?;
            let auth = bearer
                .as_ref()
                .filter(|(origin, _)| *origin == current.origin())
                .map(|(_, token)| token.clone());
            let response = send(current.clone(), auth).await?;
            let status = response.status();
            if !redirects {
                return if status == StatusCode::OK {
                    Ok(response)
                } else {
                    Err(HttpError::Status(status))
                };
            }
            if !matches!(status.as_u16(), 301 | 302 | 303 | 307 | 308) {
                return Ok(response);
            }
            let Some(location) = response.headers().get(LOCATION) else {
                return Ok(response);
            };
            if hop == MAX_REDIRECTS {
                return Err(HttpError::RedirectLimit);
            }
            let next = current
                .join(location.to_str().map_err(|_| HttpError::Redirect)?)
                .map_err(|_| HttpError::Redirect)?;
            if current.origin() != next.origin() {
                bearer = None;
            }
            current = next;
        }
        Err(HttpError::RedirectLimit)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::{
        matchers::{header, method, path},
        Mock, MockServer, ResponseTemplate,
    };

    #[test]
    fn redact_url_strips_credentials_query_and_fragment() {
        let raw =
            Url::parse("https://alice:s3cr3t@digest.example.com/daily/2026-01-15?token=abc#sec")
                .unwrap();
        let redacted = redact_url(&raw);
        assert_eq!(redacted, "https://digest.example.com/daily/2026-01-15");
        assert!(!redacted.contains("alice"));
        assert!(!redacted.contains("s3cr3t"));
        assert!(!redacted.contains("token=abc"));
        assert!(!redacted.contains("#sec"));
    }

    #[tokio::test]
    async fn read_bytes_limited_rejects_oversized_response() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/big"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![b'a'; 2048]))
            .mount(&server)
            .await;
        let client = SafeHttpClient::default()
            .with_fixture_origin(*server.address())
            .unwrap();
        let url = Url::parse("https://example.com/big").unwrap();
        let resp = client.get(&url).await.unwrap();
        assert!(matches!(
            read_bytes_limited(resp, 1024).await,
            Err(HttpError::BodyTooLarge(1024))
        ));
        let resp = client.get(&url).await.unwrap();
        assert_eq!(read_bytes_limited(resp, 2048).await.unwrap().len(), 2048);
    }

    #[test]
    fn rejects_every_unapproved_dns_result() {
        let public = "1.1.1.1:443".parse().unwrap();
        let private = "[::ffff:127.0.0.1]:443".parse().unwrap();
        assert_eq!(validate_addresses([public]).unwrap(), public);
        assert!(matches!(validate_addresses([]), Err(HttpError::NonPublic)));
        assert!(matches!(
            validate_addresses([public, private]),
            Err(HttpError::NonPublic)
        ));
        assert!(matches!(
            validate_addresses([private, public]),
            Err(HttpError::NonPublic)
        ));
    }

    #[tokio::test]
    async fn validates_mock_redirects_and_drops_bearer_on_origin_change() {
        let server = MockServer::start().await;
        let fixture_credential = ["Bearer ", "fixture-value"].concat();
        for (route, target) in [
            ("/first", "/same"),
            ("/same", "http://second.example/final"),
            ("/blocked", "http://169.254.169.254/forbidden"),
            ("/cycle", "/cycle"),
        ] {
            let mut mock = Mock::given(method("GET")).and(path(route));
            if matches!(route, "/first" | "/same") {
                mock = mock.and(header("authorization", fixture_credential.as_str()));
            }
            mock.respond_with(ResponseTemplate::new(302).insert_header("location", target))
                .mount(&server)
                .await;
        }
        Mock::given(method("GET"))
            .and(path("/final"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;

        let client = SafeHttpClient::default();
        let local = Url::parse(&server.uri()).unwrap();
        let transport = reqwest::Client::builder()
            .no_proxy()
            .redirect(Policy::none())
            .build()
            .unwrap();
        // Only the test transport maps already-validated logical origins to Wiremock.
        // The production transport always pins a real, validated DNS result.
        let send = |target: Url, auth: Option<HeaderValue>| {
            let transport = transport.clone();
            let endpoint = local.join(target.path()).unwrap();
            async move {
                let mut get = transport.get(endpoint);
                if let Some(auth) = auth {
                    get = get.header(reqwest::header::AUTHORIZATION, auth);
                }
                Ok(get.send().await?)
            }
        };
        let first = Url::parse("http://first.example/first").unwrap();
        let auth = HeaderValue::from_str(&fixture_credential).unwrap();
        let response = client
            .follow(&first, Some((first.origin(), auth)), true, send)
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let requests = server.received_requests().await.unwrap();
        assert_eq!(
            requests.iter().map(|r| r.url.path()).collect::<Vec<_>>(),
            ["/first", "/same", "/final"]
        );
        assert!(requests[2].headers.get("authorization").is_none());

        assert!(matches!(
            client
                .follow(&first.join("/blocked").unwrap(), None, true, send)
                .await,
            Err(HttpError::NonPublic)
        ));
        assert!(matches!(
            client
                .follow(&first.join("/cycle").unwrap(), None, true, send)
                .await,
            Err(HttpError::RedirectLimit)
        ));
        assert!(matches!(
            client
                .follow(
                    &first,
                    Some((
                        first.origin(),
                        HeaderValue::from_str(&fixture_credential).unwrap()
                    )),
                    false,
                    send
                )
                .await,
            Err(HttpError::Status(StatusCode::FOUND))
        ));
        assert!(matches!(
            client.get(&local).await,
            Err(HttpError::NonPublic)
        ));
        assert!(matches!(
            client.fetch_exact_200_no_redirect(&local).await,
            Err(HttpError::NonPublic)
        ));
        let requests = server.received_requests().await.unwrap();
        assert!(!requests
            .iter()
            .any(|r| matches!(r.url.path(), "/forbidden" | "/")));
        assert_eq!(
            requests.iter().filter(|r| r.url.path() == "/cycle").count(),
            MAX_REDIRECTS + 1
        );
    }
}
