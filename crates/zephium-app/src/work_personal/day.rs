//! Where the person's day lives: their connections, the daily apps they use
//! signed in, and Zephium's own tasks and notes. The first time, one question
//! offers what was found; the answer is remembered and every later day plan
//! reads the same sources without asking.
use serde_json::{json, Value};
use zephium_core::work::{
    parts::WorkInputKindV1,
    personal::{WorkContextSourceV1, WorkMemoryKindV1},
};

use crate::work_lead::tools::{LeadToolContext, LeadToolOutcome};

/// The daily apps a day plan reads, by the name the person knows them by.
pub(crate) struct DayApp {
    pub key: &'static str,
    pub name: &'static str,
    pub host: &'static str,
    /// What the part covers there.
    pub read: &'static str,
}

pub(crate) const APPS: [DayApp; 6] = [
    DayApp {
        key: "calendar",
        name: "Google Calendar",
        host: "calendar.google.com",
        read: "today's events with their times, from the day view",
    },
    DayApp {
        key: "gmail",
        name: "Gmail",
        host: "mail.google.com",
        read: "unread and starred mail from the last two days that asks something of the person",
    },
    DayApp {
        key: "slack",
        name: "Slack",
        host: "app.slack.com",
        read: "unread direct messages, mentions and threads waiting on the person",
    },
    DayApp {
        key: "linear",
        name: "Linear",
        host: "linear.app",
        read: "issues assigned to the person that are due or in progress",
    },
    DayApp {
        key: "notion",
        name: "Notion",
        host: "app.notion.com",
        read: "pages and tasks assigned to the person or changed for them since yesterday",
    },
    DayApp {
        key: "github",
        name: "GitHub",
        host: "github.com",
        read: "review requests, assigned issues and mentions",
    },
];
/// Zephium's own tasks and notes, as the person sees the choice.
const ZEPHIUM: &str = "Quasar tasks and notes";
/// How a remembered answer begins; the lead reads it in its memory too.
const REMEMBERED: &str = "Plans the day from";
const USE_THESE: &str = "Use these";
const ONLY_ZEPHIUM: &str = "Only my Quasar tasks";
/// Visits older than this don't make an app a daily one.
const RECENT_DAYS: i64 = 30;

/// One place the day is read from, and how.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum DaySource {
    /// A daily app, through the person's connection when they have one.
    App {
        key: &'static str,
        connection: Option<String>,
    },
    Zephium,
}

fn app(key: &str) -> Option<&'static DayApp> {
    APPS.iter().find(|app| app.key == key)
}

/// The sources an answer or a remembered line names, in catalogue order.
pub(crate) fn named(text: &str) -> (Vec<&'static str>, bool) {
    let lower = text.to_lowercase();
    let apps = APPS
        .iter()
        .filter(|app| {
            lower.contains(&app.name.to_lowercase())
                || lower.contains(app.key)
                || (app.key == "calendar" && lower.contains("calendar"))
        })
        .map(|app| app.key)
        .collect();
    (apps, lower.contains("zephium") || lower.contains("tasks"))
}

/// "Gmail, Google Calendar and Zephium tasks and notes".
fn listed(names: &[String]) -> String {
    match names {
        [] => String::new(),
        [one] => one.clone(),
        [first @ .., last] => format!("{} and {last}", first.join(", ")),
    }
}

fn source_name(source: &DaySource) -> String {
    match source {
        DaySource::App { key, connection } => {
            let name = app(key).map_or(*key, |app| app.name);
            match connection {
                Some(connection) if connection != name => {
                    format!("{name} (your {connection} connection)")
                }
                Some(_) => format!("{name} (your connection)"),
                None => name.to_owned(),
            }
        }
        DaySource::Zephium => ZEPHIUM.to_owned(),
    }
}

/// The person's connection for a daily app, when they added one.
fn offer(context: &LeadToolContext<'_>, key: &str) -> Option<crate::work_lead::route::Offer> {
    let servers = crate::work_connections::store::shared()
        .and_then(|store| store.servers(&context.profile().to_string()).ok())
        .unwrap_or_default();
    crate::work_lead::route::offer(
        key,
        crate::work_connections::helper::shared().gh_ready(),
        &servers,
    )
}
fn connection(context: &LeadToolContext<'_>, key: &str) -> Option<String> {
    offer(context, key).map(|offer| offer.connection)
}

