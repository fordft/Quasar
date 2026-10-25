//! Speech to text through a provider's audio transcription endpoint: the
//! person's own OpenAI or Groq key, or Zephium Cloud's OpenAI passthrough.
//! The audio is sent once and never retried, so a hosted take is never
//! charged twice; nothing of it is kept or logged.

use std::time::Duration;

use reqwest::header::{HeaderMap, HeaderValue, CONTENT_TYPE};
use reqwest::Url;
use serde_json::Value;
use zephium_core::work::model::{WorkModelError, WorkModelProvider};

use super::{
    bounded_body, cloud, http, idempotency_key, is_loopback, join, LeadCloudAccounting,
    LeadCloudObserver, LeadCloudRecord, LeadCredential, GROQ_BASE, OPENAI_BASE,
};

/// OpenAI's and Groq's upload ceiling.
const PROVIDER_MAX_BYTES: usize = 25 * 1024 * 1024;
/// The gateway's request body ceiling.
const CLOUD_MAX_BYTES: usize = 8 * 1024 * 1024;
const MAX_TEXT_BYTES: usize = 64 * 1024;
const TIMEOUT: Duration = Duration::from_secs(90);
const OPENAI_MODEL: &str = "gpt-4o-transcribe";
const GROQ_MODEL: &str = "whisper-large-v3-turbo";

/// Where one transcription goes.
#[derive(Clone, Debug)]
pub struct TranscriptionTarget {
    url: Url,
    model: String,
    cloud: bool,
    max_bytes: usize,
}

fn valid_model(model: &str) -> bool {
    (1..=100).contains(&model.len())
        && model
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b':' | b'/' | b'-'))
}

impl TranscriptionTarget {
    /// The provider's own endpoint with its default speech model.
    pub fn direct(provider: WorkModelProvider) -> Result<Self, WorkModelError> {
        let (base, model) = match provider {
            WorkModelProvider::OpenAi => (OPENAI_BASE, OPENAI_MODEL),
            WorkModelProvider::Groq => (GROQ_BASE, GROQ_MODEL),
            _ => return Err(WorkModelError::BadRequest),
        };
        let base = Url::parse(base).map_err(|_| WorkModelError::BadRequest)?;
        Ok(Self {
            url: join(&base, "audio/transcriptions")?,
            model: model.to_owned(),
            cloud: false,
            max_bytes: PROVIDER_MAX_BYTES,
        })
    }

    /// Zephium Cloud's `/openai/v1/audio/transcriptions` with a hosted
    /// catalog model, held to the model's own request limit.
    pub fn cloud(
        base: &str,
        model: &str,
        max_request_bytes: Option<u64>,
    ) -> Result<Self, WorkModelError> {
        let root = Url::parse(base).map_err(|_| WorkModelError::BadRequest)?;
        if (root.scheme() != "https" && !is_loopback(&root)) || !valid_model(model) {
            return Err(WorkModelError::BadRequest);
        }
        let limit = max_request_bytes
            .and_then(|bytes| usize::try_from(bytes).ok())
            .unwrap_or(CLOUD_MAX_BYTES)
            .min(CLOUD_MAX_BYTES);
        Ok(Self {
            url: join(&root, "openai/v1/audio/transcriptions")?,
            model: model.to_owned(),
            cloud: true,
            max_bytes: limit,
        })
    }

    /// The endpoint this target posts to.
    pub fn url(&self) -> &Url {
        &self.url
    }

    /// The model named in the upload.
    pub fn model(&self) -> &str {
        &self.model
    }
}

/// What came back.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Transcript {
    /// The words, trimmed.
    pub text: String,
    /// The gateway's request id, for a hosted take's receipt.
    pub request_id: Option<String>,
    /// What the gateway said the take cost so far: `Pending` until settled.
    pub accounting: Option<LeadCloudAccounting>,
}

