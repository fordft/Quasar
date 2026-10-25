//! Loopback-only, synthetic password-manager WebAuthn workflow gate.
//!
//! This developer tool deliberately lives outside product code. It creates no
//! browser authority, stores no credential material, contacts no remote host,
//! and accepts no caller-selected page content. A successful run proves that a
//! selected credential provider completed registration and that the resulting
//! P-256 credential produced a valid, user-verified assertion for the exact
//! one-use challenge and loopback origin.

use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddrV4, TcpListener, TcpStream};
use std::thread;
use std::time::{Duration, Instant};

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use ring::rand::{SecureRandom as _, SystemRandom};
use ring::signature::{UnparsedPublicKey, ECDSA_P256_SHA256_ASN1};
use serde::Deserialize;
use serde_json::json;
use sha2::{Digest as _, Sha256};

const SERVER_LIFETIME: Duration = Duration::from_secs(10 * 60);
const ACCEPT_POLL: Duration = Duration::from_millis(5);
const IO_TIMEOUT: Duration = Duration::from_secs(3);
const MAX_REQUESTS: usize = 64;
const MAX_HEADER_BYTES: usize = 16 * 1024;
const MAX_BODY_BYTES: usize = 64 * 1024;
const CHALLENGE_BYTES: usize = 32;
const SESSION_TOKEN_BYTES: usize = 16;
const USER_ID_BYTES: usize = 16;
const MAX_CREDENTIAL_ID_BYTES: usize = 1024;
const MAX_CLIENT_DATA_BYTES: usize = 8 * 1024;
const MAX_ATTESTATION_BYTES: usize = 64 * 1024;
const MAX_AUTHENTICATOR_DATA_BYTES: usize = 8 * 1024;
const MAX_SIGNATURE_BYTES: usize = 256;
const P256_SPKI_PREFIX: &[u8] = &[
    0x30, 0x59, 0x30, 0x13, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01, 0x06, 0x08, 0x2a,
    0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07, 0x03, 0x42, 0x00,
];

const PAGE: &[u8] = br##"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Quasar password-manager WebAuthn QA</title>
  <link rel="stylesheet" href="./app.css">
</head>
<body>
  <main>
    <p class="eyebrow">Local development gate</p>
    <h1>Password-manager passkey assertion</h1>
    <p>This page creates one synthetic localhost passkey, then verifies a real assertion. Select only the fresh development vault. No real account is used or transmitted.</p>
    <ol>
      <li><button id="register" type="button">1. Create synthetic passkey</button></li>
      <li><button id="assert" type="button" disabled>2. Verify passkey assertion</button></li>
    </ol>
    <output id="status" role="status" aria-live="polite">Ready.</output>
    <details>
      <summary>Security boundary</summary>
      <p>Loopback only, one-use challenges, exact-origin POSTs, no CORS, no remote assets, and no password or vault enumeration.</p>
    </details>
  </main>
  <script src="./app.js" defer></script>
</body>
</html>
"##;

const STYLE: &[u8] = br#":root{color-scheme:light dark;font:16px/1.5 system-ui,sans-serif;background:#111;color:#f5f5f7}body{margin:0;min-height:100vh;display:grid;place-items:center}main{width:min(42rem,calc(100% - 3rem));padding:2.5rem;border:1px solid #45454d;border-radius:1.5rem;background:#202024;box-shadow:0 1.5rem 5rem #0008}h1{font-size:clamp(1.7rem,5vw,2.5rem);line-height:1.1;margin:.2rem 0 1rem}.eyebrow{color:#9a9aa3;text-transform:uppercase;letter-spacing:.09em;font-size:.75rem;font-weight:700}ol{padding-left:1.5rem;margin:2rem 0}li+li{margin-top:1rem}button{font:inherit;font-weight:650;padding:.8rem 1rem;border:0;border-radius:.8rem;background:#6d5dfc;color:white;cursor:pointer}button:disabled{opacity:.45;cursor:not-allowed}output{display:block;min-height:3rem;padding:1rem;border-radius:.8rem;background:#151518;white-space:pre-wrap}output[data-state=passed]{color:#7ee787}output[data-state=failed]{color:#ff7b72}details{margin-top:1.5rem;color:#b5b5bd}@media(prefers-color-scheme:light){:root{background:#f2f2f7;color:#1d1d1f}main{background:white;border-color:#d2d2d7;box-shadow:0 1.5rem 5rem #0002}output{background:#f2f2f7}details{color:#515154}}"#;

