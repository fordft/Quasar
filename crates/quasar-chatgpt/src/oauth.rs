//! The documented public-client flow; no client secret or API key is involved.
use std::time::Duration;

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use ring::{digest, signature};
use serde::Deserialize;
use serde_json::Value;
use subtle::ConstantTimeEq;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;
use url::Url;
use zeroize::Zeroizing;

use crate::{now, random, Error, Identity, Model, Profile, Tokens, ISSUER, RESOURCE};

const AUTHORIZE: &str = "https://auth.openai.com/api/accounts/authorize";
const TOKEN: &str = "https://auth.openai.com/api/accounts/oauth/token";
const JWKS: &str = "https://auth.openai.com/.well-known/jwks.json";
const DISCOVERY: &str = "https://auth.openai.com/.well-known/openid-configuration";
const INITIAL_CLIENT: &str = "dynamic_agent_client";
const SCOPES: &str =
    "openid profile email offline_access resource.invoke chatgpt.tokens.use.direct";
const MAX_BODY: usize = 512 * 1024;

/// One native browser authorization. Its URL can include an ID-token hint.
pub struct Authorization {
    pub(crate) profile: String,
    pub(crate) selected: Option<Profile>,
    pub(crate) epoch: u64,
    url: Zeroizing<String>,
    listener: TcpListener,
    redirect: String,
    state: Zeroizing<String>,
    nonce: Zeroizing<String>,
    verifier: Zeroizing<String>,
    cancel: watch::Receiver<bool>,
}

impl std::fmt::Debug for Authorization {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Authorization([redacted])")
    }
}

pub(crate) struct Callback {
    pub(crate) client_id: String,
    code: Zeroizing<String>,
}

impl Authorization {
    pub(crate) async fn start(
        host: &str,
        profile: String,
        selected: Option<Profile>,
        consent: bool,
        epoch: u64,
        cancel: watch::Receiver<bool>,
    ) -> Result<Self, Error> {
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .await
            .map_err(|_| Error::Network)?;
        let address = listener.local_addr().map_err(|_| Error::Network)?;
        let redirect = format!("http://127.0.0.1:{}/auth/callback", address.port());
        let state = Zeroizing::new(URL_SAFE_NO_PAD.encode(random::<32>()?));
        let nonce = Zeroizing::new(URL_SAFE_NO_PAD.encode(random::<32>()?));
        let verifier = Zeroizing::new(URL_SAFE_NO_PAD.encode(random::<32>()?));
        let challenge =
            URL_SAFE_NO_PAD.encode(digest::digest(&digest::SHA256, verifier.as_bytes()).as_ref());
        let mut url = Url::parse(AUTHORIZE).map_err(|_| Error::Protocol)?;
        {
            let mut query = url.query_pairs_mut();
            query
                .append_pair(
                    "client_id",
                    selected.as_ref().map_or(INITIAL_CLIENT, |p| &p.client_id),
                )
                .append_pair("ext_agent_host_id", host)
                .append_pair("response_type", "code")
                .append_pair("redirect_uri", &redirect)
                .append_pair("scope", SCOPES)
                .append_pair("resource", RESOURCE)
                .append_pair("state", &state)
                .append_pair("nonce", &nonce)
                .append_pair("code_challenge_method", "S256")
                .append_pair("code_challenge", &challenge);
            if let Some(profile) = &selected {
                if let Some(tokens) = &profile.tokens {
                    query.append_pair("id_token_hint", &tokens.id);
                }
                if let Some(email) = profile.identity.as_ref().and_then(|i| i.email.as_ref()) {
                    query.append_pair("login_hint", email);
                }
            } else {
                query.append_pair("agent_name_hint", "Quasar");
            }
            if consent {
                // force_reconsent requires a separately confirmed rollout.
                query.append_pair("prompt", "consent");
            }
        }
        Ok(Self {
            profile,
            selected,
            epoch,
            url: Zeroizing::new(url.into()),
            listener,
            redirect,
            state,
            nonce,
            verifier,
            cancel,
        })
    }

    /// Open this only in the system browser; never emit it to the renderer or logs.
    pub fn url(&self) -> &str {
        &self.url
    }

    pub(crate) fn cancellation(&self) -> watch::Receiver<bool> {
        self.cancel.clone()
    }

