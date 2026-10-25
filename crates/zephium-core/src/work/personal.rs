//! What the agent knows about the person: facts it remembered, and whether a
//! work may look through their history, notes and open tabs. Memory is the
//! person's, visible and editable; consent is asked once per work.
use serde::{Deserialize, Serialize};

use super::{WorkError, WorkExecutionId, WorkId};

pub const MAX_WORK_MEMORIES: usize = 500;
pub const MAX_WORK_MEMORY_CHARS: usize = 280;
pub const MAX_WORK_MEMORY_PAGE: u16 = 500;
pub const MAX_WORK_RECALL_QUERY_BYTES: usize = 256;
pub const MAX_WORK_HISTORY_HITS: u16 = 12;
/// Pages one look at recent history returns, one per address.
pub const MAX_WORK_RECENT_HISTORY: u16 = 40;

/// What kind of thing a memory is; the canvas and Settings group by it.
#[cfg_attr(feature = "ipc-types", derive(specta::Type))]
#[derive(Clone, Copy, Debug, Serialize, Deserialize, Eq, PartialEq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum WorkMemoryKindV1 {
    /// "Prefers aisle seats", "Writes in British English".
    Preference,
    /// "Anna is the design lead", "My manager is Tom".
    Person,
    /// "Zephium ships from the work-mode-integration branch".
    Project,
    Fact,
}
impl WorkMemoryKindV1 {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Preference => "preference",
            Self::Person => "person",
            Self::Project => "project",
            Self::Fact => "fact",
        }
    }
    pub fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "preference" => Self::Preference,
            "person" => Self::Person,
            "project" => Self::Project,
            "fact" => Self::Fact,
            _ => return None,
        })
    }
}

/// One remembered fact.
#[cfg_attr(feature = "ipc-types", derive(specta::Type))]
#[derive(Clone, Serialize, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct WorkMemoryV1 {
    /// A ULID.
    pub id: String,
    pub text: String,
    pub kind: WorkMemoryKindV1,
    /// The work whose run remembered it; absent when the person wrote it or
    /// the work is gone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub work: Option<WorkId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution: Option<WorkExecutionId>,
    /// What that work was asked, clipped, so the person knows where it came from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// Unix epoch milliseconds as decimal text.
    pub created_ms: String,
    /// When a run last drew on it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub used_ms: Option<String>,
}
impl std::fmt::Debug for WorkMemoryV1 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("WorkMemoryV1([content redacted])")
    }
}

/// A source of the person's own the agent may look through with consent.
#[cfg_attr(feature = "ipc-types", derive(specta::Type))]
#[derive(Clone, Copy, Debug, Serialize, Deserialize, Eq, PartialEq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum WorkContextSourceV1 {
    History,
    Notes,
    Tabs,
}
impl WorkContextSourceV1 {
    pub const fn name(self) -> &'static str {
        match self {
            Self::History => "history",
            Self::Notes => "notes",
            Self::Tabs => "tabs",
        }
    }
    pub fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "history" => Self::History,
            "notes" => Self::Notes,
            "tabs" => Self::Tabs,
            _ => return None,
        })
    }
}

/// A page from the person's history: never its content.
#[derive(Clone, Eq, PartialEq)]
pub struct WorkHistoryHit {
    pub url: String,
    pub title: String,
    /// Unix seconds of the latest visit.
    pub last_visit: i64,
}
impl std::fmt::Debug for WorkHistoryHit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("WorkHistoryHit([content redacted])")
    }
}

#[derive(Clone)]
pub enum WorkPersonalRequest {
    /// Newest first; `query` searches the words, `work` keeps one work's.
    Memories {
        query: Option<String>,
        work: Option<WorkId>,
        limit: u16,
    },
    /// Adds a fact, or refreshes the same fact already kept.
    Remember {
        id: String,
        text: String,
        kind: WorkMemoryKindV1,
        work: Option<WorkId>,
        execution: Option<WorkExecutionId>,
        now_ms: u64,
    },
    Edit {
        id: String,
        text: String,
        kind: WorkMemoryKindV1,
    },
    Forget {
        id: String,
    },
    ForgetAll,
    /// A run drew on these.
    Used {
        ids: Vec<String>,
        now_ms: u64,
    },
    /// The work's standing answer for a source; `set` records one first.
    Consent {
        work: WorkId,
        source: WorkContextSourceV1,
        set: Option<bool>,
    },
    SearchHistory {
        query: String,
        limit: u16,
    },
    /// The pages visited in `[since, until)` (unix seconds), newest first,
    /// each address once.
    RecentHistory {
        since: i64,
        until: i64,
        limit: u16,
    },
}
impl std::fmt::Debug for WorkPersonalRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("WorkPersonalRequest([content redacted])")
    }
}

#[derive(Clone, Debug)]
pub enum WorkPersonalReply {
    Memories(Vec<WorkMemoryV1>),
    Memory(Option<WorkMemoryV1>),
    Consent(Option<bool>),
    History(Vec<WorkHistoryHit>),
    Done,
}

/// Mints a memory's identity at a trusted edge.
pub fn new_memory_id() -> String {
    ulid::Ulid::new().to_string()
}

pub fn valid_memory_id(id: &str) -> bool {
    ulid::Ulid::from_string(id).is_ok_and(|parsed| parsed.to_string() == id)
}

/// Why a fact is refused, in words the model can act on.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkMemoryFault {
    Empty,
    TooLong,
    /// A memory is one line.
    Lines,
    /// It looks like a password, key, card or account number.
    Secret,
}
impl WorkMemoryFault {
    pub const fn words(self) -> &'static str {
        match self {
            Self::Empty => "text is empty",
            Self::TooLong => "text is longer than 280 characters; keep one short fact",
            Self::Lines => "text has a line break; keep one fact on one line",
            Self::Secret => {
                "text looks like a password, key, card or account number; never remember secrets"
            }
        }
    }
}

