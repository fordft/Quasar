//! Fixed-origin HTTPS transport for authenticated Zephium repositories.
//!
//! This crate owns only the untrusted network boundary shared by independent
//! update domains. It grants no package, catalog, activation, or filesystem
//! authority. Repository callers authenticate returned bytes with their signed
//! policy. The official-filter API explicitly provides only HTTPS server trust
//! and requires independent source validation and last-known-good activation.

#![deny(missing_docs)]
#![deny(unsafe_code)]

pub mod chrome_store;
pub mod official_filters;

use std::fmt;
use std::time::Duration;

#[cfg(feature = "tough")]
use async_trait::async_trait;
use futures_util::TryStreamExt;
use reqwest::header::{
    HeaderMap, HeaderValue, ACCEPT, ACCEPT_ENCODING, CACHE_CONTROL, CONTENT_ENCODING,
    CONTENT_LENGTH, IF_NONE_MATCH, TRANSFER_ENCODING,
};
use reqwest::redirect::Policy;
use reqwest::{Client, Response, StatusCode};
use thiserror::Error;
#[cfg(feature = "tough")]
use tough::{Transport, TransportError, TransportErrorKind, TransportStream};
use url::Url;

/// Untrusted conditional HTTP result. A cache hit is not a signature check
/// and does not extend signed metadata expiry or policy freshness.
#[derive(Debug, Eq, PartialEq)]
#[must_use = "conditional responses require cached-byte revalidation or authentication of new bytes"]
pub enum ConditionalFixedOriginResponse {
    /// Complete bounded bytes; the caller must authenticate before caching.
    Modified(Box<[u8]>),
    /// The server reports the caller's exact cached content hash unchanged.
    /// The caller must revalidate its cached bytes and signed deadlines.
    NotModified,
}

/// Strong ETag derived only from a shared content digest, never an opaque
/// server-selected token that could become a persistent client identifier.
pub fn content_digest_etag(sha256: [u8; 32]) -> HeaderValue {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut bytes = [b'"'; 66];
    for (index, byte) in sha256.into_iter().enumerate() {
        bytes[1 + index * 2] = HEX[usize::from(byte >> 4)];
        bytes[2 + index * 2] = HEX[usize::from(byte & 15)];
    }
    HeaderValue::from_bytes(&bytes).expect("fixed quoted hexadecimal digest is an HTTP header")
}

/// Configuration failure before any request can be issued.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum FixedOriginTransportConfigError {
    /// A base URL is not an unambiguous absolute HTTPS directory.
    #[error("repository base URL is invalid")]
    InvalidBaseUrl,
    /// Request and connection deadlines are empty or contradictory.
    #[error("repository transport deadlines are invalid")]
    InvalidDeadlines,
    /// The static product user agent is not a valid HTTP header value.
    #[error("repository transport user agent is invalid")]
    InvalidUserAgent,
    /// The HTTPS client could not be constructed.
    #[error("repository HTTPS client is unavailable")]
    ClientUnavailable,
}

/// Stable, URL-free failure while reading one fixed-origin response.
///
/// Diagnostics deliberately retain no response body, request path, query, or
/// transport-library error. Callers may classify availability without making
/// attacker-controlled network text part of logs or user-visible failures.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
#[non_exhaustive]
pub enum FixedOriginFetchError {
    /// The request escaped the two exact configured HTTPS directories.
    #[error("repository request is outside its fixed origin boundary")]
    Boundary,
    /// The request or response body exceeded the configured deadline.
    #[error("repository request timed out")]
    Timeout,
    /// A connection could not be established.
    #[error("repository connection failed")]
    Connect,
    /// Request construction or transmission failed.
    #[error("repository request failed")]
    Request,
    /// The requested immutable object is absent.
    #[error("repository object was not found")]
    NotFound,
    /// The server returned another non-success status.
    #[error("repository returned a non-success status")]
    Status,
    /// The final response URL did not remain exact and fixed-origin.
    #[error("repository response crossed its fixed origin boundary")]
    FinalUrl,
    /// The server returned a compressed or ambiguously encoded response.
    #[error("repository response encoding is unsupported")]
    ResponseEncoding,
    /// A bounded immutable-object read lacked one exact decimal length.
    #[error("repository response length is missing or ambiguous")]
    ResponseLength,
    /// The declared or observed response body exceeded the caller's ceiling.
    #[error("repository response exceeded its byte limit")]
    ResponseTooLarge,
    /// The body ended at a different length than the exact HTTP declaration.
    #[error("repository response length changed during transfer")]
    ResponseLengthMismatch,
    /// Memory for the already-bounded response could not be reserved.
    #[error("repository response memory is unavailable")]
    CapacityUnavailable,
    /// Streaming the admitted response body failed.
    #[error("repository response body failed")]
    Body,
}