    pub(crate) async fn callback(&mut self) -> Result<Callback, Error> {
        let deadline = tokio::time::sleep(Duration::from_secs(5 * 60));
        tokio::pin!(deadline);
        loop {
            if *self.cancel.borrow() {
                return Err(Error::Cancelled);
            }
            let (mut stream, peer) = tokio::select! {
                biased;
                _ = self.cancel.changed() => return Err(Error::Cancelled),
                _ = &mut deadline => return Err(Error::TimedOut),
                incoming = self.listener.accept() => incoming.map_err(|_| Error::Network)?,
            };
            if !peer.ip().is_loopback() {
                continue;
            }
            let request = tokio::select! {
                biased;
                _ = self.cancel.changed() => return Err(Error::Cancelled),
                _ = &mut deadline => return Err(Error::TimedOut),
                request = tokio::time::timeout(Duration::from_secs(5), read_request(&mut stream)) =>
                    request.map_err(|_| Error::InvalidCallback).and_then(|v| v),
            };
            let parsed = request.and_then(|request| self.parse_callback(&request));
            let valid_state = !matches!(&parsed, Err(Error::InvalidCallback));
            let message = if parsed.is_ok() {
                "You can return to Quasar."
            } else {
                "Sign-in was not completed. Return to Quasar to try again."
            };
            let status = if parsed.is_ok() {
                "200 OK"
            } else {
                "400 Bad Request"
            };
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: text/plain; charset=utf-8\r\n\
                 Cache-Control: no-store\r\nReferrer-Policy: no-referrer\r\n\
                 Content-Security-Policy: default-src 'none'; frame-ancestors 'none'\r\n\
                 Connection: close\r\nContent-Length: {}\r\n\r\n{message}",
                message.len()
            );
            let _ = tokio::time::timeout(
                Duration::from_secs(2),
                stream.write_all(response.as_bytes()),
            )
            .await;
            if valid_state {
                return parsed;
            }
        }
    }

    fn parse_callback(&self, request: &str) -> Result<Callback, Error> {
        let mut lines = request.split("\r\n");
        let first = lines.next().ok_or(Error::InvalidCallback)?;
        let mut words = first.split_whitespace();
        if words.next() != Some("GET") {
            return Err(Error::InvalidCallback);
        }
        let target = words.next().ok_or(Error::InvalidCallback)?;
        if words.next() != Some("HTTP/1.1") || words.next().is_some() {
            return Err(Error::InvalidCallback);
        }
        let expected_host = self
            .redirect
            .strip_prefix("http://")
            .and_then(|v| v.split('/').next())
            .ok_or(Error::Protocol)?;
        let mut hosts = lines
            .filter_map(|line| line.split_once(':'))
            .filter(|(name, _)| name.eq_ignore_ascii_case("host"));
        if hosts.next().map(|(_, value)| value.trim()) != Some(expected_host)
            || hosts.next().is_some()
        {
            return Err(Error::InvalidCallback);
        }
        let (path, query) = target.split_once('?').ok_or(Error::InvalidCallback)?;
        if path != "/auth/callback" || target.len() > 12 * 1024 {
            return Err(Error::InvalidCallback);
        }
        let mut values = std::collections::HashMap::new();
        for (key, value) in url::form_urlencoded::parse(query.as_bytes()) {
            if values
                .insert(key.into_owned(), value.into_owned())
                .is_some()
            {
                return Err(Error::InvalidCallback);
            }
        }
        let state = values.get("state").ok_or(Error::InvalidCallback)?;
        if !bool::from(state.as_bytes().ct_eq(self.state.as_bytes())) {
            return Err(Error::InvalidCallback);
        }
        if values.contains_key("error") {
            return Err(Error::Denied);
        }
        let client = match (&self.selected, values.get("client_id")) {
            (Some(selected), Some(returned)) if returned != &selected.client_id => {
                return Err(Error::Protocol)
            }
            (Some(selected), _) => selected.client_id.clone(),
            (None, Some(returned)) if valid_client(returned) => returned.clone(),
            _ => return Err(Error::Protocol),
        };
        let code = values
            .remove("code")
            .filter(|v| !v.is_empty() && v.len() <= 4096)
            .ok_or(Error::Protocol)?;
        Ok(Callback {
            client_id: client,
            code: Zeroizing::new(code),
        })
    }
}

fn valid_client(value: &str) -> bool {
    value != INITIAL_CLIENT
        && !value.is_empty()
        && value.len() <= 256
        && value.bytes().all(|b| matches!(b, 0x21..=0x7e))
}