fn part(body: &mut Vec<u8>, boundary: &str, name: &str, value: &str) {
    body.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n"
        )
        .as_bytes(),
    );
}

fn multipart(boundary: &str, model: &str, language: Option<&str>, wav: &[u8]) -> Vec<u8> {
    let mut body = Vec::with_capacity(wav.len() + 640);
    part(&mut body, boundary, "model", model);
    part(&mut body, boundary, "response_format", "json");
    if let Some(language) = language {
        part(&mut body, boundary, "language", language);
    }
    body.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"speech.wav\"\r\nContent-Type: audio/wav\r\n\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(wav);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    body
}

/// An ISO 639-1 code, the only language hint the endpoints take.
fn language_hint(language: Option<&str>) -> Option<String> {
    let code = language?.split(['-', '_']).next()?.to_ascii_lowercase();
    (code.len() == 2 && code.bytes().all(|b| b.is_ascii_lowercase())).then_some(code)
}

/// Sends one WAV take. `language` (a BCP 47 tag) is a hint; without it the
/// model detects the language.
pub async fn transcribe(
    target: &TranscriptionTarget,
    credential: &dyn LeadCredential,
    wav: &[u8],
    language: Option<&str>,
) -> Result<Transcript, WorkModelError> {
    transcribe_observed(target, credential, wav, language, None).await
}

/// [`transcribe`], reporting a hosted take's accounting to `observer` however
/// it ends, so the account's usage can refresh and the receipt be checked.
pub async fn transcribe_observed(
    target: &TranscriptionTarget,
    credential: &dyn LeadCredential,
    wav: &[u8],
    language: Option<&str>,
    observer: Option<&LeadCloudObserver>,
) -> Result<Transcript, WorkModelError> {
    if wav.is_empty() {
        return Err(WorkModelError::BadRequest);
    }
    let boundary = format!("zephium-{}", ulid::Ulid::new());
    let body = multipart(
        &boundary,
        &target.model,
        language_hint(language).as_deref(),
        wav,
    );
    if body.len() > target.max_bytes {
        return Err(WorkModelError::ContextTooLong);
    }
    let secret = credential.secret().await?;
    let mut headers = HeaderMap::new();
    headers.insert("authorization", secret.header("Bearer ")?);
    headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_str(&format!("multipart/form-data; boundary={boundary}"))
            .map_err(|_| WorkModelError::Protocol)?,
    );
    let key = if target.cloud {
        let key = idempotency_key()?;
        headers.insert("idempotency-key", key.clone());
        Some(key)
    } else {
        None
    };
    let record =
        |answered: cloud::Answered, request_id: Option<String>, accounting: LeadCloudAccounting| {
            if let (Some(observer), Some(key)) = (observer, &key) {
                observer(LeadCloudRecord {
                    idempotency_key: key.to_str().unwrap_or_default().to_owned(),
                    request_id: request_id.or(answered.request_id),
                    policy_revision: answered.policy_revision,
                    accounting,
                });
            }
        };
    let response = http(is_loopback(&target.url))?
        .post(target.url.clone())
        .headers(headers)
        .timeout(TIMEOUT)
        .body(body)
        .send()
        .await
        .map_err(|error| {
            if error.is_builder() {
                WorkModelError::Protocol
            } else {
                // The take may have arrived; the caller never sends it again on its own.
                record(Default::default(), None, LeadCloudAccounting::Unknown);
                WorkModelError::Network
            }
        })?;
    let status = response.status();
    let headers = response.headers().clone();
    let answered = cloud::Answered::read(&headers);
    let request_id = answered.request_id.clone();
    let body = bounded_body(response).await;
    if !status.is_success() {
        let (error, _) = super::refused(status, &headers, &body, target.cloud);
        if error == WorkModelError::Unauthorized {
            credential.rejected();
        }
        let request_id = cloud::error_request_id(&body, &answered);
        let accounting = match cloud::refusal(status, request_id.as_deref()) {
            cloud::Refusal::Uncertain => LeadCloudAccounting::Unknown,
            _ => LeadCloudAccounting::Refused,
        };
        record(answered, request_id, accounting);
        return Err(error);
    }
    let accounting = answered
        .usage
        .clone()
        .unwrap_or(LeadCloudAccounting::Unknown);
    record(answered, request_id.clone(), accounting.clone());
    let json: Value = serde_json::from_slice(&body).map_err(|_| WorkModelError::Protocol)?;
    let text = json
        .get("text")
        .and_then(Value::as_str)
        .ok_or(WorkModelError::Protocol)?
        .trim();
    if text.len() > MAX_TEXT_BYTES {
        return Err(WorkModelError::Protocol);
    }
    Ok(Transcript {
        text: text.to_owned(),
        request_id,
        accounting: target.cloud.then_some(accounting),
    })
}