/// Redirect-free HTTPS transport confined to two exact repository directories.
///
/// Metadata and target origins may differ, but each request must remain below
/// one of the two configured base paths. URLs carrying credentials, queries,
/// fragments, backslashes, or percent-encoded path components are refused so
/// intermediary-specific decoding cannot widen the boundary.
#[derive(Clone)]
pub struct FixedOriginTransport {
    client: Client,
    metadata_base: Url,
    targets_base: Url,
}

impl fmt::Debug for FixedOriginTransport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FixedOriginTransport")
            .field("metadata_origin", &redacted_origin(&self.metadata_base))
            .field("targets_origin", &redacted_origin(&self.targets_base))
            .finish_non_exhaustive()
    }
}

impl FixedOriginTransport {
    /// Constructs one transport without performing network I/O.
    ///
    /// The caller retains responsibility for a stricter domain-specific upper
    /// bound on both deadlines and for authenticating repository metadata.
    pub fn new(
        metadata_base: Url,
        targets_base: Url,
        request_timeout: Duration,
        connect_timeout: Duration,
        user_agent: &'static str,
    ) -> Result<Self, FixedOriginTransportConfigError> {
        if !valid_base(&metadata_base) || !valid_base(&targets_base) {
            return Err(FixedOriginTransportConfigError::InvalidBaseUrl);
        }
        if connect_timeout.is_zero()
            || request_timeout.is_zero()
            || connect_timeout > request_timeout
        {
            return Err(FixedOriginTransportConfigError::InvalidDeadlines);
        }
        let user_agent = HeaderValue::from_str(user_agent)
            .map_err(|_| FixedOriginTransportConfigError::InvalidUserAgent)?;
        if user_agent.is_empty() {
            return Err(FixedOriginTransportConfigError::InvalidUserAgent);
        }
        let client = Client::builder()
            .https_only(true)
            .redirect(Policy::none())
            .timeout(request_timeout)
            .connect_timeout(connect_timeout)
            .user_agent(user_agent)
            .build()
            .map_err(|_| FixedOriginTransportConfigError::ClientUnavailable)?;
        Ok(Self {
            client,
            metadata_base,
            targets_base,
        })
    }

    fn admits(&self, url: &Url) -> bool {
        is_below_fixed_base(url, &self.metadata_base)
            || is_below_fixed_base(url, &self.targets_base)
    }

    /// Fetches one immutable object into an exact-size allocation.
    ///
    /// This stricter API is intended for package/catalog domains that already
    /// know a hard object ceiling. It requires one uncompressed response with
    /// exactly one decimal `Content-Length`, rejects transfer framing that can
    /// contradict that declaration, reserves only the declared bounded size,
    /// and verifies the observed body length before returning bytes.
    pub async fn fetch_bounded(
        &self,
        url: Url,
        max_bytes: usize,
    ) -> Result<Box<[u8]>, FixedOriginFetchError> {
        if max_bytes == 0 {
            return Err(FixedOriginFetchError::ResponseTooLarge);
        }
        let response = self.send(url).await?;
        Self::read_bounded_response(response, max_bytes).await
    }

    /// Conditionally fetches shared metadata using SHA-256 of the caller's
    /// authenticated cached bytes. Arbitrary server ETags are never replayed.
    /// A 304 without a supplied cache digest is rejected. This method retains
    /// the ordinary redirect, encoding, size, and response-length boundaries.
    pub async fn fetch_bounded_conditional(
        &self,
        url: Url,
        max_bytes: usize,
        cached_sha256: Option<[u8; 32]>,
    ) -> Result<ConditionalFixedOriginResponse, FixedOriginFetchError> {
        if max_bytes == 0 {
            return Err(FixedOriginFetchError::ResponseTooLarge);
        }
        let response = self.send_conditional(url, cached_sha256).await?;
        if response.status() == StatusCode::NOT_MODIFIED {
            return Ok(ConditionalFixedOriginResponse::NotModified);
        }
        Self::read_bounded_response(response, max_bytes)
            .await
            .map(ConditionalFixedOriginResponse::Modified)
    }