const SCRIPT: &[u8] = br##"(() => {
  "use strict";
  const register = document.querySelector("#register");
  const assert = document.querySelector("#assert");
  const status = document.querySelector("#status");
  const base = location.pathname.replace(/\/$/, "");

  const decode = (value) => {
    const padded = value.replace(/-/g, "+").replace(/_/g, "/") + "===".slice((value.length + 3) % 4);
    const bytes = Uint8Array.from(atob(padded), (character) => character.charCodeAt(0));
    return bytes.buffer;
  };
  const encode = (value) => {
    const bytes = new Uint8Array(value);
    let binary = "";
    for (const byte of bytes) binary += String.fromCharCode(byte);
    return btoa(binary).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
  };
  const request = async (suffix, options = {}) => {
    const response = await fetch(`${base}/${suffix}`, {
      cache: "no-store",
      credentials: "same-origin",
      ...options,
      headers: { "Content-Type": "application/json", ...(options.headers ?? {}) },
    });
    const body = await response.json();
    if (!response.ok) throw new Error(body.error ?? `HTTP ${response.status}`);
    return body;
  };
  const fail = async (phase, error) => {
    const message = error instanceof Error ? `${error.name}: ${error.message}` : String(error);
    status.dataset.state = "failed";
    status.textContent = `${phase} failed. ${message}`;
    try {
      await request("client-error", { method: "POST", body: JSON.stringify({ phase, message: message.slice(0, 512) }) });
    } catch {}
  };

  register.addEventListener("click", async () => {
    register.disabled = true;
    status.dataset.state = "";
    status.textContent = "Waiting for the selected password manager to create the synthetic passkey...";
    try {
      if (typeof PublicKeyCredential !== "function") throw new Error("WebAuthn is unavailable");
      const options = await request("register-options");
      options.publicKey.challenge = decode(options.publicKey.challenge);
      options.publicKey.user.id = decode(options.publicKey.user.id);
      const credential = await navigator.credentials.create(options);
      if (!(credential instanceof PublicKeyCredential)) throw new Error("No public-key credential returned");
      const publicKey = credential.response.getPublicKey?.();
      const algorithm = credential.response.getPublicKeyAlgorithm?.();
      if (!(publicKey instanceof ArrayBuffer) || algorithm !== -7) {
        throw new Error("The provider did not return an ES256 public key");
      }
      await request("register-result", {
        method: "POST",
        body: JSON.stringify({
          credential_id: credential.id,
          raw_id: encode(credential.rawId),
          client_data_json: encode(credential.response.clientDataJSON),
          attestation_object: encode(credential.response.attestationObject),
          public_key_spki: encode(publicKey),
          public_key_algorithm: algorithm,
        }),
      });
      assert.disabled = false;
      status.textContent = "Registration verified. Continue with the assertion.";
    } catch (error) {
      register.disabled = false;
      await fail("Registration", error);
    }
  });

  assert.addEventListener("click", async () => {
    assert.disabled = true;
    status.dataset.state = "";
    status.textContent = "Waiting for the selected password manager to authorize the assertion...";
    try {
      const options = await request("assert-options");
      options.publicKey.challenge = decode(options.publicKey.challenge);
      options.publicKey.allowCredentials = options.publicKey.allowCredentials.map((entry) => ({ ...entry, id: decode(entry.id) }));
      const credential = await navigator.credentials.get(options);
      if (!(credential instanceof PublicKeyCredential)) throw new Error("No public-key assertion returned");
      const result = await request("assert-result", {
        method: "POST",
        body: JSON.stringify({
          credential_id: credential.id,
          raw_id: encode(credential.rawId),
          authenticator_data: encode(credential.response.authenticatorData),
          client_data_json: encode(credential.response.clientDataJSON),
          signature: encode(credential.response.signature),
          user_handle: credential.response.userHandle == null ? null : encode(credential.response.userHandle),
        }),
      });
      status.dataset.state = "passed";
      status.textContent = result.message;
    } catch (error) {
      assert.disabled = false;
      await fail("Assertion", error);
    }
  });
})();
"##;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RegistrationResult {
    credential_id: String,
    raw_id: String,
    client_data_json: String,
    attestation_object: String,
    public_key_spki: String,
    public_key_algorithm: i32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AssertionResult {
    credential_id: String,
    raw_id: String,
    authenticator_data: String,
    client_data_json: String,
    signature: String,
    user_handle: Option<String>,
}