async fn read_request(stream: &mut TcpStream) -> Result<Zeroizing<String>, Error> {
    let mut bytes = Zeroizing::new(Vec::new());
    let mut chunk = [0u8; 1024];
    loop {
        let read = stream
            .read(&mut chunk)
            .await
            .map_err(|_| Error::InvalidCallback)?;
        if read == 0 {
            return Err(Error::InvalidCallback);
        }
        bytes.extend_from_slice(&chunk[..read]);
        if bytes.len() > 16 * 1024 {
            return Err(Error::InvalidCallback);
        }
        if bytes.windows(4).any(|v| v == b"\r\n\r\n") {
            return String::from_utf8(bytes.to_vec())
                .map(Zeroizing::new)
                .map_err(|_| Error::InvalidCallback);
        }
    }
}

#[derive(Clone)]
pub(crate) struct Client {
    http: reqwest::Client,
    #[cfg(test)]
    fixture: Option<String>,
}

#[derive(Deserialize)]
struct TokenReply {
    access_token: String,
    refresh_token: Option<String>,
    id_token: Option<String>,
    token_type: String,
    expires_in: u64,
    scope: Option<String>,
    earliest_refresh_at: Option<u64>,
}

impl Client {
    pub(crate) fn new() -> Result<Self, Error> {
        let http = reqwest::Client::builder()
            .https_only(true)
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .user_agent("Quasar-ChatGPT/1")
            .build()
            .map_err(|_| Error::Network)?;
        Ok(Self {
            http,
            #[cfg(test)]
            fixture: None,
        })
    }

    fn endpoint(&self, url: &str) -> String {
        #[cfg(test)]
        if let Some(base) = &self.fixture {
            let path = Url::parse(url)
                .expect("fixed test endpoint")
                .path()
                .to_owned();
            return format!("{base}{path}");
        }
        url.to_owned()
    }