    async fn read_bounded_response(
        response: Response,
        max_bytes: usize,
    ) -> Result<Box<[u8]>, FixedOriginFetchError> {
        let declared = exact_content_length(response.headers())?;
        if declared > max_bytes {
            return Err(FixedOriginFetchError::ResponseTooLarge);
        }

        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(declared)
            .map_err(|_| FixedOriginFetchError::CapacityUnavailable)?;
        if bytes.capacity() > max_bytes {
            return Err(FixedOriginFetchError::ResponseTooLarge);
        }
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream
            .try_next()
            .await
            .map_err(|error| classify_network_error(&error))?
        {
            let observed = bytes
                .len()
                .checked_add(chunk.len())
                .ok_or(FixedOriginFetchError::ResponseTooLarge)?;
            if observed > declared || observed > max_bytes {
                return Err(FixedOriginFetchError::ResponseTooLarge);
            }
            bytes.extend_from_slice(&chunk);
        }
        if bytes.len() != declared {
            return Err(FixedOriginFetchError::ResponseLengthMismatch);
        }
        Ok(bytes.into_boxed_slice())
    }

    async fn send(&self, url: Url) -> Result<Response, FixedOriginFetchError> {
        self.send_conditional(url, None).await
    }

    fn request(
        &self,
        url: &Url,
        cached_sha256: Option<[u8; 32]>,
    ) -> Result<reqwest::RequestBuilder, FixedOriginFetchError> {
        if !self.admits(url) {
            return Err(FixedOriginFetchError::Boundary);
        }
        let mut request = self
            .client
            .get(url.clone())
            .header(
                ACCEPT,
                HeaderValue::from_static(
                    "application/octet-stream, application/json;q=0.9, text/plain;q=0.8",
                ),
            )
            .header(ACCEPT_ENCODING, HeaderValue::from_static("identity"))
            .header(CACHE_CONTROL, HeaderValue::from_static("no-cache"));
        if let Some(digest) = cached_sha256 {
            request = request.header(IF_NONE_MATCH, content_digest_etag(digest));
        }
        Ok(request)
    }

    async fn send_conditional(
        &self,
        url: Url,
        cached_sha256: Option<[u8; 32]>,
    ) -> Result<Response, FixedOriginFetchError> {
        let response = self
            .request(&url, cached_sha256)?
            .send()
            .await
            .map_err(|error| classify_network_error(&error))?;

        if !self.admits(response.url()) || response.url() != &url {
            return Err(FixedOriginFetchError::FinalUrl);
        }
        if !response_encoding_admitted(response.headers()) {
            return Err(FixedOriginFetchError::ResponseEncoding);
        }
        let status = response.status();
        if !response_status_admitted(status, cached_sha256.is_some()) {
            return Err(
                if matches!(
                    status,
                    StatusCode::FORBIDDEN | StatusCode::NOT_FOUND | StatusCode::GONE
                ) {
                    FixedOriginFetchError::NotFound
                } else {
                    FixedOriginFetchError::Status
                },
            );
        }
        Ok(response)
    }
}

fn response_status_admitted(status: StatusCode, has_cached_digest: bool) -> bool {
    status == StatusCode::OK || (status == StatusCode::NOT_MODIFIED && has_cached_digest)
}

#[cfg(feature = "tough")]
#[async_trait]
impl Transport for FixedOriginTransport {
    async fn fetch(&self, url: Url) -> Result<TransportStream, TransportError> {
        let diagnostic_url = redacted_origin(&url);
        let response = self.send(url).await.map_err(|error| {
            let kind = if error == FixedOriginFetchError::Boundary {
                TransportErrorKind::UnsupportedUrlScheme
            } else if error == FixedOriginFetchError::NotFound {
                TransportErrorKind::FileNotFound
            } else {
                TransportErrorKind::Other
            };
            TransportError::new_with_cause(kind, &diagnostic_url, error)
        })?;

        let stream_url = diagnostic_url.clone();
        let stream = response.bytes_stream().map_err(move |error| {
            TransportError::new_with_cause(
                TransportErrorKind::Other,
                &stream_url,
                classify_network_error(&error),
            )
        });
        Ok(Box::pin(stream))
    }
}

fn exact_content_length(headers: &HeaderMap) -> Result<usize, FixedOriginFetchError> {
    if headers.contains_key(TRANSFER_ENCODING) {
        return Err(FixedOriginFetchError::ResponseLength);
    }
    let mut values = headers.get_all(CONTENT_LENGTH).iter();
    let value = match (values.next(), values.next()) {
        (Some(value), None) => value.as_bytes(),
        _ => return Err(FixedOriginFetchError::ResponseLength),
    };
    if value.is_empty()
        || (value.len() > 1 && value[0] == b'0')
        || !value.iter().all(u8::is_ascii_digit)
    {
        return Err(FixedOriginFetchError::ResponseLength);
    }
    let value = std::str::from_utf8(value)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .and_then(|value| usize::try_from(value).ok())
        .filter(|value| *value != 0)
        .ok_or(FixedOriginFetchError::ResponseLength)?;
    Ok(value)
}