#[derive(Deserialize)]
struct ClientData {
    #[serde(rename = "type")]
    kind: String,
    challenge: String,
    origin: String,
    #[serde(rename = "crossOrigin", default)]
    cross_origin: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ClientError {
    phase: String,
    message: String,
}

struct Registration {
    credential_id: Vec<u8>,
    public_key: [u8; 65],
}

struct State {
    origin: String,
    host: String,
    session_path: String,
    registration_challenge: [u8; CHALLENGE_BYTES],
    assertion_challenge: [u8; CHALLENGE_BYTES],
    user_id: [u8; USER_ID_BYTES],
    registration: Option<Registration>,
    assertion_passed: bool,
}

struct Request {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Request {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(candidate, _)| candidate.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

pub(crate) fn run(arguments: &[String]) -> Result<(), String> {
    let requested_port = parse_arguments(arguments)?;
    let listener = TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, requested_port))
        .map_err(|error| format!("cannot bind the loopback QA server: {error}"))?;
    listener
        .set_nonblocking(true)
        .map_err(|error| format!("cannot bound loopback acceptance: {error}"))?;
    let port = listener
        .local_addr()
        .map_err(|error| format!("cannot inspect the loopback listener: {error}"))?
        .port();
    let random = SystemRandom::new();
    let mut session_token = [0_u8; SESSION_TOKEN_BYTES];
    let mut registration_challenge = [0_u8; CHALLENGE_BYTES];
    let mut assertion_challenge = [0_u8; CHALLENGE_BYTES];
    let mut user_id = [0_u8; USER_ID_BYTES];
    for output in [
        session_token.as_mut_slice(),
        registration_challenge.as_mut_slice(),
        assertion_challenge.as_mut_slice(),
        user_id.as_mut_slice(),
    ] {
        random
            .fill(output)
            .map_err(|_| "cannot generate WebAuthn QA entropy".to_owned())?;
    }
    let session_path = format!("/qa/{}", URL_SAFE_NO_PAD.encode(session_token));
    let origin = format!("http://localhost:{port}");
    let host = format!("localhost:{port}");
    let mut state = State {
        origin: origin.clone(),
        host,
        session_path: session_path.clone(),
        registration_challenge,
        assertion_challenge,
        user_id,
        registration: None,
        assertion_passed: false,
    };
    println!("password-manager WebAuthn QA ready: {origin}{session_path}/");
    println!("Select only the fresh development vault; no Apple Passwords item is required.");