/// A fact worth keeping is one short line with no secret in it.
pub fn check_memory(text: &str) -> Result<(), WorkMemoryFault> {
    let text = text.trim();
    if text.is_empty() {
        return Err(WorkMemoryFault::Empty);
    }
    if text.chars().count() > MAX_WORK_MEMORY_CHARS {
        return Err(WorkMemoryFault::TooLong);
    }
    if text.chars().any(char::is_control) {
        return Err(WorkMemoryFault::Lines);
    }
    let lower = text.to_lowercase();
    const WORDS: &[&str] = &[
        "password",
        "passcode",
        "passphrase",
        "api key",
        "api_key",
        "apikey",
        "secret key",
        "private key",
        "access token",
        "bearer ",
        "one-time code",
        "2fa code",
        "recovery code",
        "seed phrase",
        "pin code",
        "cvv",
        "cvc",
        "iban",
        "social security",
        "hasło",
    ];
    if WORDS.iter().any(|word| lower.contains(word)) {
        return Err(WorkMemoryFault::Secret);
    }
    let digits = text.chars().filter(char::is_ascii_digit).count();
    let mut run = 0;
    let mut longest = 0;
    for c in text.chars() {
        if c.is_ascii_digit() {
            run += 1;
            longest = longest.max(run);
        } else if !matches!(c, ' ' | '-') {
            run = 0;
        }
    }
    let tokenlike = text.split_whitespace().any(|word| {
        let word = word.trim_matches(|c: char| !c.is_alphanumeric());
        word.len() >= 20
            && word.chars().any(|c| c.is_ascii_digit())
            && word.chars().any(|c| c.is_ascii_alphabetic())
            && !word.contains('.')
    });
    if longest >= 12 || digits >= 16 || tokenlike {
        return Err(WorkMemoryFault::Secret);
    }
    Ok(())
}

impl WorkPersonalRequest {
    pub fn validate(&self) -> Result<(), WorkError> {
        let fact = |text: &str| check_memory(text).map_err(|_| WorkError::Invalid);
        match self {
            Self::Memories { query, limit, .. } => {
                if *limit == 0
                    || *limit > MAX_WORK_MEMORY_PAGE
                    || query
                        .as_ref()
                        .is_some_and(|q| q.len() > MAX_WORK_RECALL_QUERY_BYTES)
                {
                    return Err(WorkError::Invalid);
                }
                Ok(())
            }
            Self::Remember { id, text, .. } | Self::Edit { id, text, .. } => {
                if !valid_memory_id(id) {
                    return Err(WorkError::Invalid);
                }
                fact(text)
            }
            Self::Forget { id } => valid_memory_id(id).then_some(()).ok_or(WorkError::Invalid),
            Self::Used { ids, .. } => (ids.len() <= 64 && ids.iter().all(|id| valid_memory_id(id)))
                .then_some(())
                .ok_or(WorkError::Invalid),
            Self::SearchHistory { query, limit } => {
                if query.trim().is_empty()
                    || query.len() > MAX_WORK_RECALL_QUERY_BYTES
                    || *limit == 0
                    || *limit > MAX_WORK_HISTORY_HITS
                {
                    return Err(WorkError::Invalid);
                }
                Ok(())
            }
            Self::RecentHistory {
                since,
                until,
                limit,
            } => (since < until && *limit > 0 && *limit <= MAX_WORK_RECENT_HISTORY)
                .then_some(())
                .ok_or(WorkError::Invalid),
            Self::ForgetAll | Self::Consent { .. } => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_memory_is_one_short_line_and_never_a_secret() {
        for fact in [
            "Prefers aisle seats on long flights",
            "Anna Kowalska leads design at Quasar",
            "Works from Warsaw, CET",
            "Budget for the YC trip is about $6,000",
            "Flies LOT when it's direct",
        ] {
            assert_eq!(check_memory(fact), Ok(()), "{fact}");
        }
        for (fact, fault) in [
            ("", WorkMemoryFault::Empty),
            ("two\nlines", WorkMemoryFault::Lines),
            ("My password is hunter2", WorkMemoryFault::Secret),
            ("Card 4242 4242 4242 4242", WorkMemoryFault::Secret),
            ("Key sk1proj9AbCdEfGhIjKlMnOp", WorkMemoryFault::Secret),
            ("PESEL 850101123456", WorkMemoryFault::Secret),
        ] {
            assert_eq!(check_memory(fact), Err(fault), "{fact}");
        }
        assert_eq!(
            check_memory(&"a".repeat(MAX_WORK_MEMORY_CHARS + 1)),
            Err(WorkMemoryFault::TooLong)
        );
    }

    #[test]
    fn requests_are_bounded() {
        let id = ulid::Ulid::new().to_string();
        let remember = |text: &str| WorkPersonalRequest::Remember {
            id: id.clone(),
            text: text.into(),
            kind: WorkMemoryKindV1::Preference,
            work: None,
            execution: None,
            now_ms: 1,
        };
        assert!(remember("Prefers window seats").validate().is_ok());
        assert!(remember("password: x").validate().is_err());
        assert!(WorkPersonalRequest::Forget { id: "x".into() }
            .validate()
            .is_err());
        assert!(WorkPersonalRequest::SearchHistory {
            query: " ".into(),
            limit: 5
        }
        .validate()
        .is_err());
        assert!(WorkPersonalRequest::Memories {
            query: None,
            work: None,
            limit: 0
        }
        .validate()
        .is_err());
    }
}