fn response_encoding_admitted(headers: &HeaderMap) -> bool {
    let mut encodings = headers.get_all(CONTENT_ENCODING).iter();
    match (encodings.next(), encodings.next()) {
        (None, None) => true,
        (Some(encoding), None) => encoding.as_bytes().eq_ignore_ascii_case(b"identity"),
        _ => false,
    }
}

fn classify_network_error(error: &reqwest::Error) -> FixedOriginFetchError {
    if error.is_timeout() {
        FixedOriginFetchError::Timeout
    } else if error.is_connect() {
        FixedOriginFetchError::Connect
    } else if error.is_request() {
        FixedOriginFetchError::Request
    } else if error.is_body() {
        FixedOriginFetchError::Body
    } else {
        FixedOriginFetchError::Request
    }
}

fn valid_base(url: &Url) -> bool {
    url.scheme() == "https"
        && url.has_host()
        && url.username().is_empty()
        && url.password().is_none()
        && url.query().is_none()
        && url.fragment().is_none()
        && url.path().starts_with('/')
        && url.path().ends_with('/')
        && valid_path(url.path())
}

fn is_below_fixed_base(candidate: &Url, base: &Url) -> bool {
    candidate.scheme() == "https"
        && candidate.scheme() == base.scheme()
        && candidate.host_str() == base.host_str()
        && candidate.port_or_known_default() == base.port_or_known_default()
        && candidate.username().is_empty()
        && candidate.password().is_none()
        && candidate.query().is_none()
        && candidate.fragment().is_none()
        && valid_path(candidate.path())
        && candidate.path().starts_with(base.path())
}

fn valid_path(path: &str) -> bool {
    path.is_ascii()
        && !path.contains('\\')
        && !path.contains('%')
        && !path.bytes().any(|byte| byte.is_ascii_control())
}