/// Daily apps the person has a connection for or visited lately, by host
/// only; never what the pages held.
async fn detect(context: &LeadToolContext<'_>) -> Vec<DaySource> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    let mut found = Vec::new();
    for app in &APPS {
        let connection = connection(context, app.key);
        let visited = connection.is_some()
            || super::history(context.handle(), context.profile(), app.host.into(), 8)
                .await
                .unwrap_or_default()
                .iter()
                .any(|hit| {
                    url::Url::parse(&hit.url)
                        .ok()
                        .and_then(|url| url.host_str().map(str::to_owned))
                        .is_some_and(|host| host == app.host)
                        && now - hit.last_visit <= RECENT_DAYS * 86_400
                });
        if visited {
            found.push(DaySource::App {
                key: app.key,
                connection,
            });
        }
    }
    found.push(DaySource::Zephium);
    found
}

/// The remembered choice, when there is one.
async fn remembered(context: &LeadToolContext<'_>) -> Option<(Vec<&'static str>, bool)> {
    let facts = super::memories(
        context.handle(),
        context.profile(),
        Some(REMEMBERED.into()),
        None,
        8,
    )
    .await
    .ok()?;
    let fact = facts.iter().find(|f| f.text.starts_with(REMEMBERED))?;
    super::used(context.handle(), context.profile(), vec![fact.id.clone()]).await;
    Some(named(&fact.text))
}

/// `day_sources`: the remembered sources, or the detected ones offered once.
pub(crate) async fn day_sources(
    context: LeadToolContext<'_>,
    _arguments: &Value,
) -> LeadToolOutcome {
    // Apps the person named ("check my Slack") are the sources: nothing
    // else is offered, asked or remembered.
    let said = named(&context.run.said());
    let remembered = match said.0.is_empty() {
        true => remembered(&context).await,
        false => None,
    };
    let (keys, zephium) = match remembered {
        _ if !said.0.is_empty() => said,
        Some(choice) => {
            context
                .input(super::tools::input(
                    WorkInputKindV1::Memory,
                    "Your day sources",
                    None,
                ))
                .await;
            choice
        }
        None => {
            let detected = detect(&context).await;
            let names: Vec<String> = detected.iter().map(source_name).collect();
            let prompt = format!("Plan your day from {}?", listed(&names));
            let answer =
                match context
                    .ask(prompt, vec![USE_THESE.into(), ONLY_ZEPHIUM.into()])
                    .await
                {
                    Ok(Some(answer)) => answer,
                    _ => return LeadToolOutcome::ok(
                        "The person didn't answer; plan from their Quasar tasks and notes only.",
                    ),
                };
            let answer = answer.trim();
            let (keys, zephium) = if answer.eq_ignore_ascii_case(USE_THESE) {
                (
                    detected
                        .iter()
                        .filter_map(|source| match source {
                            DaySource::App { key, .. } => Some(*key),
                            DaySource::Zephium => None,
                        })
                        .collect(),
                    true,
                )
            } else if answer.eq_ignore_ascii_case(ONLY_ZEPHIUM) {
                (Vec::new(), true)
            } else {
                named(answer)
            };
            let mut chosen: Vec<String> = keys
                .iter()
                .filter_map(|key| app(key).map(|app| app.name.to_owned()))
                .collect();
            if zephium {
                chosen.push(ZEPHIUM.into());
            }
            if !chosen.is_empty() {
                let _ = super::remember(
                    context.handle(),
                    context.profile(),
                    format!("{REMEMBERED} {}", listed(&chosen)),
                    WorkMemoryKindV1::Preference,
                    None,
                    Some(context.execution()),
                )
                .await;
            }
            (keys, zephium)
        }
    };
    if zephium {
        // Choosing Zephium's tasks and notes answers the notes question.
        let _ = super::consent(
            context.handle(),
            context.profile(),
            context.work(),
            WorkContextSourceV1::Notes,
            Some(true),
        )
        .await;
    }
    let mut sources = Vec::new();
    for key in keys {
        let Some(app) = app(key) else { continue };
        let offer = offer(&context, key);
        if let Some(offer) = &offer {
            context.run.accept_connection(&offer.yes);
        }
        let connection = offer.map(|offer| offer.connection);
        sources.push(match connection {
            Some(connection) => json!({
                "name": app.name, "part": {"title": app.name, "helper": "connection", "service": connection},
                "read": app.read
            }),
            None => json!({
                "name": app.name,
                "part": {
                    "title": app.name, "helper": "browser", "service": app.host,
                    "brief": format!("Browse https://{}/ as it is, with mine and view set: the app opens on the view to read from and its rows come back at once. Open an item only when its row lacks what the plan needs.", app.host)
                },
                "read": app.read
            }),
        });
    }
    if zephium {
        sources.push(json!({"name": ZEPHIUM, "read": "list_tasks (open tasks, overdue and today first), then search_notes for today's date or meeting names when a note would help"}));
    }
    if sources.is_empty() {
        return LeadToolOutcome::ok(
            "The person chose no source; plan from what they said in the request.",
        );
    }
    LeadToolOutcome::ok(
        json!({
            "sources": sources,
            "how": "Start one part per app in one turn, with the part given, and read Quasar's tasks yourself meanwhile. Never ask about these sources again."
        })
        .to_string(),
    )
}