    #[cfg(test)]
    pub(crate) fn fixture(base: String) -> Self {
        Self {
            http: reqwest::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_secs(2))
                .build()
                .expect("fixture client"),
            fixture: Some(base),
        }
    }

    pub(crate) async fn exchange(
        &self,
        auth: &Authorization,
        callback: &Callback,
    ) -> Result<(Identity, Tokens), Error> {
        let form = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("grant_type", "authorization_code")
            .append_pair("client_id", &callback.client_id)
            .append_pair("code", &callback.code)
            .append_pair("code_verifier", &auth.verifier)
            .append_pair("redirect_uri", &auth.redirect)
            .append_pair("resource", RESOURCE)
            .finish();
        let reply = self.token(form).await?;
        let id = reply.id_token.as_ref().ok_or(Error::InvalidIdentity)?;
        let identity = self
            .verify(id, &callback.client_id, Some(&auth.nonce))
            .await?;
        if auth
            .selected
            .as_ref()
            .and_then(|p| p.identity.as_ref())
            .is_some_and(|expected| {
                expected.issuer != identity.issuer || expected.subject != identity.subject
            })
        {
            return Err(Error::InvalidIdentity);
        }
        Ok((identity, tokens(reply, None)?))
    }

    pub(crate) async fn refresh(&self, profile: &Profile) -> Result<Tokens, Error> {
        let old = profile.tokens.as_ref().ok_or(Error::SignedOut)?;
        let refresh = old.refresh.as_ref().ok_or(Error::SignedOut)?;
        let form = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("grant_type", "refresh_token")
            .append_pair("client_id", &profile.client_id)
            .append_pair("refresh_token", refresh)
            .append_pair("resource", RESOURCE)
            .finish();
        let reply = self.token(form).await?;
        if let Some(id) = &reply.id_token {
            let identity = self.verify(id, &profile.client_id, None).await?;
            if profile
                .identity
                .as_ref()
                .is_none_or(|i| i.issuer != identity.issuer || i.subject != identity.subject)
            {
                return Err(Error::InvalidIdentity);
            }
        }
        tokens(reply, Some(old))
    }

    async fn token(&self, form: String) -> Result<TokenReply, Error> {
        let form = Zeroizing::new(form);
        let response = self
            .http
            .post(self.endpoint(TOKEN))
            .header(
                reqwest::header::CONTENT_TYPE,
                "application/x-www-form-urlencoded",
            )
            .body(form.to_string())
            .send()
            .await
            .map_err(|_| Error::Network)?;
        let status = response.status();
        let body = bounded(response).await?;
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(Error::RateLimited);
        }
        if !status.is_success() {
            let code = serde_json::from_slice::<Value>(&body)
                .ok()
                .and_then(|v| v.get("error").and_then(Value::as_str).map(str::to_owned));
            return Err(if code.as_deref() == Some("invalid_grant") {
                Error::SignedOut
            } else if status.is_server_error() {
                Error::Network
            } else {
                Error::Protocol
            });
        }
        serde_json::from_slice(&body).map_err(|_| Error::Protocol)
    }

    async fn verify(
        &self,
        token: &str,
        client: &str,
        nonce: Option<&str>,
    ) -> Result<Identity, Error> {
        let response = self
            .http
            .get(self.endpoint(JWKS))
            .send()
            .await
            .map_err(|_| Error::Network)?;
        if !response.status().is_success() {
            return Err(Error::Network);
        }
        let keys: Value = serde_json::from_slice(&bounded(response).await?)
            .map_err(|_| Error::InvalidIdentity)?;
        let time = now()?;
        #[cfg(test)]
        let time = if self.fixture.is_some() {
            1800000030
        } else {
            time
        };
        verify_identity(token, client, nonce, &keys, time)
    }

    pub(crate) async fn models(&self, token: &crate::Secret) -> Result<Vec<Model>, Error> {
        let response = self
            .http
            .get(self.endpoint(&format!("{RESOURCE}/models")))
            .bearer_auth(token.expose())
            .send()
            .await
            .map_err(|_| Error::Network)?;
        let status = response.status();
        if status == reqwest::StatusCode::UNAUTHORIZED {
            return Err(Error::SignedOut);
        }
        if status == reqwest::StatusCode::FORBIDDEN {
            return Err(Error::PlanPermissionRequired);
        }
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(Error::UsageLimit);
        }
        if !status.is_success() {
            return Err(Error::Network);
        }
        let body: Value =
            serde_json::from_slice(&bounded(response).await?).map_err(|_| Error::Protocol)?;
        let rows = body
            .get("models")
            .and_then(Value::as_array)
            .ok_or(Error::Protocol)?;
        if rows.len() > 512 {
            return Err(Error::Protocol);
        }
        let mut seen = std::collections::HashSet::new();
        Ok(rows
            .iter()
            .filter(|row| row.get("visibility").and_then(Value::as_str) == Some("list"))
            .filter_map(|row| {
                let slug = row.get("slug")?.as_str()?;
                let name = row.get("display_name")?.as_str()?;
                if !text(slug, 128) || !text(name, 256) || !seen.insert(slug.to_owned()) {
                    return None;
                }
                Some(Model {
                    slug: slug.to_owned(),
                    display_name: name.to_owned(),
                })
            })
            .collect())
    }

    pub(crate) async fn revoke(&self, profile: &Profile) -> bool {
        let Some(refresh) = profile.tokens.as_ref().and_then(|t| t.refresh.as_ref()) else {
            return true;
        };
        let Ok(response) = self.http.get(self.endpoint(DISCOVERY)).send().await else {
            return false;
        };
        if !response.status().is_success() {
            return false;
        }
        let Ok(body) = bounded(response).await else {
            return false;
        };
        let Ok(discovery) = serde_json::from_slice::<Value>(&body) else {
            return false;
        };
        if discovery.get("issuer").and_then(Value::as_str) != Some(ISSUER) {
            return false;
        }
        let Some(endpoint) = discovery.get("revocation_endpoint").and_then(Value::as_str) else {
            return false;
        };
        let Ok(url) = Url::parse(endpoint) else {
            return false;
        };
        if url.origin() != Url::parse(ISSUER).expect("static issuer").origin()
            || url.username() != ""
            || url.password().is_some()
            || url.fragment().is_some()
        {
            return false;
        }
        for delay in [0, 300, 1000] {
            tokio::time::sleep(Duration::from_millis(delay)).await;
            let form = Zeroizing::new(
                url::form_urlencoded::Serializer::new(String::new())
                    .append_pair("token", refresh)
                    .append_pair("token_type_hint", "refresh_token")
                    .append_pair("client_id", &profile.client_id)
                    .finish(),
            );
            match self
                .http
                .post(self.endpoint(url.as_str()))
                .header(
                    reqwest::header::CONTENT_TYPE,
                    "application/x-www-form-urlencoded",
                )
                .body(form.to_string())
                .send()
                .await
            {
                Ok(response) if response.status() == reqwest::StatusCode::OK => return true,
                Ok(response) if !response.status().is_server_error() => return false,
                _ => {}
            }
        }
        false
    }
}