#[cfg(test)]
mod tests {
    use super::super::tests::{http_reply, serve};
    use super::super::{LeadSecret, LeadStaticCredential};
    use super::*;
    use std::sync::{Arc, Mutex};
    use zephium_core::work::model::HostedRefusal;

    fn credential() -> LeadStaticCredential {
        LeadStaticCredential::new(LeadSecret::new("sk-test".into()).unwrap())
    }

    fn at(base: &str, cloud: bool) -> TranscriptionTarget {
        TranscriptionTarget {
            url: Url::parse(&format!("{base}/audio/transcriptions")).unwrap(),
            model: "gpt-4o-transcribe".into(),
            cloud,
            max_bytes: CLOUD_MAX_BYTES,
        }
    }

    #[test]
    fn targets_name_each_providers_endpoint_and_model() {
        let openai = TranscriptionTarget::direct(WorkModelProvider::OpenAi).unwrap();
        assert_eq!(
            openai.url().as_str(),
            "https://api.openai.com/v1/audio/transcriptions"
        );
        assert_eq!(openai.model(), OPENAI_MODEL);
        let groq = TranscriptionTarget::direct(WorkModelProvider::Groq).unwrap();
        assert_eq!(
            groq.url().as_str(),
            "https://api.groq.com/openai/v1/audio/transcriptions"
        );
        assert_eq!(groq.model(), GROQ_MODEL);
        assert!(TranscriptionTarget::direct(WorkModelProvider::Anthropic).is_err());
        let cloud = TranscriptionTarget::cloud(
            "https://legacy-gateway.example.invalid",
            "transcribe-1",
            None,
        )
        .unwrap();
        assert_eq!(
            cloud.url().as_str(),
            "https://legacy-gateway.example.invalid/openai/v1/audio/transcriptions"
        );
        assert!(TranscriptionTarget::cloud("http://example.com", "m", None).is_err());
        assert!(
            TranscriptionTarget::cloud("https://legacy-gateway.example.invalid", "a b", None)
                .is_err()
        );
    }

    #[test]
    fn the_language_hint_is_a_bare_iso_639_1_code() {
        assert_eq!(language_hint(Some("pl-PL")).as_deref(), Some("pl"));
        assert_eq!(language_hint(Some("en_US")).as_deref(), Some("en"));
        assert_eq!(language_hint(Some("yue")), None);
        assert_eq!(language_hint(None), None);
    }