/// Today in the person's own time zone, as `YYYY-MM-DD`.
pub fn local_day() -> String {
    #[cfg(unix)]
    {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as libc::time_t)
            .unwrap_or(0);
        let mut tm: libc::tm = unsafe { std::mem::zeroed() };
        // SAFETY: localtime_r reads `now` and writes only into `tm`.
        if !unsafe { libc::localtime_r(&now, &mut tm) }.is_null() {
            return format!(
                "{:04}-{:02}-{:02}",
                tm.tm_year + 1900,
                tm.tm_mon + 1,
                tm.tm_mday
            );
        }
    }
    let days = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64 / 86_400);
    super::tools::day(days * 86_400_000)
}

/// Unix seconds of local midnight `days_ago` days back (0 is today's).
pub(super) fn local_midnight(days_ago: u32) -> i64 {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    #[cfg(unix)]
    {
        let at = now as libc::time_t;
        let mut tm: libc::tm = unsafe { std::mem::zeroed() };
        // SAFETY: localtime_r reads `at` and writes only into `tm`; mktime
        // reads and normalizes only `tm`.
        if !unsafe { libc::localtime_r(&at, &mut tm) }.is_null() {
            tm.tm_hour = 0;
            tm.tm_min = 0;
            tm.tm_sec = 0;
            tm.tm_mday -= days_ago as i32;
            tm.tm_isdst = -1;
            let midnight = unsafe { libc::mktime(&mut tm) };
            if midnight >= 0 {
                return midnight as i64;
            }
        }
    }
    now - now.rem_euclid(86_400) - i64::from(days_ago) * 86_400
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn midnights_step_back_by_days() {
        let today = local_midnight(0);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        assert!(today <= now && now - today < 25 * 3600);
        let yesterday = local_midnight(1);
        assert!((82_800..=90_000).contains(&(today - yesterday)));
    }

    #[test]
    fn an_answer_names_its_sources() {
        assert_eq!(
            named("Gmail and my calendar, plus Linear"),
            (vec!["calendar", "gmail", "linear"], false)
        );
        assert_eq!(
            named("Plans the day from Slack and Quasar tasks and notes"),
            (vec!["slack"], true)
        );
        assert_eq!(
            listed(&["Gmail".into(), "Slack".into(), ZEPHIUM.into()]),
            "Gmail, Slack and Quasar tasks and notes"
        );
        assert_eq!(
            source_name(&DaySource::App {
                key: "slack",
                connection: Some("Slack".into())
            }),
            "Slack (your connection)"
        );
    }
}