    let deadline = Instant::now()
        .checked_add(SERVER_LIFETIME)
        .ok_or_else(|| "QA server deadline overflowed".to_owned())?;
    let mut requests = 0_usize;
    while Instant::now() < deadline && requests < MAX_REQUESTS && !state.assertion_passed {
        match listener.accept() {
            Ok((mut stream, peer)) => {
                requests += 1;
                if !peer.ip().is_loopback() {
                    continue;
                }
                stream
                    // Some platforms inherit O_NONBLOCK from the accepting
                    // listener. Restore blocking I/O before applying the hard
                    // per-request deadline so a browser preconnect cannot turn
                    // an ordinary first read into a spurious EAGAIN failure.
                    .set_nonblocking(false)
                    .and_then(|()| stream.set_read_timeout(Some(IO_TIMEOUT)))
                    .and_then(|()| stream.set_write_timeout(Some(IO_TIMEOUT)))
                    .map_err(|error| format!("cannot bound loopback I/O: {error}"))?;
                if let Err(error) = handle(&mut stream, &mut state) {
                    eprintln!("password-manager WebAuthn QA rejected a loopback request: {error}");
                    let _ = respond_json(&mut stream, 400, &json!({ "error": error }).to_string());
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(ACCEPT_POLL);
            }
            Err(error) => return Err(format!("loopback acceptance failed: {error}")),
        }
    }
    if !state.assertion_passed {
        return Err(if requests >= MAX_REQUESTS {
            "the bounded request budget expired before a valid assertion".to_owned()
        } else {
            "the ten-minute user-interaction window expired before a valid assertion".to_owned()
        });
    }
    println!(
        "password-manager WebAuthn QA passed: registration=validated; assertion_signature=validated; user_presence=passed; user_verification=passed; rp_id=localhost; remote_network=false; product_authority=false"
    );
    Ok(())
}

fn parse_arguments(arguments: &[String]) -> Result<u16, String> {
    match arguments {
        [] => Ok(0),
        [flag, value] if flag == "--port" => {
            let port = value
                .parse::<u16>()
                .map_err(|_| "WebAuthn QA port must be a non-zero u16".to_owned())?;
            if port == 0 {
                return Err("WebAuthn QA port must be non-zero".into());
            }
            Ok(port)
        }
        _ => Err("expected no arguments or exactly `--port PORT`".into()),
    }
}

fn handle(stream: &mut TcpStream, state: &mut State) -> Result<(), String> {
    let request = read_request(stream)?;
    if request.header("host") != Some(state.host.as_str()) {
        return Err("request Host did not match the exact loopback origin".into());
    }
    let suffix = request
        .path
        .strip_prefix(&state.session_path)
        .ok_or_else(|| "request path did not contain the random session capability".to_owned())?;
    match (request.method.as_str(), suffix) {
        ("GET", "") => respond_redirect(stream, &format!("{}/", state.session_path)),
        ("GET", "/") => respond_bytes(stream, 200, "text/html; charset=utf-8", PAGE),
        ("GET", "/app.js") => respond_bytes(stream, 200, "text/javascript; charset=utf-8", SCRIPT),
        ("GET", "/app.css") => respond_bytes(stream, 200, "text/css; charset=utf-8", STYLE),
        ("GET", "/register-options") => {
            if state.registration.is_some() {
                return Err("registration challenge was already consumed".into());
            }
            let body = json!({
                "publicKey": {
                    "challenge": URL_SAFE_NO_PAD.encode(state.registration_challenge),
                    "rp": { "id": "localhost", "name": "Quasar Password Manager QA" },
                    "user": {
                        "id": URL_SAFE_NO_PAD.encode(state.user_id),
                        "name": "zephium-password-manager-qa",
                        "displayName": "Quasar synthetic passkey QA"
                    },
                    "pubKeyCredParams": [{ "type": "public-key", "alg": -7 }],
                    "timeout": 120000,
                    "attestation": "none",
                    "authenticatorSelection": {
                        "residentKey": "required",
                        "requireResidentKey": true,
                        "userVerification": "required"
                    }
                }
            });
            respond_json(stream, 200, &body.to_string())
        }
        ("POST", "/register-result") => {
            require_json_post(&request, state)?;
            let result: RegistrationResult = parse_json(&request.body, "registration result")?;
            state.registration = Some(validate_registration(state, result)?);
            respond_json(stream, 200, r#"{"accepted":true}"#)
        }
        ("GET", "/assert-options") => {
            let registration = state
                .registration
                .as_ref()
                .ok_or_else(|| "assertion requested before registration".to_owned())?;
            let body = json!({
                "publicKey": {
                    "challenge": URL_SAFE_NO_PAD.encode(state.assertion_challenge),
                    "rpId": "localhost",
                    "allowCredentials": [{
                        "type": "public-key",
                        "id": URL_SAFE_NO_PAD.encode(&registration.credential_id)
                    }],
                    "timeout": 120000,
                    "userVerification": "required"
                }
            });
            respond_json(stream, 200, &body.to_string())
        }
        ("POST", "/assert-result") => {
            require_json_post(&request, state)?;
            let result: AssertionResult = parse_json(&request.body, "assertion result")?;
            validate_assertion(state, result)?;
            state.assertion_passed = true;
            respond_json(
                stream,
                200,
                r#"{"message":"Passkey assertion verified cryptographically. This synthetic localhost gate passed."}"#,
            )
        }
        ("POST", "/client-error") => {
            require_json_post(&request, state)?;
            let error: ClientError = parse_json(&request.body, "client error")?;
            if error.phase.len() > 64 || error.message.len() > 512 {
                return Err("client diagnostic exceeded its bound".into());
            }
            eprintln!(
                "password-manager WebAuthn QA browser diagnostic: {}: {}",
                error.phase, error.message
            );
            respond_json(stream, 200, r#"{"accepted":true}"#)
        }
        _ => Err("unsupported QA route or method".into()),
    }
}

fn require_json_post(request: &Request, state: &State) -> Result<(), String> {
    if request.header("origin") != Some(state.origin.as_str()) {
        return Err("POST Origin did not match the exact loopback origin".into());
    }
    let content_type = request
        .header("content-type")
        .and_then(|value| value.split(';').next())
        .map(str::trim);
    if content_type != Some("application/json") {
        return Err("POST content type was not application/json".into());
    }
    Ok(())
}

fn validate_registration(
    state: &State,
    result: RegistrationResult,
) -> Result<Registration, String> {
    if result.public_key_algorithm != -7 {
        return Err("registration did not negotiate ES256".into());
    }
    let credential_id = decode_canonical(
        &result.raw_id,
        MAX_CREDENTIAL_ID_BYTES,
        "registration credential id",
    )?;
    let textual_id = decode_canonical(
        &result.credential_id,
        MAX_CREDENTIAL_ID_BYTES,
        "registration textual credential id",
    )?;
    if credential_id.is_empty() || credential_id != textual_id {
        return Err("registration credential identities differed".into());
    }
    validate_client_data(
        &result.client_data_json,
        "webauthn.create",
        &state.registration_challenge,
        &state.origin,
    )?;
    let attestation = decode_canonical(
        &result.attestation_object,
        MAX_ATTESTATION_BYTES,
        "registration attestation",
    )?;
    if attestation.is_empty() {
        return Err("registration attestation was empty".into());
    }
    let spki = decode_canonical(&result.public_key_spki, 512, "registration public key")?;
    let point = parse_p256_spki(&spki)?;
    Ok(Registration {
        credential_id,
        public_key: point,
    })
}

fn validate_assertion(state: &State, result: AssertionResult) -> Result<(), String> {
    let registration = state
        .registration
        .as_ref()
        .ok_or_else(|| "assertion arrived before registration".to_owned())?;
    let credential_id = decode_canonical(
        &result.raw_id,
        MAX_CREDENTIAL_ID_BYTES,
        "assertion credential id",
    )?;
    let textual_id = decode_canonical(
        &result.credential_id,
        MAX_CREDENTIAL_ID_BYTES,
        "assertion textual credential id",
    )?;
    if credential_id != registration.credential_id || credential_id != textual_id {
        return Err("assertion credential did not match the registered credential".into());
    }
    let client_data = validate_client_data(
        &result.client_data_json,
        "webauthn.get",
        &state.assertion_challenge,
        &state.origin,
    )?;
    let authenticator_data = decode_canonical(
        &result.authenticator_data,
        MAX_AUTHENTICATOR_DATA_BYTES,
        "assertion authenticator data",
    )?;
    if authenticator_data.len() < 37 {
        return Err("assertion authenticator data was truncated".into());
    }
    let expected_rp_id_hash: [u8; 32] = Sha256::digest(b"localhost").into();
    if authenticator_data[..32] != expected_rp_id_hash {
        return Err("assertion relying-party hash did not match localhost".into());
    }
    let flags = authenticator_data[32];
    if flags & 0x01 == 0 {
        return Err("assertion omitted user presence".into());
    }
    if flags & 0x04 == 0 {
        return Err("assertion omitted user verification".into());
    }
    if let Some(user_handle) = result.user_handle {
        let user_handle = decode_canonical(&user_handle, 1024, "assertion user handle")?;
        if !user_handle.is_empty() && user_handle != state.user_id {
            return Err("assertion user handle did not match the synthetic user".into());
        }
    }
    let signature = decode_canonical(
        &result.signature,
        MAX_SIGNATURE_BYTES,
        "assertion signature",
    )?;
    let mut signed = Vec::with_capacity(authenticator_data.len() + 32);
    signed.extend_from_slice(&authenticator_data);
    signed.extend_from_slice(&Sha256::digest(&client_data));
    UnparsedPublicKey::new(&ECDSA_P256_SHA256_ASN1, registration.public_key)
        .verify(&signed, &signature)
        .map_err(|_| "assertion signature did not verify against the registered key".to_owned())
}

fn validate_client_data(
    encoded: &str,
    expected_kind: &str,
    expected_challenge: &[u8],
    expected_origin: &str,
) -> Result<Vec<u8>, String> {
    let bytes = decode_canonical(encoded, MAX_CLIENT_DATA_BYTES, "client data")?;
    let client: ClientData = serde_json::from_slice(&bytes)
        .map_err(|error| format!("client data JSON was invalid: {error}"))?;
    if client.kind != expected_kind
        || client.origin != expected_origin
        || client.cross_origin
        || decode_canonical(&client.challenge, 128, "client challenge")? != expected_challenge
    {
        return Err("client data did not bind the exact ceremony".into());
    }
    Ok(bytes)
}

fn parse_p256_spki(bytes: &[u8]) -> Result<[u8; 65], String> {
    if bytes.len() != P256_SPKI_PREFIX.len() + 65 || !bytes.starts_with(P256_SPKI_PREFIX) {
        return Err("registration public key was not canonical P-256 SPKI".into());
    }
    let point: [u8; 65] = bytes[P256_SPKI_PREFIX.len()..]
        .try_into()
        .map_err(|_| "registration public key point was truncated".to_owned())?;
    if point[0] != 0x04 {
        return Err("registration public key point was not uncompressed".into());
    }
    Ok(point)
}

fn decode_canonical(value: &str, max: usize, label: &str) -> Result<Vec<u8>, String> {
    if value.len() > max.saturating_mul(2) {
        return Err(format!("{label} exceeded its encoded bound"));
    }
    let bytes = URL_SAFE_NO_PAD
        .decode(value)
        .map_err(|_| format!("{label} was not unpadded base64url"))?;
    if bytes.len() > max || URL_SAFE_NO_PAD.encode(&bytes) != value {
        return Err(format!("{label} was not canonical or exceeded its bound"));
    }
    Ok(bytes)
}

fn parse_json<T: for<'de> Deserialize<'de>>(bytes: &[u8], label: &str) -> Result<T, String> {
    serde_json::from_slice(bytes).map_err(|error| format!("{label} JSON was invalid: {error}"))
}

fn read_request(stream: &mut TcpStream) -> Result<Request, String> {
    let mut bytes = Vec::with_capacity(4096);
    let mut chunk = [0_u8; 4096];
    let header_end = loop {
        let read = stream
            .read(&mut chunk)
            .map_err(|error| format!("cannot read loopback request: {error}"))?;
        if read == 0 {
            return Err("loopback request closed before its headers completed".into());
        }
        bytes.extend_from_slice(&chunk[..read]);
        if let Some(index) = find_header_end(&bytes) {
            break index;
        }
        if bytes.len() > MAX_HEADER_BYTES {
            return Err("loopback request headers exceeded their bound".into());
        }
    };
    if header_end > MAX_HEADER_BYTES {
        return Err("loopback request headers exceeded their bound".into());
    }
    let header_text = std::str::from_utf8(&bytes[..header_end - 4])
        .map_err(|_| "loopback request headers were not UTF-8".to_owned())?;
    let mut lines = header_text.split("\r\n");
    let mut request_line = lines
        .next()
        .ok_or_else(|| "loopback request omitted its request line".to_owned())?
        .split_ascii_whitespace();
    let method = request_line
        .next()
        .filter(|method| matches!(*method, "GET" | "POST"))
        .ok_or_else(|| "loopback request used an unsupported method".to_owned())?
        .to_owned();
    let path = request_line
        .next()
        .filter(|path| path.starts_with('/') && !path.contains(['?', '#']))
        .ok_or_else(|| "loopback request path was not canonical".to_owned())?
        .to_owned();
    if request_line.next() != Some("HTTP/1.1") || request_line.next().is_some() {
        return Err("loopback request did not use canonical HTTP/1.1".into());
    }
    let mut headers = Vec::new();
    let mut content_length = None;
    for line in lines {
        let (name, value) = line
            .split_once(':')
            .ok_or_else(|| "loopback request contained a malformed header".to_owned())?;
        let value = value.trim();
        if name.eq_ignore_ascii_case("content-length") {
            if content_length.is_some() {
                return Err("loopback request repeated Content-Length".into());
            }
            let parsed = value
                .parse::<usize>()
                .map_err(|_| "loopback Content-Length was invalid".to_owned())?;
            if parsed > MAX_BODY_BYTES {
                return Err("loopback request body exceeded its bound".into());
            }
            content_length = Some(parsed);
        }
        if name.eq_ignore_ascii_case("transfer-encoding") {
            return Err("loopback request transfer encoding is unsupported".into());
        }
        headers.push((name.to_owned(), value.to_owned()));
    }
    let body_length = content_length.unwrap_or(0);
    let total = header_end
        .checked_add(body_length)
        .ok_or_else(|| "loopback request length overflowed".to_owned())?;
    while bytes.len() < total {
        let read = stream
            .read(&mut chunk)
            .map_err(|error| format!("cannot read loopback request body: {error}"))?;
        if read == 0 {
            return Err("loopback request body was truncated".into());
        }
        bytes.extend_from_slice(&chunk[..read]);
        if bytes.len() > total {
            return Err("loopback request contained trailing bytes".into());
        }
    }
    if bytes.len() != total {
        return Err("loopback request length was inconsistent".into());
    }
    Ok(Request {
        method,
        path,
        headers,
        body: bytes[header_end..].to_vec(),
    })
}

fn find_header_end(bytes: &[u8]) -> Option<usize> {
    bytes
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|index| index + 4)
}

fn respond_bytes(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    body: &[u8],
) -> Result<(), String> {
    let reason = if status == 200 { "OK" } else { "Bad Request" };
    let header = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nContent-Security-Policy: default-src 'none'; script-src 'self'; style-src 'self'; connect-src 'self'; base-uri 'none'; frame-ancestors 'none'; form-action 'none'\r\nCross-Origin-Opener-Policy: same-origin\r\nCross-Origin-Resource-Policy: same-origin\r\nReferrer-Policy: no-referrer\r\nX-Content-Type-Options: nosniff\r\nPermissions-Policy: publickey-credentials-get=(self), publickey-credentials-create=(self)\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream
        .write_all(header.as_bytes())
        .and_then(|()| stream.write_all(body))
        .and_then(|()| stream.flush())
        .map_err(|error| format!("cannot write loopback response: {error}"))
}

fn respond_json(stream: &mut TcpStream, status: u16, body: &str) -> Result<(), String> {
    respond_bytes(
        stream,
        status,
        "application/json; charset=utf-8",
        body.as_bytes(),
    )
}

fn respond_redirect(stream: &mut TcpStream, location: &str) -> Result<(), String> {
    let header = format!(
        "HTTP/1.1 308 Permanent Redirect\r\nLocation: {location}\r\nContent-Length: 0\r\nCache-Control: no-store\r\nReferrer-Policy: no-referrer\r\nX-Content-Type-Options: nosniff\r\nConnection: close\r\n\r\n"
    );
    stream
        .write_all(header.as_bytes())
        .and_then(|()| stream.flush())
        .map_err(|error| format!("cannot write loopback redirect: {error}"))
}

#[cfg(test)]
mod tests {
    use super::{
        decode_canonical, find_header_end, parse_arguments, parse_p256_spki, P256_SPKI_PREFIX,
    };
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine as _;

    #[test]
    fn qa_arguments_are_closed_and_bounded() {
        assert_eq!(parse_arguments(&[]).unwrap(), 0);
        assert_eq!(
            parse_arguments(&["--port".into(), "38473".into()]).unwrap(),
            38473
        );
        assert!(parse_arguments(&["--port".into(), "0".into()]).is_err());
        assert!(parse_arguments(&["--bind".into(), "0.0.0.0".into()]).is_err());
    }

    #[test]
    fn base64url_decoder_rejects_padding_and_oversize() {
        assert_eq!(decode_canonical("AA", 1, "fixture").unwrap(), [0]);
        assert!(decode_canonical("AA==", 4, "fixture").is_err());
        assert!(decode_canonical(&URL_SAFE_NO_PAD.encode([0_u8; 3]), 2, "fixture").is_err());
    }

    #[test]
    fn p256_key_parser_requires_the_exact_spki_shape() {
        let mut spki = P256_SPKI_PREFIX.to_vec();
        spki.extend_from_slice(&[0x04; 65]);
        assert_eq!(parse_p256_spki(&spki).unwrap()[0], 0x04);
        spki[0] ^= 1;
        assert!(parse_p256_spki(&spki).is_err());
    }

    #[test]
    fn header_terminator_is_exact() {
        assert_eq!(find_header_end(b"GET / HTTP/1.1\r\n\r\nbody"), Some(18));
        assert_eq!(find_header_end(b"GET / HTTP/1.1\n\n"), None);
    }
}