    #[tokio::test]
    async fn a_direct_take_is_one_multipart_upload_with_the_key() {
        let (base, seen) = serve(vec![http_reply(
            "200 OK",
            "content-type: application/json\r\n",
            "{\"text\":\" Book a table for two. \"}",
        )]);
        let target = at(&base, false);
        let wav = zephium_wav();
        let transcript = transcribe(&target, &credential(), &wav, Some("en-US"))
            .await
            .unwrap();
        assert_eq!(transcript.text, "Book a table for two.");
        assert_eq!(transcript.request_id, None);
        let request = seen.lock().unwrap()[0].clone();
        let lower = request.to_ascii_lowercase();
        assert!(request.starts_with("POST /v1/audio/transcriptions "));
        assert!(lower.contains("authorization: bearer sk-test"));
        assert!(lower.contains("content-type: multipart/form-data; boundary=zephium-"));
        assert!(!lower.contains("idempotency-key"));
        assert!(request.contains("name=\"model\"\r\n\r\ngpt-4o-transcribe\r\n"));
        assert!(request.contains("name=\"language\"\r\n\r\nen\r\n"));
        assert!(request.contains("filename=\"speech.wav\"\r\nContent-Type: audio/wav"));
    }

    #[tokio::test]
    async fn a_hosted_take_carries_an_idempotency_key_and_reports_its_request() {
        let (base, seen) = serve(vec![http_reply(
            "200 OK",
            "content-type: application/json\r\nx-zephium-request-id: 6f1c1c1e-1d6b-4c86-9b77-0c1f1d3e9a10\r\n",
            "{\"text\":\"Cześć\"}",
        )]);
        let target = at(&base, true);
        let transcript = transcribe(&target, &credential(), &zephium_wav(), None)
            .await
            .unwrap();
        assert_eq!(transcript.text, "Cześć");
        assert_eq!(
            transcript.request_id.as_deref(),
            Some("6f1c1c1e-1d6b-4c86-9b77-0c1f1d3e9a10")
        );
        let request = seen.lock().unwrap()[0].to_ascii_lowercase();
        assert!(request.contains("idempotency-key: "));
        assert!(!request.contains("name=\"language\""));
    }

    #[tokio::test]
    async fn a_hosted_takes_usage_header_says_whether_the_charge_is_final() {
        let reply = |usage: &str| {
            http_reply(
                "200 OK",
                &format!(
                    "content-type: application/json\r\nx-zephium-request-id: r-1\r\nx-zephium-policy-revision: 4\r\nx-zephium-usage: {usage}\r\n"
                ),
                "{\"text\":\"\"}",
            )
        };
        let (base, _) = serve(vec![reply("pending"), reply("settled")]);
        let target = at(&base, true);
        let records: Arc<Mutex<Vec<LeadCloudRecord>>> = Default::default();
        let log = records.clone();
        let observer: LeadCloudObserver = Arc::new(move |record| log.lock().unwrap().push(record));
        let wav = zephium_wav();
        let pending = transcribe_observed(&target, &credential(), &wav, None, Some(&observer))
            .await
            .unwrap();
        assert_eq!(pending.text, "", "an empty take is valid");
        assert_eq!(pending.accounting, Some(LeadCloudAccounting::Pending));
        let settled = transcribe_observed(&target, &credential(), &wav, None, Some(&observer))
            .await
            .unwrap();
        assert_eq!(
            settled.accounting,
            Some(LeadCloudAccounting::Settled { charged: None })
        );
        let records = records.lock().unwrap();
        assert_eq!(records.len(), 2);
        assert!(
            records[0].unsettled(),
            "pending is checked against its receipt"
        );
        assert!(!records[1].unsettled());
        assert_eq!(records[0].request_id.as_deref(), Some("r-1"));
        assert_eq!(records[0].policy_revision.as_deref(), Some("4"));
        assert_eq!(records[0].idempotency_key.len(), 36);
    }

