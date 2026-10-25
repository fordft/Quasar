//! Public-only qualification through the real shell actor and desktop adapter.

use std::{
    io::Write as _,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use zephium_agent_controller::AgentWorkEventKind;
use zephium_agent_provider_transport::{
    load_macos_probe_openai_credential, AgentProviderTransportConfig,
};
use zephium_agent_runtime::{AgentRuntimeConfig, AgentRuntimeStopReason};
use zephium_app::{AgentWorkApplicationConfig, AgentWorkApplicationPhase, ShutdownOutcome};
use zephium_work_composition::{MacosWorkComposition, TrustedWorkRequest};

pub(super) use super::loopback_support::NoChrome;
pub(super) fn run() -> Result<(), super::ProbeFailure> {
    run_mode(Qualification::Actions)
}

pub(super) fn run_extraction() -> Result<(), super::ProbeFailure> {
    run_mode(Qualification::Extraction)
}

pub(super) fn run_artifact() -> Result<(), super::ProbeFailure> {
    run_mode(Qualification::Artifact)
}

pub(super) fn run_cancellation() -> Result<(), super::ProbeFailure> {
    run_mode(Qualification::CancelExtraction)
}

pub(super) fn run_sequential() -> Result<(), super::ProbeFailure> {
    run_mode(Qualification::Sequential)
}

pub(super) fn run_combined() -> Result<(), super::ProbeFailure> {
    run_mode(Qualification::ActionsAndExtraction)
}

pub(super) fn run_scoped() -> Result<(), super::ProbeFailure> {
    run_mode(Qualification::ActionsAndScopedExtraction)
}

pub(super) fn run_review() -> Result<(), super::ProbeFailure> {
    run_mode(Qualification::ReviewAndFreshActions)
}

pub(super) fn run_read() -> Result<(), super::ProbeFailure> {
    run_mode(Qualification::ReadActions)
}

pub(super) fn run_site(site: super::work_sites::Site) -> Result<(), super::ProbeFailure> {
    writeln!(
        std::io::stdout().lock(),
        "work-site: site={}; task=closed_public_cited_read; effects=read_only; content=redacted",
        site.name()
    )
    .map_err(|_| super::ProbeFailure::Output)?;
    run_mode(Qualification::Site(site))
}

#[derive(Clone, Copy)]
enum Qualification {
    Site(super::work_sites::Site),
    ReadActions,
    Actions,
    Extraction,
    Artifact,
    CancelExtraction,
    Sequential,
    ActionsAndExtraction,
    ActionsAndScopedExtraction,
    ReviewAndFreshActions,
}

fn run_mode(mode: Qualification) -> Result<(), super::ProbeFailure> {
    use super::ProbeFailure as Error;
    let review = matches!(mode, Qualification::ReviewAndFreshActions);
    let site = match mode {
        Qualification::Site(site) => Some(site),
        _ => None,
    };
    let inspecting = matches!(mode, Qualification::ReadActions);
    let extraction = !matches!(
        mode,
        Qualification::Actions | Qualification::ReviewAndFreshActions | Qualification::ReadActions
    );
    let durable = matches!(mode, Qualification::Artifact);
    let sequential = matches!(
        mode,
        Qualification::Sequential | Qualification::ReviewAndFreshActions
    );
    let scoped = matches!(mode, Qualification::ActionsAndScopedExtraction);
    let combined = matches!(
        mode,
        Qualification::ActionsAndExtraction | Qualification::ActionsAndScopedExtraction
    );
    let cancel_after_turn = matches!(
        mode,
        Qualification::CancelExtraction | Qualification::Sequential
    );
    let started = Instant::now();
    let (profile, input, task) = if let Some(site) = site {
        super::work_actor::site_input(started, site)?
    } else if inspecting {
        super::work_actor::read_input(started)?
    } else if review {
        super::work_actor::review_input(started)?
    } else if scoped {
        super::work_actor::scoped_input(started)?
    } else if combined {
        super::work_actor::combined_input(started)?
    } else if durable {
        super::work_actor::artifact_input(started)?
    } else if extraction {
        super::work_actor::extraction_input(started)?
    } else {
        super::work_actor::input(started)?
    };
    let data = tempfile::Builder::new()
        .prefix("zephium-public-work-")
        .tempdir()
        .map_err(|_| Error::Runtime)?;
    let store =
        Arc::new(zephium_store::SqliteStore::open(data.path()).map_err(|_| Error::Runtime)?);
    if durable {
        register_public_profile(&store, profile)?;
    }
    let blocker = seeded_blocker(data.path())?;
    let request = TrustedWorkRequest::new(
        input,
        AgentWorkApplicationConfig::new(
            AgentRuntimeConfig::STANDARD,
            AgentProviderTransportConfig::STANDARD,
        ),
        load_macos_probe_openai_credential().map_err(|_| Error::Keychain)?,
        task,
    );
    let result = zephium_engine::run_macos_agentic_work_application_probe(profile, move |engine| {
        let failed = Arc::new(AtomicBool::new(false));
        let fail_sink = failed.clone();
        let shell = zephium_app::spawn_suspended(
            engine.clone(),
            store.clone(),
            blocker,
            Box::new(move |_| {
                fail_sink.store(true, Ordering::Release);
            }),
            Arc::new(NoChrome),
            Box::new(|_| {}),
        )
        .map_err(|_| "application_shell_spawn")?;
        let composition = MacosWorkComposition::new(engine.clone(), store.clone());
        let prepared = composition
            .prepare_public_qualification(request)
            .map_err(|_| "application_prepare")?;
        let mut view = composition
            .attach(&shell.callback_handle())
            .ok_or("application_attach")?;
        view.admit(prepared).map_err(|_| "application_admit")?;
        if !shell.admit_startup() {
            return Err("application_startup");
        }
        let mut shutdown: Option<
            std::thread::JoinHandle<Result<ShutdownOutcome, std::sync::mpsc::RecvTimeoutError>>,
        > = None;
        let mut terminal_success = false;
        let mut terminal_cancelled = false;
        let mut cancellation_requested = false;
        let mut review_requested = false;
        let mut terminal_reviewed = false;
        let mut lifetime = 1_u8;
        let mut predecessor: Option<(
            zephium_app::AgentWorkApplicationHandle,
            zephium_agentic::AgentWorkRecord,
        )> = None;
        let mut stale_control_sent = false;
        let mut native_actions = 0_u32;
        let mut baseline_reads = 0_u32;
        let mut navigation_proposals = 0_u32;
        let mut terminal_observed = false;
        let mut archive_requested = false;
        let mut archive_verified = false;
        let mut cleanup = None;
        let (mut turns, mut effects, mut tokens_in, mut tokens_out, mut cost) =
            (0_u32, 0_u32, 0_u64, 0_u64, 0_u64);
        Ok(Box::new(move |native_failed| {
            let snapshot = view.snapshot();
            if native_failed || failed.load(Ordering::Acquire) {
                if let Some(run) = snapshot.run {
                    let _ = view.stop(run, AgentRuntimeStopReason::Cancelled);
                }
            }
            while let Some(event) = view.take_event() {
                let mut output = std::io::stdout().lock();
                let written = match event.kind() {
                    AgentWorkEventKind::ModelSettled {
                        call,
                        input_tokens,
                        output_tokens,
                        request_bytes,
                        semantic_bytes,
                        cost_micro_usd,
                        accounting,
                        elapsed_millis,
                        ..
                    } => {
                        turns += 1;
                        tokens_in += input_tokens;
                        tokens_out += output_tokens;
                        cost += cost_micro_usd;
                        writeln!(output, "work-application-turn: call={}; input_tokens={input_tokens}; output_tokens={output_tokens}; request_bytes={request_bytes}; semantic_bytes={semantic_bytes}; cost_micro_usd={cost_micro_usd}; accounting={accounting:?}; turn_ms={elapsed_millis}; wall_ms={}; content=redacted", call.get(), event.elapsed_millis())
                    }
                    kind => {
                        if matches!(kind, AgentWorkEventKind::ActionActive) {
                            native_actions += 1;
                        }
                        if matches!(kind, AgentWorkEventKind::Verified) {
                            effects += 1;
                        }
                        if matches!(
                            kind,
                            AgentWorkEventKind::ToolProposed(
                                zephium_agentic::AgentBrowserToolKind::Read
                            )
                        ) {
                            baseline_reads += 1;
                        }
                        if matches!(
                            kind,
                            AgentWorkEventKind::ToolProposed(
                                zephium_agentic::AgentBrowserToolKind::Navigate
                            )
                        ) {
                            navigation_proposals += 1;
                        }
                        writeln!(output, "work-application-event: sequence={}; phase={kind:?}; wall_ms={}; content=redacted", event.sequence(), event.elapsed_millis())
                    }
                };
                if written.is_err() {
                    failed.store(true, Ordering::Release);
                }
            }
            // Explicit trusted test control after one real settled Luna turn.
            // This is the same application stop port as a human takeover; no
            // provider or native fault is synthesized and no request is retried.
            if cancel_after_turn && lifetime == 1 && turns > 0 && !cancellation_requested {
                cancellation_requested = snapshot
                    .run
                    .is_some_and(|run| view.stop(run, AgentRuntimeStopReason::HumanTakeover));
                if !cancellation_requested {
                    failed.store(true, Ordering::Release);
                }
            }
            if review
                && lifetime == 1
                && snapshot.phase == AgentWorkApplicationPhase::NeedsReview
                && !native_failed
                && !failed.load(Ordering::Acquire)
            {
                if !review_requested {
                    let Some(record) = view.records().into_iter().find(|record| {
                        record.disposition() == zephium_agentic::AgentWorkDisposition::NeedsApproval
                            && record.debt() == zephium_agentic::AgentWorkDebt::NONE
                    }) else {
                        return Some(Err("review_debt"));
                    };
                    if turns == 0
                        || native_actions != 0
                        || effects != 0
                        || snapshot.persistence_failure.is_some()
                    {
                        return Some(Err("review_unissued"));
                    }
                    // Explicit public fixture decision through the application
                    // review port. It never executes the refused proposal.
                    review_requested = view.review(
                        record,
                        zephium_app::AgentWorkReviewDecision::AcceptFreshAdmission,
                    );
                    if !review_requested {
                        return Some(Err("review_mailbox"));
                    }
                }
                return None;
            }
            if lifetime == 2
                && snapshot.phase == AgentWorkApplicationPhase::Running
                && !stale_control_sent
            {
                stale_control_sent = predecessor.as_ref().is_some_and(|(old, record)| {
                    old.snapshot().run.is_some_and(|run| {
                        old.records().contains(record)
                            && old.stop(run, AgentRuntimeStopReason::HumanTakeover)
                    })
                });
                if !stale_control_sent {
                    failed.store(true, Ordering::Release);
                }
            }
            if shutdown.is_none()
                && (matches!(
                    snapshot.phase,
                    AgentWorkApplicationPhase::Succeeded
                        | AgentWorkApplicationPhase::Failed
                        | AgentWorkApplicationPhase::Cancelled
                        | AgentWorkApplicationPhase::Recovery
                        | AgentWorkApplicationPhase::NeedsReview
                        | AgentWorkApplicationPhase::Reviewed
                        | AgentWorkApplicationPhase::PersistenceUncertain
                ) || native_failed
                    || failed.load(Ordering::Acquire))
            {
                if !terminal_observed {
                    terminal_observed = true;
                    terminal_success = snapshot.phase == AgentWorkApplicationPhase::Succeeded
                        && snapshot.failure.is_none()
                        && view.records().iter().any(|record| {
                            record.disposition() == zephium_agentic::AgentWorkDisposition::Succeeded
                        });
                    terminal_cancelled = cancel_after_turn
                        && lifetime == 1
                        && cancellation_requested
                        && snapshot.phase == AgentWorkApplicationPhase::Cancelled
                        && snapshot.failure
                            == Some(zephium_agent_controller::AgentWorkFailure::HumanTakeover)
                        && snapshot.persistence_failure.is_none()
                        && turns > 0
                        && native_actions == 0
                        && effects == 0
                        && view.take_extraction().is_none()
                        && view.records().iter().any(|record| {
                            record.disposition() == zephium_agentic::AgentWorkDisposition::Cancelled
                                && record.debt() == zephium_agentic::AgentWorkDebt::NONE
                        });
                    terminal_reviewed = review
                        && lifetime == 1
                        && review_requested
                        && snapshot.phase == AgentWorkApplicationPhase::Reviewed
                        && snapshot.persistence_failure.is_none()
                        && matches!(snapshot.last_review, Some(Ok(record)) if record.disposition() == zephium_agentic::AgentWorkDisposition::FreshAdmissionRequired && record.debt() == zephium_agentic::AgentWorkDebt::NONE)
                        && turns > 0
                        && native_actions == 0
                        && effects == 0;
                    if review && lifetime == 2 {
                        terminal_success &= effects == 3 && native_actions == 3;
                    }
                    if inspecting {
                        terminal_success &=
                            baseline_reads > 0 && native_actions == 3 && effects == 3;
                    }
                    if extraction && terminal_success {
                        terminal_success = view.take_extraction().is_some_and(|result| {
                        let verified = if let Some(site) = site {
                            native_actions == 0 && effects == 0 && baseline_reads == 0
                                && navigation_proposals == site.navigation_proposals()
                                && site.verify_owned(&result)
                        } else if combined {
                            effects >= 3 && effects == native_actions && verify_prepared_result(&result)
                        } else {
                            verify_extraction(&result)
                        };
                        let stats = result.stats();
                        let _ = writeln!(std::io::stdout().lock(), "work-application-result: fields={}; values={}; source_edges={}; independently_verified={verified}; artifact_promised={durable}; content=redacted", stats.fields(), stats.values(), stats.source_edges());
                        verified && view.take_extraction().is_none()
                    });
                    }
                }
                if sequential
                    && lifetime == 1
                    && (terminal_cancelled || terminal_reviewed)
                    && !native_failed
                    && !failed.load(Ordering::Acquire)
                {
                    let Some(record) = view.records().into_iter().find(|record| {
                        record.disposition()
                            == if review {
                                zephium_agentic::AgentWorkDisposition::FreshAdmissionRequired
                            } else {
                                zephium_agentic::AgentWorkDisposition::Cancelled
                            }
                    }) else {
                        return Some(Err("successor_prior_record"));
                    };
                    let Ok((next_profile, input, task)) = (if review {
                        super::work_actor::input(Instant::now())
                    } else {
                        super::work_actor::extraction_input(Instant::now())
                    }) else {
                        return Some(Err("successor_input"));
                    };
                    if next_profile != profile {
                        return Some(Err("successor_profile"));
                    }
                    let Ok(credential) = load_macos_probe_openai_credential() else {
                        return Some(Err("successor_keychain"));
                    };
                    let request = TrustedWorkRequest::new(
                        input,
                        AgentWorkApplicationConfig::new(
                            AgentRuntimeConfig::STANDARD,
                            AgentProviderTransportConfig::STANDARD,
                        ),
                        credential,
                        task,
                    );
                    let Ok(prepared) = composition.prepare_public_qualification(request) else {
                        return Some(Err("successor_prepare"));
                    };
                    let Some(next) = composition.attach_successor(&shell.callback_handle(), &view)
                    else {
                        return Some(Err("successor_attachment"));
                    };
                    if next.admit(prepared).is_err() {
                        return Some(Err("successor_admit"));
                    }
                    let _ = writeln!(std::io::stdout().lock(), "work-application-predecessor: lifetime={lifetime}; cancelled_closed={terminal_cancelled}; reviewed_closed={terminal_reviewed}; turns={turns}; input_tokens={tokens_in}; output_tokens={tokens_out}; cost_micro_usd={cost}; native_actions={native_actions}; verified_effects={effects}; wall_ms={}; content=redacted", started.elapsed().as_millis());
                    predecessor = Some((std::mem::replace(&mut view, next), record));
                    lifetime = 2;
                    terminal_observed = false;
                    terminal_cancelled = false;
                    terminal_reviewed = false;
                    terminal_success = false;
                    (turns, effects, tokens_in, tokens_out, cost) = (0, 0, 0, 0, 0);
                    return None;
                }
                if durable && terminal_success && !archive_verified {
                    if !archive_requested {
                        archive_requested = view
                            .records()
                            .into_iter()
                            .find(|record| {
                                record.disposition()
                                    == zephium_agentic::AgentWorkDisposition::Succeeded
                            })
                            .is_some_and(|record| view.read_artifact(record, profile));
                        if archive_requested {
                            return None;
                        }
                        terminal_success = false;
                    } else if let Some(archive) = view.take_archived_extraction() {
                        archive_verified = verify_archive(&archive);
                        terminal_success &= archive_verified;
                        let _ = writeln!(std::io::stdout().lock(), "work-application-artifact: body_bytes={}; fields={}; independently_verified={archive_verified}; publication=atomic; content=redacted", archive.descriptor().bytes(), archive.fields().len());
                    } else if snapshot.artifact_read.is_some() {
                        terminal_success = false;
                    } else {
                        return None;
                    }
                }
                if durable {
                    if cleanup.is_none() {
                        cleanup = Some(super::work_artifact_cleanup::Cleanup::start(
                            store.clone(),
                            engine.clone(),
                            profile,
                        ));
                    }
                    let erased = cleanup.as_mut().and_then(|cleanup| cleanup.poll())?;
                    terminal_success &= archive_verified && erased;
                }
                if sequential {
                    terminal_success &= lifetime == 2
                        && stale_control_sent
                        && predecessor.as_ref().is_some_and(|(old, record)| {
                            (if review {
                                old.snapshot().phase == AgentWorkApplicationPhase::Reviewed
                                    && matches!(old.snapshot().last_review, Some(Ok(reviewed)) if reviewed == *record)
                            } else {
                                old.snapshot().phase == AgentWorkApplicationPhase::Cancelled
                                    && old.snapshot().failure == Some(zephium_agent_controller::AgentWorkFailure::HumanTakeover)
                            })
                                && old.snapshot().run != snapshot.run
                                && old.records() == [*record]
                                && view.records().len() == 2
                                && view.records().contains(record)
                                && view.take_extraction().is_none()
                        });
                    let stale_control_ignored = terminal_success && stale_control_sent;
                    let _ = writeln!(std::io::stdout().lock(), "work-application-successor: lifetimes={lifetime}; exact_predecessor_preserved={terminal_success}; stale_control_ignored={stale_control_ignored}; content=redacted");
                }
                let _ = writeln!(std::io::stdout().lock(), "work-application-terminal: phase={:?}; failure={:?}; persistence={:?}; durable_success={terminal_success}; cancelled_closed={terminal_cancelled}; content=redacted", snapshot.phase, snapshot.failure, snapshot.persistence_failure);
                let request = shell.shutdown_with_deadline(Instant::now() + Duration::from_secs(8));
                shutdown = std::thread::Builder::new()
                    .name("work-qualifier-join".into())
                    .spawn(move || request.recv_until_deadline())
                    .ok();
                if shutdown.is_none() {
                    return Some(Err("application_join_spawn"));
                }
            }
            if shutdown.as_ref().is_some_and(|join| join.is_finished()) {
                let clean = shutdown
                    .take()
                    .is_some_and(|join| matches!(join.join(), Ok(Ok(ShutdownOutcome::Clean))));
                if inspecting {
                    let _ = writeln!(std::io::stdout().lock(), "work-application-read: baseline_reads={baseline_reads}; fresh_capture=false; new_reference_authority=false; content=redacted");
                }
                if site.is_some() {
                    let _ = writeln!(std::io::stdout().lock(), "work-site-tools: navigation_proposals={navigation_proposals}; baseline_reads={baseline_reads}; native_actions={native_actions}; content=redacted");
                }
                let _ = writeln!(std::io::stdout().lock(), "work-application-closure: model=gpt-5.6-luna; durable_success={terminal_success}; cancelled_closed={terminal_cancelled}; shell_clean={clean}; turns={turns}; native_actions={native_actions}; verified_effects={effects}; input_tokens={tokens_in}; output_tokens={tokens_out}; cost_micro_usd={cost}; elapsed_ms={}; content=redacted", started.elapsed().as_millis());
                return Some(
                    if clean
                        && (if sequential {
                            lifetime == 2 && terminal_success
                        } else if cancel_after_turn {
                            terminal_cancelled
                        } else {
                            terminal_success
                        })
                        && !failed.load(Ordering::Acquire)
                    {
                        Ok(())
                    } else {
                        Err("application_closure")
                    },
                );
            }
            None
        }))
    });
    if durable && result.is_err() {
        // Preserve exact Store recovery metadata if native deletion was not
        // qualified. Never discard its only durable obligation on failure.
        let _ = data.keep();
        let _ = writeln!(
            std::io::stdout().lock(),
            "work-artifact-recovery: private_test_directory_retained=true; content=redacted"
        );
    }
    result.map_err(Error::Engine)?;
    writeln!(std::io::stdout().lock(), "work-application-qualified: focus_isolation=passed; teardown=application_owned; elapsed_ms={}; content=redacted", started.elapsed().as_millis()).map_err(|_| Error::Output)
}

fn register_public_profile(
    store: &zephium_store::SqliteStore,
    profile: zephium_core::ids::ProfileId,
) -> Result<(), super::ProbeFailure> {
    use zephium_core::{
        ids::{ProfileId, SpaceId},
        ports::store::Store,
        profiles::ProfileKind,
        session::{PersistedProfile, PersistedSpace, SessionState},
    };
    let default = ProfileId::generate();
    let default_space = SpaceId::generate();
    store.save_session(SessionState {
        profiles: vec![
            PersistedProfile {
                id: default,
                name: "Public test default".into(),
                kind: ProfileKind::Default,
            },
            PersistedProfile {
                id: profile,
                name: "Public Work test".into(),
                kind: ProfileKind::Named,
            },
        ],
        spaces: vec![
            PersistedSpace {
                id: default_space,
                profile: default,
                name: "Public test default".into(),
                look: Default::default(),
            },
            PersistedSpace {
                id: SpaceId::generate(),
                profile,
                name: "Public Work test".into(),
                look: Default::default(),
            },
        ],
        items: vec![],
        active_space: Some(default_space),
        active_item: None,
        splits: None,
        recently_closed: vec![],
    });
    if store.flush() {
        Ok(())
    } else {
        Err(super::ProbeFailure::Runtime)
    }
}

fn verify_archive(archive: &zephium_agentic::AgentWorkArchivedExtraction) -> bool {
    use zephium_agentic::*;
    if archive.trust() != SemanticExtractionTrust::ModelMapped || archive.fields().len() != 1 {
        return false;
    }
    let ArchivedValue::TextList { items, .. } = archive.fields()[0].value() else {
        return false;
    };
    if items.len() != 10 {
        return false;
    }
    let mut ids = std::collections::BTreeSet::new();
    let (mut english, mut german) = (false, false);
    for item in items {
        let [id] = item.source_ids() else {
            return false;
        };
        let Some(source) = archive.source(*id) else {
            return false;
        };
        let ArchivedSourceContent::Text { value } = source.content() else {
            return false;
        };
        if !ids.insert(id) || source.role() != "link" || value != item.value() {
            return false;
        }
        english |= value.contains("English");
        german |= value.contains("Deutsch");
    }
    english && german
}

fn verify_extraction(result: &zephium_agentic::SemanticOwnedExtractionResult) -> bool {
    use zephium_agentic::*;
    if result.trust() != SemanticExtractionTrust::ModelMapped || result.fields().len() != 1 {
        return false;
    }
    let SemanticExtractedValue::TextList(list) = result.fields()[0].value() else {
        return false;
    };
    if list.items().len() != 10 {
        return false;
    }
    let mut ids = std::collections::BTreeSet::new();
    let mut english = false;
    let mut german = false;
    for item in list.items() {
        let Some(mut sources) = result.sources(item.source_span()) else {
            return false;
        };
        let Some(source) = sources.next() else {
            return false;
        };
        if sources.next().is_some() || source.role != SemanticRole::Link || !ids.insert(source.id) {
            return false;
        }
        let SemanticOwnedReadContent::Text(text) = &source.content else {
            return false;
        };
        if text != item.as_str() {
            return false;
        }
        english |= text.contains("English");
        german |= text.contains("Deutsch");
    }
    english && german
}

fn verify_prepared_result(result: &zephium_agentic::SemanticOwnedExtractionResult) -> bool {
    use zephium_agentic::*;
    if result.trust() != SemanticExtractionTrust::ModelMapped || result.fields().len() != 1 {
        return false;
    }
    let SemanticExtractedValue::Text(value) = result.fields()[0].value() else {
        return false;
    };
    let Some(mut sources) = result.sources(value.source_span()) else {
        return false;
    };
    let Some(source) = sources.next() else {
        return false;
    };
    let SemanticOwnedReadContent::ValuePreview {
        text,
        source_bytes,
        truncated,
    } = &source.content
    else {
        return false;
    };
    value.as_str() == "Quasar open source browser"
        && text == value.as_str()
        && *source_bytes == text.len()
        && !truncated
        && matches!(source.role, SemanticRole::Searchbox | SemanticRole::Textbox)
        && source.snapshot != SemanticSnapshotGeneration::INITIAL
        && sources.next().is_none()
}

// The merged product enables protection by default. Give isolated probes the
// same authenticated bundled rules so Work policy readiness can settle.
pub(super) use super::loopback_support::seeded_blocker;
