//! On-device dictation. Retired cloud identifiers remain readable in stored
//! preferences, but cannot upload audio or authenticate requests.

use zephium_core::work::model::WorkModelError;

use crate::work_models;

/// Who turns speech into words.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum VoiceEngine {
    /// Apple's recognizer on this Mac.
    OnDevice,
    /// The signed-in account's hosted transcription.
    Zephium,
    /// The person's own OpenAI key.
    OpenAi,
    /// The person's own Groq key.
    Groq,
}

const ENGINE_KEY: &str = "voice.engine";
const LANGUAGE_KEY: &str = "voice.language";
impl VoiceEngine {
    fn id(self) -> &'static str {
        match self {
            Self::OnDevice => "on_device",
            Self::Zephium => "zephium",
            Self::OpenAi => "openai",
            Self::Groq => "groq",
        }
    }

    fn from_id(id: &str) -> Option<Self> {
        [Self::OnDevice, Self::Zephium, Self::OpenAi, Self::Groq]
            .into_iter()
            .find(|engine| engine.id() == id)
    }

    /// Whether the audio leaves this device.
    pub fn cloud(self) -> bool {
        self != Self::OnDevice
    }
}

/// The engine the person chose in Settings → AI, if any.
pub fn chosen() -> Option<VoiceEngine> {
    work_models::setting(ENGINE_KEY)
        .and_then(|id| VoiceEngine::from_id(&id))
        .filter(|engine| !engine.cloud())
}

pub fn choose(engine: Option<VoiceEngine>) -> bool {
    if engine.is_some_and(VoiceEngine::cloud) {
        return false;
    }
    work_models::store_setting(
        ENGINE_KEY.to_owned(),
        engine.map(VoiceEngine::id).unwrap_or_default().to_owned(),
    )
}

/// A BCP 47 tag such as `pl` or `en-GB`, or `None` for the system's.
fn valid_language(tag: &str) -> bool {
    (2..=35).contains(&tag.len())
        && tag.split('-').all(|part| {
            (1..=8).contains(&part.len()) && part.bytes().all(|b| b.is_ascii_alphanumeric())
        })
        && tag
            .bytes()
            .take_while(|b| *b != b'-')
            .all(|b| b.is_ascii_alphabetic())
}

/// The language the person chose for dictation; `None` follows the system.
pub fn language() -> Option<String> {
    work_models::setting(LANGUAGE_KEY).filter(|tag| valid_language(tag))
}

pub fn choose_language(tag: Option<&str>) -> bool {
    match tag {
        Some(tag) if !valid_language(tag) => false,
        tag => work_models::store_setting(LANGUAGE_KEY.to_owned(), tag.unwrap_or_default().into()),
    }
}

/// ChatGPT plan authorization does not grant the Audio Transcriptions API.
/// Dictation uses the operating system's recognizer without uploading audio.
pub async fn cloud_engines() -> Vec<VoiceEngine> {
    Vec::new()
}

pub fn effective(
    _: Option<VoiceEngine>,
    on_device: bool,
    _: &[VoiceEngine],
) -> Option<VoiceEngine> {
    on_device.then_some(VoiceEngine::OnDevice)
}

/// Legacy cloud selections cannot send audio using retired credentials.
pub async fn transcribe(
    _: VoiceEngine,
    _: &[u8],
    _: Option<&str>,
) -> Result<String, WorkModelError> {
    Err(WorkModelError::BadRequest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use VoiceEngine::*;

    #[test]
    fn retired_cloud_engines_never_receive_audio() {
        for engine in [
            None,
            Some(OnDevice),
            Some(Zephium),
            Some(OpenAi),
            Some(Groq),
        ] {
            assert_eq!(
                effective(engine, true, &[Zephium, OpenAi, Groq]),
                Some(OnDevice)
            );
            assert_eq!(effective(engine, false, &[Zephium, OpenAi, Groq]), None);
        }
    }

    #[test]
    fn engine_ids_round_trip_and_unknown_ones_read_as_unset() {
        for engine in [OnDevice, Zephium, OpenAi, Groq] {
            assert_eq!(VoiceEngine::from_id(engine.id()), Some(engine));
        }
        assert_eq!(VoiceEngine::from_id("whisper"), None);
        assert!(!OnDevice.cloud() && Zephium.cloud());
    }

    #[test]
    fn only_plain_language_tags_are_kept() {
        for tag in ["pl", "en-GB", "zh-Hant-TW", "es-419"] {
            assert!(valid_language(tag), "{tag}");
        }
        for tag in [
            "",
            "p",
            "en_GB",
            "en-",
            "419",
            "x".repeat(40).as_str(),
            "pl;rm",
        ] {
            assert!(!valid_language(tag), "{tag}");
        }
    }
}
