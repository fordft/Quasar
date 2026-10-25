//! Direct device-to-Google package transport. A response is untrusted until
//! the caller verifies its CRX signature, requested ID and archive contents.
use super::{
    FixedOriginFetchError as Error, FixedOriginTransport, FixedOriginTransportConfigError,
};
use reqwest::{
    header::{ACCEPT_ENCODING, LOCATION},
    Client, StatusCode,
};
use std::time::{Duration, Instant};
use url::Url;

/// Bounded, cookieless extension-download client with fixed Google endpoints.
/// It accepts an extension ID, never a page-provided download URL.
pub struct ChromeStoreTransport {
    client: Client,
    version: String,
}
impl std::fmt::Debug for ChromeStoreTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChromeStoreTransport")
            .finish_non_exhaustive()
    }
}
impl ChromeStoreTransport {
    /// `selection_version` is the update service's package-selection version,
    /// not a claim that the host implements every API in that Chrome release.
    pub fn new(selection_version: &str) -> Result<Self, FixedOriginTransportConfigError> {
        if !valid_version(selection_version) {
            return Err(FixedOriginTransportConfigError::InvalidBaseUrl);
        }
        let client = Client::builder()
            .https_only(true)
            .redirect(reqwest::redirect::Policy::none())
            .referer(false)
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(45))
            .user_agent("Quasar/0.1 ExtensionInstaller")
            .build()
            .map_err(|_| FixedOriginTransportConfigError::ClientUnavailable)?;
        Ok(Self {
            client,
            version: selection_version.to_owned(),
        })
    }

    /// Downloads the current original package, or reports no offered update.
    /// No opaque server ETag, cookie, account or client identifier is stored.
    /// Dropping this future cancels the request; the client owns no worker.
    pub async fn fetch(
        &self,
        id: &str,
        installed_version: Option<&str>,
        max_bytes: usize,
    ) -> Result<Option<Box<[u8]>>, Error> {
        if max_bytes == 0 {
            return Err(Error::ResponseTooLarge);
        }
        let mut url = request_url(id, &self.version, installed_version)?;
        let started = Instant::now();
        for hop in 0..=2 {
            let remaining = Duration::from_secs(45)
                .checked_sub(started.elapsed())
                .ok_or(Error::Timeout)?;
            let response = self
                .client
                .get(url.clone())
                .header(ACCEPT_ENCODING, "identity")
                .timeout(remaining)
                .send()
                .await
                .map_err(|error| super::classify_network_error(&error))?;
            if response.url() != &url {
                return Err(Error::FinalUrl);
            }
            if !super::response_encoding_admitted(response.headers()) {
                return Err(Error::ResponseEncoding);
            }
            if response.status() == StatusCode::NO_CONTENT {
                return Ok(None);
            }
            if response.status().is_redirection() {
                if hop == 2 {
                    return Err(Error::FinalUrl);
                }
                let values = response
                    .headers()
                    .get_all(LOCATION)
                    .iter()
                    .collect::<Vec<_>>();
                if values.len() != 1 {
                    return Err(Error::FinalUrl);
                }
                let next = values[0]
                    .to_str()
                    .ok()
                    .filter(|value| value.len() <= 4096)
                    .and_then(|value| url.join(value).ok())
                    .ok_or(Error::FinalUrl)?;
                if !package_endpoint(&next) {
                    return Err(Error::Boundary);
                }
                url = next;
                continue;
            }
            if !package_endpoint(&url) {
                return Err(Error::Boundary);
            }
            if response.status() == StatusCode::NOT_FOUND {
                return Err(Error::NotFound);
            }
            if response.status() != StatusCode::OK {
                return Err(Error::Status);
            }
            return FixedOriginTransport::read_bounded_response(response, max_bytes)
                .await
                .map(Some);
        }
        Err(Error::FinalUrl)
    }
}
fn valid_version(value: &str) -> bool {
    if value.len() > 32 {
        return false;
    }
    let parts = value.split('.').collect::<Vec<_>>();
    value.len() <= 32
        && (1..=4).contains(&parts.len())
        && parts.iter().all(|part| {
            !part.is_empty()
                && part.bytes().all(|b| b.is_ascii_digit())
                && (part.len() == 1 || !part.starts_with('0'))
                && part.parse::<u32>().is_ok()
        })
}
fn request_url(id: &str, version: &str, installed: Option<&str>) -> Result<Url, Error> {
    if id.len() != 32
        || !id.bytes().all(|b| (b'a'..=b'p').contains(&b))
        || !valid_version(version)
        || installed.is_some_and(|v| !valid_version(v))
    {
        return Err(Error::Boundary);
    }
    let mut url = Url::parse("https://clients2.google.com/service/update2/crx")
        .map_err(|_| Error::Boundary)?;
    let mut item = url::form_urlencoded::Serializer::new(String::new());
    item.append_pair("id", id);
    if let Some(installed) = installed {
        item.append_pair("v", installed);
    }
    item.append_pair("uc", "");
    url.query_pairs_mut()
        .append_pair("response", "redirect")
        .append_pair("prodversion", version)
        .append_pair("acceptformat", "crx3")
        .append_pair("x", &item.finish());
    Ok(url)
}
fn package_endpoint(url: &Url) -> bool {
    url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.port().is_none()
        && url.fragment().is_none()
        && url.host_str() == Some("clients2.googleusercontent.com")
        && url.path().starts_with("/crx/blobs/")
        && url.query().is_none()
        && !url.path().contains(['%', '\\'])
        && !url.path().contains("//")
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn store_request_encodes_only_the_selected_item_and_version() {
        let url = request_url(
            "dbepggeogbaibhgnhhndojpepiihcmeb",
            "152.0.4191.66",
            Some("2.4.2"),
        )
        .unwrap();
        let pairs = url
            .query_pairs()
            .collect::<std::collections::BTreeMap<_, _>>();
        assert_eq!(pairs.len(), 4);
        assert_eq!(
            pairs["x"],
            "id=dbepggeogbaibhgnhhndojpepiihcmeb&v=2.4.2&uc="
        );
        let install = request_url("dbepggeogbaibhgnhhndojpepiihcmeb", "152", None).unwrap();
        assert_eq!(
            install.query_pairs().find(|(key, _)| key == "x").unwrap().1,
            "id=dbepggeogbaibhgnhhndojpepiihcmeb&uc="
        );
        assert!(request_url("https://attacker.invalid/", "152", None).is_err());
        assert!(request_url(
            "dbepggeogbaibhgnhhndojpepiihcmeb",
            "152",
            Some("2&client=id")
        )
        .is_err());
    }
    #[test]
    fn redirect_must_stay_in_the_exact_google_package_namespace() {
        assert!(package_endpoint(
            &Url::parse("https://clients2.googleusercontent.com/crx/blobs/object.crx").unwrap()
        ));
        for value in [
            "http://clients2.googleusercontent.com/crx/blobs/x",
            "https://clients2.googleusercontent.com.attacker.invalid/crx/blobs/x",
            "https://user@clients2.googleusercontent.com/crx/blobs/x",
            "https://clients2.googleusercontent.com:444/crx/blobs/x",
            "https://clients2.googleusercontent.com/crx/blobs/../other",
            "https://clients2.googleusercontent.com/crx/blobs/x?identity=tracking",
            "https://clients2.googleusercontent.com/crx/blobs/x#fragment",
        ] {
            assert!(!package_endpoint(&Url::parse(value).unwrap()), "{value}");
        }
    }
}