fn redacted_origin(url: &Url) -> String {
    let host = url.host_str().unwrap_or("<invalid>");
    match url.port() {
        Some(port) => format!("{}://{host}:{port}/…", url.scheme()),
        None => format!("{}://{host}/…", url.scheme()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conditional_status_cannot_authorize_an_unsolicited_cache_hit_or_partial_body() {
        assert!(response_status_admitted(StatusCode::OK, false));
        assert!(response_status_admitted(StatusCode::NOT_MODIFIED, true));
        assert!(!response_status_admitted(StatusCode::NOT_MODIFIED, false));
        for status in [
            StatusCode::PARTIAL_CONTENT,
            StatusCode::NO_CONTENT,
            StatusCode::FOUND,
            StatusCode::NOT_FOUND,
            StatusCode::TOO_MANY_REQUESTS,
            StatusCode::INTERNAL_SERVER_ERROR,
        ] {
            assert!(!response_status_admitted(status, false));
            assert!(!response_status_admitted(status, true));
        }
    }

    #[test]
    fn conditional_requests_replay_only_a_shared_content_digest() {
        let transport = transport();
        let url = Url::parse("https://updates.example/metadata/timestamp.json").unwrap();
        let request = transport
            .request(&url, Some([0xab; 32]))
            .unwrap()
            .build()
            .unwrap();
        assert_eq!(
            request.headers()[IF_NONE_MATCH],
            format!("\"{}\"", "ab".repeat(32))
        );
        assert!(!request.headers().contains_key(reqwest::header::COOKIE));
        assert!(!request
            .headers()
            .contains_key(reqwest::header::AUTHORIZATION));
        assert!(!transport
            .request(&url, None)
            .unwrap()
            .build()
            .unwrap()
            .headers()
            .contains_key(IF_NONE_MATCH));
        assert!(transport
            .request(
                &Url::parse("https://evil.example/metadata/timestamp.json").unwrap(),
                Some([0; 32])
            )
            .is_err());
    }

    fn transport() -> FixedOriginTransport {
        FixedOriginTransport::new(
            Url::parse("https://updates.example/metadata/").unwrap(),
            Url::parse("https://cdn.example/targets/").unwrap(),
            Duration::from_secs(30),
            Duration::from_secs(10),
            "Quasar-Test-Updater/1",
        )
        .unwrap()
    }

    #[test]
    fn exact_origins_ports_and_base_directories_are_required() {
        let transport = transport();

        assert!(transport
            .admits(&Url::parse("https://updates.example/metadata/timestamp.json").unwrap()));
        assert!(transport.admits(&Url::parse("https://cdn.example/targets/a.crx").unwrap()));
        for rejected in [
            "https://updates.example/metadata-evil/timestamp.json",
            "https://updates.example/other/timestamp.json",
            "https://updates.example.evil/metadata/root.json",
            "http://updates.example/metadata/root.json",
            "https://updates.example:444/metadata/root.json",
            "https://updates.example/metadata/root.json?token=x",
            "https://updates.example/metadata/root.json#fragment",
            "https://updates.example/metadata/%2f..%2fprivate",
            "https://updates.example/metadata/%5cprivate",
            "https://updates.example/metadata/%2e%2e/private",
            "https://updates.example/metadata/%252fprivate",
        ] {
            assert!(
                !transport.admits(&Url::parse(rejected).unwrap()),
                "{rejected}"
            );
        }
    }

    #[test]
    fn ambiguous_bases_and_deadlines_are_rejected() {
        let good = Url::parse("https://updates.example/metadata/").unwrap();
        let target = Url::parse("https://cdn.example/targets/").unwrap();
        for invalid in [
            "http://updates.example/metadata/",
            "https://user@updates.example/metadata/",
            "https://updates.example/metadata",
            "https://updates.example/metadata/?channel=stable",
            "https://updates.example/%6detadata/",
        ] {
            assert_eq!(
                FixedOriginTransport::new(
                    Url::parse(invalid).unwrap(),
                    target.clone(),
                    Duration::from_secs(30),
                    Duration::from_secs(10),
                    "Quasar-Test-Updater/1",
                )
                .unwrap_err(),
                FixedOriginTransportConfigError::InvalidBaseUrl,
                "{invalid}"
            );
        }
        assert_eq!(
            FixedOriginTransport::new(
                good,
                target,
                Duration::from_secs(5),
                Duration::from_secs(6),
                "Quasar-Test-Updater/1",
            )
            .unwrap_err(),
            FixedOriginTransportConfigError::InvalidDeadlines
        );
    }

    #[test]
    fn debug_output_never_contains_repository_paths() {
        let rendered = format!("{:?}", transport());
        assert!(rendered.contains("https://updates.example/…"));
        assert!(rendered.contains("https://cdn.example/…"));
        assert!(!rendered.contains("/metadata/"));
        assert!(!rendered.contains("/targets/"));
    }

    #[test]
    fn successful_response_encoding_must_be_absent_or_exact_identity() {
        let mut headers = HeaderMap::new();
        assert!(response_encoding_admitted(&headers));
        headers.insert(CONTENT_ENCODING, HeaderValue::from_static("identity"));
        assert!(response_encoding_admitted(&headers));
        headers.insert(CONTENT_ENCODING, HeaderValue::from_static("gzip"));
        assert!(!response_encoding_admitted(&headers));
        headers.append(CONTENT_ENCODING, HeaderValue::from_static("identity"));
        assert!(!response_encoding_admitted(&headers));
    }

    #[test]
    fn bounded_reads_require_one_canonical_nonzero_content_length() {
        let mut headers = HeaderMap::new();
        assert_eq!(
            exact_content_length(&headers),
            Err(FixedOriginFetchError::ResponseLength)
        );
        headers.insert(CONTENT_LENGTH, HeaderValue::from_static("17"));
        assert_eq!(exact_content_length(&headers), Ok(17));
        headers.insert(CONTENT_LENGTH, HeaderValue::from_static("0"));
        assert_eq!(
            exact_content_length(&headers),
            Err(FixedOriginFetchError::ResponseLength)
        );
        headers.insert(CONTENT_LENGTH, HeaderValue::from_static("017"));
        assert_eq!(
            exact_content_length(&headers),
            Err(FixedOriginFetchError::ResponseLength)
        );
        headers.insert(CONTENT_LENGTH, HeaderValue::from_static("17"));
        headers.append(CONTENT_LENGTH, HeaderValue::from_static("17"));
        assert_eq!(
            exact_content_length(&headers),
            Err(FixedOriginFetchError::ResponseLength)
        );
        headers = HeaderMap::new();
        headers.insert(CONTENT_LENGTH, HeaderValue::from_static("17"));
        headers.insert(TRANSFER_ENCODING, HeaderValue::from_static("chunked"));
        assert_eq!(
            exact_content_length(&headers),
            Err(FixedOriginFetchError::ResponseLength)
        );
    }
}