    #[tokio::test]
    async fn the_recorders_own_refusals_and_hosted_codes_stay_distinct() {
        let error = |status: &str, code: &str, extra: &str| {
            http_reply(
                status,
                extra,
                &format!("{{\"error\":{{\"code\":\"{code}\"}},\"request_id\":null}}"),
            )
        };
        let (base, seen) = serve(vec![
            error("400 Bad Request", "invalid_audio", ""),
            error("415 Unsupported Media Type", "unsupported_audio_format", ""),
            error("422 Unprocessable Entity", "audio_too_long", ""),
            error("403 Forbidden", "plan_required", ""),
            error("403 Forbidden", "billing_review_required", ""),
            error("402 Payment Required", "daily_limit_reached", ""),
            error(
                "429 Too Many Requests",
                "free_usage_limit_reached",
                "retry-after: 600\r\n",
            ),
            error(
                "503 Service Unavailable",
                "cloud_budget_exhausted",
                "retry-after: 3600\r\n",
            ),
        ]);
        let target = at(&base, true);
        let wav = zephium_wav();
        let mut got = Vec::new();
        for _ in 0..8 {
            got.push(
                transcribe(&target, &credential(), &wav, None)
                    .await
                    .unwrap_err(),
            );
        }
        let hosted = |refusal, retry_after_ms| WorkModelError::Hosted {
            refusal,
            retry_after_ms,
        };
        assert_eq!(got[0], hosted(HostedRefusal::InvalidAudio, None));
        assert_eq!(got[1], hosted(HostedRefusal::UnsupportedAudio, None));
        assert_eq!(got[2], hosted(HostedRefusal::AudioTooLong, None));
        assert_eq!(got[3], hosted(HostedRefusal::PlanRequired, None));
        assert_eq!(got[4], hosted(HostedRefusal::BillingReview, None));
        assert!(matches!(
            got[5],
            WorkModelError::Hosted {
                refusal: HostedRefusal::DailyLimit,
                retry_after_ms: Some(_)
            }
        ));
        assert_eq!(
            got[6],
            hosted(HostedRefusal::FreeUsagePaused, Some(600_000))
        );
        assert_eq!(got[7], hosted(HostedRefusal::Paused, Some(3_600_000)));
        assert_eq!(seen.lock().unwrap().len(), 8, "no take is ever sent again");
    }

    #[tokio::test]
    async fn refusals_are_closed_errors_and_never_retried() {
        let (base, seen) = serve(vec![
            http_reply(
                "402 Payment Required",
                "",
                "{\"error\":{\"code\":\"insufficient_credits\"}}",
            ),
            http_reply(
                "401 Unauthorized",
                "",
                "{\"error\":{\"code\":\"unauthorized\"}}",
            ),
            http_reply("429 Too Many Requests", "retry-after: 7\r\n", "{}"),
        ]);
        let target = at(&base, true);
        let wav = zephium_wav();
        assert_eq!(
            transcribe(&target, &credential(), &wav, None).await,
            Err(WorkModelError::Hosted {
                refusal: HostedRefusal::OutOfCredits,
                retry_after_ms: None
            })
        );
        assert_eq!(
            transcribe(&target, &credential(), &wav, None).await,
            Err(WorkModelError::Unauthorized)
        );
        assert_eq!(
            transcribe(&target, &credential(), &wav, None).await,
            Err(WorkModelError::RateLimited {
                retry_after_ms: Some(7_000)
            })
        );
        assert_eq!(seen.lock().unwrap().len(), 3);
    }

    #[tokio::test]
    async fn an_oversized_take_is_refused_before_it_is_sent() {
        let mut target = TranscriptionTarget::cloud(
            "https://legacy-gateway.example.invalid",
            "transcribe-1",
            Some(1_024),
        )
        .unwrap();
        assert_eq!(target.max_bytes, 1_024);
        let wav = vec![0_u8; 2_048];
        assert_eq!(
            transcribe(&target, &credential(), &wav, None).await,
            Err(WorkModelError::ContextTooLong)
        );
        target.max_bytes = CLOUD_MAX_BYTES;
        assert_eq!(
            transcribe(&target, &credential(), &[], None).await,
            Err(WorkModelError::BadRequest)
        );
    }

    fn zephium_wav() -> Vec<u8> {
        let mut wav = b"RIFF".to_vec();
        wav.extend_from_slice(&[0; 40]);
        wav
    }
}