async fn bounded(mut response: reqwest::Response) -> Result<Zeroizing<Vec<u8>>, Error> {
    if response
        .content_length()
        .is_some_and(|n| n > MAX_BODY as u64)
    {
        return Err(Error::Protocol);
    }
    let mut bytes = Zeroizing::new(Vec::new());
    while let Some(chunk) = response.chunk().await.map_err(|_| Error::Network)? {
        if bytes.len().saturating_add(chunk.len()) > MAX_BODY {
            return Err(Error::Protocol);
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

fn tokens(reply: TokenReply, old: Option<&Tokens>) -> Result<Tokens, Error> {
    if !reply.token_type.eq_ignore_ascii_case("bearer")
        || reply.expires_in == 0
        || reply.expires_in > 24 * 60 * 60
    {
        return Err(Error::Protocol);
    }
    let access = crate::Secret::new(reply.access_token)?;
    let refresh = match reply.refresh_token {
        Some(token) => Some(crate::Secret::new(token)?.0),
        None => old.and_then(|t| t.refresh.clone()),
    };
    let id = match reply.id_token {
        Some(token) => crate::Secret::new(token)?.0,
        None => old.map(|t| t.id.clone()).ok_or(Error::InvalidIdentity)?,
    };
    let scopes = match reply.scope {
        Some(scope) if scope.len() <= 4096 => {
            scope.split_ascii_whitespace().map(str::to_owned).collect()
        }
        Some(_) => return Err(Error::Protocol),
        None => old.map_or_else(Vec::new, |t| t.scopes.clone()),
    };
    Ok(Tokens {
        access: access.0,
        refresh,
        id,
        scopes,
        expires_at: now()?
            .checked_add(reply.expires_in)
            .ok_or(Error::Protocol)?,
        earliest_refresh_at: reply.earliest_refresh_at,
    })
}

fn text(value: &str, max: usize) -> bool {
    !value.is_empty() && value.len() <= max && !value.chars().any(char::is_control)
}

#[derive(Deserialize)]
struct Claims {
    iss: String,
    sub: String,
    aud: Value,
    exp: u64,
    iat: Option<u64>,
    nbf: Option<u64>,
    nonce: Option<String>,
    azp: Option<String>,
    email: Option<String>,
    email_verified: Option<bool>,
    name: Option<String>,
}

fn verify_identity(
    token: &str,
    client: &str,
    nonce: Option<&str>,
    jwks: &Value,
    time: u64,
) -> Result<Identity, Error> {
    if token.len() > crate::MAX_TOKEN_BYTES || !valid_client(client) {
        return Err(Error::InvalidIdentity);
    }
    let parts: Vec<_> = token.split('.').collect();
    if parts.len() != 3 {
        return Err(Error::InvalidIdentity);
    }
    let header: Value = serde_json::from_slice(
        &URL_SAFE_NO_PAD
            .decode(parts[0])
            .map_err(|_| Error::InvalidIdentity)?,
    )
    .map_err(|_| Error::InvalidIdentity)?;
    if header.get("alg").and_then(Value::as_str) != Some("RS256")
        || header
            .get("crit")
            .is_some_and(|v| v.as_array().is_none_or(|a| !a.is_empty()))
    {
        return Err(Error::InvalidIdentity);
    }
    let kid = header
        .get("kid")
        .and_then(Value::as_str)
        .filter(|s| text(s, 256))
        .ok_or(Error::InvalidIdentity)?;
    let keys = jwks
        .get("keys")
        .and_then(Value::as_array)
        .filter(|k| k.len() <= 64)
        .ok_or(Error::InvalidIdentity)?;
    let mut candidates = keys.iter().filter(|k| {
        k.get("kid").and_then(Value::as_str) == Some(kid)
            && k.get("kty").and_then(Value::as_str) == Some("RSA")
    });
    let key = candidates.next().ok_or(Error::InvalidIdentity)?;
    if candidates.next().is_some()
        || key.get("alg").is_some_and(|v| v.as_str() != Some("RS256"))
        || key.get("use").is_some_and(|v| v.as_str() != Some("sig"))
    {
        return Err(Error::InvalidIdentity);
    }
    let n = URL_SAFE_NO_PAD
        .decode(
            key.get("n")
                .and_then(Value::as_str)
                .ok_or(Error::InvalidIdentity)?,
        )
        .map_err(|_| Error::InvalidIdentity)?;
    let e = URL_SAFE_NO_PAD
        .decode(
            key.get("e")
                .and_then(Value::as_str)
                .ok_or(Error::InvalidIdentity)?,
        )
        .map_err(|_| Error::InvalidIdentity)?;
    let sig = URL_SAFE_NO_PAD
        .decode(parts[2])
        .map_err(|_| Error::InvalidIdentity)?;
    let message = format!("{}.{}", parts[0], parts[1]);
    signature::RsaPublicKeyComponents { n: &n, e: &e }
        .verify(
            &signature::RSA_PKCS1_2048_8192_SHA256,
            message.as_bytes(),
            &sig,
        )
        .map_err(|_| Error::InvalidIdentity)?;
    let claims: Claims = serde_json::from_slice(
        &URL_SAFE_NO_PAD
            .decode(parts[1])
            .map_err(|_| Error::InvalidIdentity)?,
    )
    .map_err(|_| Error::InvalidIdentity)?;
    let audiences: Vec<&str> = match &claims.aud {
        Value::String(aud) => vec![aud],
        Value::Array(auds) if auds.len() <= 8 => auds
            .iter()
            .map(|v| v.as_str().ok_or(Error::InvalidIdentity))
            .collect::<Result<_, _>>()?,
        _ => return Err(Error::InvalidIdentity),
    };
    if claims.iss != ISSUER
        || !text(&claims.sub, 512)
        || !audiences.contains(&client)
        || (audiences.len() > 1 && claims.azp.as_deref() != Some(client))
        || claims.exp <= time
        || claims.nbf.is_some_and(|t| t > time.saturating_add(60))
        || claims.iat.is_some_and(|t| t > time.saturating_add(60))
        || nonce.is_some_and(|expected| {
            claims
                .nonce
                .as_deref()
                .is_none_or(|found| !bool::from(found.as_bytes().ct_eq(expected.as_bytes())))
        })
    {
        return Err(Error::InvalidIdentity);
    }
    Ok(Identity {
        issuer: claims.iss,
        subject: claims.sub,
        email: claims
            .email
            .filter(|v| text(v, 320) && claims.email_verified != Some(false)),
        name: claims.name.filter(|v| text(v, 256)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_token_requires_signature_issuer_audience_expiration_and_login_nonce() {
        let fixture: Value =
            serde_json::from_str(include_str!("oidc-test-vector.json")).expect("fixture");
        let token = fixture["token"].as_str().expect("token");
        let keys = &fixture["jwks"];
        let client = "oaiapp_quasar_unit_test";
        let valid = verify_identity(token, client, Some("quasar-test-nonce"), keys, 1800000030)
            .expect("signed identity");
        assert_eq!(valid.subject, "test-subject");
        for (audience, nonce, time) in [
            ("another_client", "quasar-test-nonce", 1800000030),
            (client, "wrong-nonce", 1800000030),
            (client, "quasar-test-nonce", 1800003601),
            (client, "quasar-test-nonce", 1700000000),
        ] {
            assert_eq!(
                verify_identity(token, audience, Some(nonce), keys, time),
                Err(Error::InvalidIdentity)
            );
        }
        let mut parts: Vec<_> = token.split('.').map(str::to_owned).collect();
        let mut claims: Value =
            serde_json::from_slice(&URL_SAFE_NO_PAD.decode(&parts[1]).expect("body"))
                .expect("claims");
        claims["sub"] = Value::String("attacker".into());
        parts[1] = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims).expect("json"));
        assert_eq!(
            verify_identity(
                &parts.join("."),
                client,
                Some("quasar-test-nonce"),
                keys,
                1800000030
            ),
            Err(Error::InvalidIdentity)
        );
        assert_eq!(
            verify_identity(
                token,
                client,
                Some("quasar-test-nonce"),
                &serde_json::json!({"keys":[]}),
                1800000030
            ),
            Err(Error::InvalidIdentity)
        );
    }

    #[tokio::test]
    async fn callback_binds_state_host_path_and_the_exact_issued_registration() {
        let (_cancel, receiver) = watch::channel(false);
        let auth =
            Authorization::start("urn:uuid:unit", "profile".into(), None, false, 1, receiver)
                .await
                .expect("start");
        let host = auth
            .redirect
            .trim_start_matches("http://")
            .split('/')
            .next()
            .expect("host");
        let request =
            |query: &str| format!("GET /auth/callback?{query} HTTP/1.1\r\nHost: {host}\r\n\r\n");
        assert_eq!(
            auth.parse_callback(&request("state=wrong&code=test&client_id=oaiapp_test"))
                .err(),
            Some(Error::InvalidCallback)
        );
        assert_eq!(
            auth.parse_callback(&request(&format!(
                "state={}&error=access_denied",
                &*auth.state
            )))
            .err(),
            Some(Error::Denied)
        );
        assert_eq!(
            auth.parse_callback(&request(&format!(
                "state={}&code=test&client_id=dynamic_agent_client",
                &*auth.state
            )))
            .err(),
            Some(Error::Protocol)
        );
        let callback = auth
            .parse_callback(&request(&format!(
                "state={}&code=test&client_id=oaiapp_test",
                &*auth.state
            )))
            .expect("callback");
        assert_eq!(callback.client_id, "oaiapp_test");
        assert_eq!(&*callback.code, "test");
        let wrong_host = request(&format!(
            "state={}&code=test&client_id=oaiapp_test",
            &*auth.state
        ))
        .replace(&format!("Host: {host}"), "Host: localhost");
        assert_eq!(
            auth.parse_callback(&wrong_host).err(),
            Some(Error::InvalidCallback)
        );
    }

    #[test]
    fn granted_scopes_come_from_tokens_not_from_callback_claims() {
        let value = TokenReply {
            access_token: "test-token".into(),
            refresh_token: None,
            id_token: Some("test-id-token".into()),
            token_type: "Bearer".into(),
            expires_in: 3600,
            scope: Some("openid email profile".into()),
            earliest_refresh_at: None,
        };
        assert!(!tokens(value, None).expect("identity tokens").sharing());
    }
    #[tokio::test]
    async fn exchange_uses_the_issued_client_pkce_redirect_and_resource() {
        let vector: Value =
            serde_json::from_str(include_str!("oidc-test-vector.json")).expect("vector");
        let server = crate::fixtures::Server::start(vec![
            (200, serde_json::json!({ "access_token": "test-access", "refresh_token": "test-refresh",
                "id_token": vector["token"], "expires_in": 3600, "token_type": "Bearer", "scope": SCOPES })),
            (200, vector["jwks"].clone()),
        ]).await;
        let (_sender, receiver) = watch::channel(false);
        let mut auth = Authorization::start(
            "urn:uuid:12345678-1234-4234-8234-123456789012",
            "one".into(),
            None,
            false,
            1,
            receiver,
        )
        .await
        .expect("start");
        let query: std::collections::HashMap<_, _> = Url::parse(auth.url())
            .expect("url")
            .query_pairs()
            .into_owned()
            .collect();
        assert_eq!(
            query.get("code_challenge"),
            Some(
                &URL_SAFE_NO_PAD
                    .encode(digest::digest(&digest::SHA256, auth.verifier.as_bytes()).as_ref())
            )
        );
        // The public signed vector has a fixed nonce; signature/nonce rejection
        // itself is independently exercised above. No private signing key exists here.
        auth.nonce = Zeroizing::new("quasar-test-nonce".into());
        let callback = Callback {
            client_id: "oaiapp_quasar_unit_test".into(),
            code: Zeroizing::new("test-code".into()),
        };
        let (identity, tokens) = Client::fixture(server.base.clone())
            .exchange(&auth, &callback)
            .await
            .expect("exchange");
        assert_eq!(identity.subject, "test-subject");
        assert!(tokens.sharing());
        let requests = server.requests.lock().expect("requests");
        let fields: std::collections::HashMap<_, _> = url::form_urlencoded::parse(
            requests[0]
                .split("\r\n\r\n")
                .nth(1)
                .expect("body")
                .as_bytes(),
        )
        .into_owned()
        .collect();
        assert_eq!(
            fields.get("client_id").map(String::as_str),
            Some("oaiapp_quasar_unit_test")
        );
        assert_eq!(
            fields.get("code_verifier"),
            Some(&auth.verifier.to_string())
        );
        assert_eq!(fields.get("redirect_uri"), Some(&auth.redirect));
        assert_eq!(fields.get("resource").map(String::as_str), Some(RESOURCE));
        assert!(!fields.contains_key("client_secret"));
    }
}
