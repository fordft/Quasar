//! Native menu text in the interface language. The frame's translations are
//! embedded; only the language shown is parsed, when it is chosen. A string
//! not yet translated, or English itself, is the text the menu was built with.

use std::collections::HashMap;
use std::sync::RwLock;

const DICTIONARIES: &[(&str, &str)] = &[
    ("de", include_str!("../../frame/messages/de.json")),
    ("es", include_str!("../../frame/messages/es.json")),
    ("fr", include_str!("../../frame/messages/fr.json")),
    ("it", include_str!("../../frame/messages/it.json")),
    ("ja", include_str!("../../frame/messages/ja.json")),
    ("ko", include_str!("../../frame/messages/ko.json")),
    ("pt-BR", include_str!("../../frame/messages/pt-BR.json")),
    ("ru", include_str!("../../frame/messages/ru.json")),
    ("tr", include_str!("../../frame/messages/tr.json")),
    ("uk", include_str!("../../frame/messages/uk.json")),
    ("zh-CN", include_str!("../../frame/messages/zh-CN.json")),
    ("zh-TW", include_str!("../../frame/messages/zh-TW.json")),
];

static SHOWN: RwLock<Option<(&'static str, HashMap<String, String>)>> = RwLock::new(None);

fn translated(tag: &str) -> Option<&'static str> {
    // Apple names Chinese by script; the dictionaries are named by region.
    let lower = tag.to_ascii_lowercase();
    if lower.starts_with("zh-hant") {
        return Some("zh-TW");
    }
    if lower.starts_with("zh-hans") {
        return Some("zh-CN");
    }
    let base = tag.split(['-', '_']).next().unwrap_or(tag);
    DICTIONARIES
        .iter()
        .map(|(locale, _)| *locale)
        .find(|locale| locale.eq_ignore_ascii_case(tag))
        .or_else(|| {
            DICTIONARIES
                .iter()
                .map(|(locale, _)| *locale)
                .find(|locale| locale.eq_ignore_ascii_case(base))
        })
}

/// Follows `ui.language`, resolving "system" the way the frame does: the
/// first of the system's languages Zephium is translated into. Returns
/// whether the language shown changed, so the menus can be rebuilt.
pub(crate) fn set_language(preference: Option<&str>) -> bool {
    let wanted = match preference {
        Some(tag) if tag != "system" => translated(tag),
        _ => zephium_voice::preferred_languages()
            .iter()
            .find_map(|tag| {
                translated(tag)
                    .map(Some)
                    .or(tag.starts_with("en").then_some(None))
            })
            .flatten(),
    };
    let mut shown = SHOWN
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if shown.as_ref().map(|(locale, _)| *locale) == wanted {
        return false;
    }
    *shown = wanted.and_then(|locale| {
        let (_, text) = DICTIONARIES.iter().find(|(known, _)| *known == locale)?;
        Some((locale, serde_json::from_str(text).ok()?))
    });
    true
}

/// The menu text for `key`, or `english` while the interface is English or
/// the key has no translation.
fn text(key: &str, english: &str) -> String {
    SHOWN
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .as_ref()
        .and_then(|(_, strings)| strings.get(key))
        .filter(|text| !text.is_empty())
        .cloned()
        .unwrap_or_else(|| english.to_owned())
}

/// A menu label in the interface language, looked up by its English text:
/// "Split View…" is `native_split_view_more` in `frame/messages`.
pub(crate) fn label(english: &str) -> String {
    text(&key(english), english)
}

pub(crate) fn key(english: &str) -> String {
    let mut key = String::from("native");
    for word in english
        .replace('…', " more")
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|word| !word.is_empty())
    {
        key.push('_');
        let word = word.to_ascii_lowercase();
        // Keep the existing dictionary IDs while displaying the fork's name.
        key.push_str(if word == "quasar" { "zephium" } else { &word });
    }
    key
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_shipped_language_is_embedded_and_parses() {
        for (locale, text) in DICTIONARIES {
            let strings: HashMap<String, String> = serde_json::from_str(text).unwrap();
            assert!(strings.len() > 1000, "{locale}");
        }
        assert_eq!(translated("de-AT"), Some("de"));
        assert_eq!(translated("pt-BR"), Some("pt-BR"));
        assert_eq!(translated("zh-Hant-TW"), Some("zh-TW"));
        assert_eq!(translated("zh-Hans-CN"), Some("zh-CN"));
        assert_eq!(translated("nl"), None);
        assert_eq!(key("Split View…"), "native_split_view_more");
        assert_eq!(key("Appearance: System"), "native_appearance_system");
    }

    #[test]
    fn a_chosen_language_translates_and_english_falls_back() {
        assert!(set_language(Some("fr")));
        assert!(!set_language(Some("fr")));
        assert_eq!(text("new_tab", "New Tab"), "Nouvel onglet");
        assert_eq!(label("Quit Quasar"), "Quitter Quasar");
        assert_eq!(text("native_not_a_key", "Fallback"), "Fallback");
        assert!(set_language(Some("en")));
        assert_eq!(text("new_tab", "New Tab"), "New Tab");
    }

    #[test]
    fn every_native_label_has_an_english_message_to_translate() {
        let english: HashMap<String, String> =
            serde_json::from_str(include_str!("../../frame/messages/en.json")).unwrap();
        let sources = [
            include_str!("lib.rs"),
            include_str!("spaces.rs"),
            include_str!("help.rs"),
        ];
        let labels = sources.iter().flat_map(|source| {
            source
                .split("menu_text::label(\"")
                .skip(1)
                // The tab and bookmark menus: `item(0, "Reload Tab", …)`.
                .chain(source.split("item(").skip(1).filter_map(|rest| {
                    let (index, rest) = rest.split_once(", \"")?;
                    index.parse::<usize>().is_ok().then_some(rest)
                }))
                .filter_map(|rest| rest.split_once('"').map(|(label, _)| label))
                .filter(|label| label.chars().next().is_some_and(char::is_uppercase))
        });
        for label in labels.chain(
            zephium_core::commands::REGISTRY
                .iter()
                .map(|command| command.title),
        ) {
            assert_eq!(
                english.get(&key(label)).map(String::as_str),
                Some(label),
                "{label}"
            );
        }
    }
}
