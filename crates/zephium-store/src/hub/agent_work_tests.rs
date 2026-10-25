use super::*;
use zephium_agentic::{AgentWorkDebt, AgentWorkJournalMutation};

static SERIAL: Mutex<()> = Mutex::new(());
#[cfg(windows)]
static WINDOWS_SESSIONS: Mutex<
    Vec<(
        std::path::PathBuf,
        zephium_private_fs::NativeWorkStorageTestSession,
    )>,
> = Mutex::new(Vec::new());
pub(crate) struct WorkTestGuard {
    _serial: std::sync::MutexGuard<'static, ()>,
}
pub(crate) fn work_test_guard() -> WorkTestGuard {
    WorkTestGuard {
        _serial: SERIAL.lock().unwrap_or_else(|error| error.into_inner()),
    }
}
impl Drop for WorkTestGuard {
    fn drop(&mut self) {
        simulate_process_exit();
        #[cfg(windows)]
        for (_, session) in std::mem::take(
            &mut *WINDOWS_SESSIONS
                .lock()
                .unwrap_or_else(|error| error.into_inner()),
        ) {
            session
                .retire()
                .expect("owned Windows journal test session retirement");
        }
    }
}

#[cfg(windows)]
pub(crate) fn work_test_storage(directory: &std::path::Path) -> super::super::WindowsWorkStorage {
    use sha2::{Digest, Sha256};
    let directory = std::fs::canonicalize(directory).unwrap();
    let digest = Sha256::digest(directory.to_string_lossy().as_bytes());
    let label = format!(
        "store-test-{}",
        digest[..16]
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    );
    // The parent alone creates and retires the scope. Child processes derive
    // the same selector and retain their original lease until actual OS exit.
    if std::env::var_os("ZEPHIUM_TEST_WORK_DIRECTORY").is_none() {
        let mut sessions = WINDOWS_SESSIONS
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if !sessions.iter().any(|(known, _)| known == &directory) {
            let session = zephium_private_fs::NativeSession::new(&label).unwrap();
            sessions.push((
                directory,
                zephium_private_fs::NativeWorkStorageTestSession::create(&session).unwrap(),
            ));
        }
    }
    super::super::WindowsWorkStorage::for_application("app.quasar.webext-qa", Some(&label)).unwrap()
}

pub(crate) fn open_hub(directory: impl AsRef<std::path::Path>) -> rusqlite::Result<Hub> {
    #[cfg(windows)]
    return Hub::open_with_windows_work_storage(
        directory.as_ref().into(),
        work_test_storage(directory.as_ref()),
    );
    #[cfg(not(windows))]
    Hub::open(directory.as_ref().into())
}

pub(super) fn journal(hub: &mut Hub) -> &mut Connection {
    #[cfg(windows)]
    if hub.windows_work_storage.is_some() {
        return hub.windows_work_database().unwrap().test_connection();
    }
    &mut hub.meta
}
pub(super) fn simulate_process_exit() {
    let mut fence = PROCESS_WORK_FENCE.lock().unwrap();
    if let Some(owner) = fence.as_ref() {
        assert_eq!(Arc::strong_count(owner), 1, "fixture still owns a live Hub");
    }
    fence.take();
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum Fault {
    BeforeWrite,
    AfterWrite,
    AfterArtifactWrite,
    AfterCommit,
    RestartPartial,
}
thread_local! { pub(super) static FAULT: std::cell::Cell<Option<Fault>> = const { std::cell::Cell::new(None) }; }
pub(super) fn fault(expected: Fault) -> bool {
    FAULT.with(|fault| {
        if fault.get() == Some(expected) {
            fault.set(None);
            true
        } else {
            false
        }
    })
}

pub(super) fn initial(owner: AgentWorkIncarnation, key: u16) -> AgentWorkRecord {
    let mut bytes = [0; AGENT_WORK_RECORD_BYTES];
    bytes[0] = 1;
    bytes[1] = 1;
    bytes[2] = 63;
    bytes[15] = 1;
    bytes[16..32].copy_from_slice(&owner.bytes());
    bytes[46..48].copy_from_slice(&key.to_be_bytes());
    bytes[62..64].copy_from_slice(&key.to_be_bytes());
    AgentWorkRecord::decode(bytes).unwrap()
}

pub(super) fn open() -> (tempfile::TempDir, Hub, AgentWorkIncarnation) {
    let directory = tempfile::tempdir().unwrap();
    let mut hub = open_hub(directory.path()).unwrap();
    let Reply::Claimed { owner, records } = hub.agent_work(Request::Claim).unwrap() else {
        panic!()
    };
    assert!(records.is_empty());
    (directory, hub, owner)
}

fn put(hub: &mut Hub, record: AgentWorkRecord) {
    compare_and_set_records(journal(hub), None, record, None, None).unwrap();
}

fn transition(
    hub: &mut Hub,
    record: AgentWorkRecord,
    to: AgentWorkDisposition,
) -> Result<AgentWorkRecord, Error> {
    let mutation = AgentWorkJournalMutation::transition(record, to)?;
    match hub.agent_work(Request::CompareAndSet(mutation))? {
        Reply::Record(Some(next)) => Ok(next),
        _ => panic!("wrong reply"),
    }
}

#[test]
fn namespace_fences_live_and_replaced_store_owners_until_process_exit() {
    let _process = work_test_guard();
    let (directory, first, owner) = open();
    let mut second = open_hub(directory.path()).unwrap();
    assert!(matches!(
        second.agent_work(Request::Claim),
        Err(Error::Fenced)
    ));
    drop(first);
    assert!(matches!(
        second.agent_work(Request::Claim),
        Err(Error::Fenced)
    ));
    simulate_process_exit();
    let Reply::Claimed {
        owner: second_owner,
        ..
    } = second.agent_work(Request::Claim).unwrap()
    else {
        panic!()
    };
    assert_ne!(owner, second_owner);
    assert!(matches!(
        second.agent_work(Request::Read {
            owner,
            key: [0; 32]
        }),
        Err(Error::Fenced)
    ));
}

#[test]
fn crash_restart_classifies_every_incomplete_stage_without_replay() {
    let _process = work_test_guard();
    let (directory, mut hub, owner) = open();
    let mut old = Vec::new();
    for (key, disposition) in [
        (1, AgentWorkDisposition::Admitted),
        (2, AgentWorkDisposition::Running),
        (3, AgentWorkDisposition::NeedsApproval),
        (4, AgentWorkDisposition::RecoveryRequired),
    ] {
        let record = initial(owner, key);
        put(&mut hub, record);
        let record = if disposition == AgentWorkDisposition::Admitted {
            record
        } else {
            let running = transition(&mut hub, record, AgentWorkDisposition::Running).unwrap();
            if disposition == AgentWorkDisposition::Running {
                running
            } else {
                transition(&mut hub, running, disposition).unwrap()
            }
        };
        old.push(record);
    }
    drop(hub);
    simulate_process_exit();
    let mut reopened = open_hub(directory.path()).unwrap();
    let Reply::Claimed {
        owner: new_owner,
        records,
    } = reopened.agent_work(Request::Claim).unwrap()
    else {
        panic!()
    };
    assert_eq!(records.len(), 4);
    for (current, old) in records.iter().zip(old) {
        assert_eq!(current.disposition(), AgentWorkDisposition::Interrupted);
        assert_eq!(current.incarnation(), new_owner);
        assert_eq!(current.debt(), AgentWorkDebt::UNKNOWN);
        assert_eq!(current.revision(), old.revision() + 1);
        assert!(matches!(
            reopened.agent_work(Request::Read {
                owner,
                key: old.key()
            }),
            Err(Error::Fenced)
        ));
        assert!(matches!(
            transition(&mut reopened, old, AgentWorkDisposition::FailedClosed),
            Err(Error::Fenced)
        ));
    }
}

#[test]
fn partial_restart_transaction_rolls_back_every_record_and_fence() {
    let _process = work_test_guard();
    let (directory, mut hub, owner) = open();
    let first = initial(owner, 1);
    let second = initial(owner, 2);
    put(&mut hub, first);
    put(&mut hub, second);
    drop(hub);
    simulate_process_exit();
    let mut reopened = open_hub(directory.path()).unwrap();
    FAULT.with(|fault| fault.set(Some(Fault::RestartPartial)));
    assert!(matches!(
        reopened.agent_work(Request::Claim),
        Err(Error::Uncertain)
    ));
    assert_eq!(
        inventory(journal(&mut reopened)).unwrap(),
        vec![first, second]
    );
    assert_eq!(verify_owner(journal(&mut reopened), owner), Ok(()));
    assert!(
        matches!(reopened.agent_work(Request::Claim), Ok(Reply::Claimed { owner: new_owner, records }) if new_owner != owner && records.iter().all(|record| record.incarnation() == new_owner))
    );
}

#[test]
fn partial_writes_and_lost_acknowledgement_reconcile_only_exact_cas() {
    let _process = work_test_guard();
    for injected in [Fault::BeforeWrite, Fault::AfterWrite, Fault::AfterCommit] {
        let (_directory, mut hub, owner) = open();
        let admitted = initial(owner, 1);
        put(&mut hub, admitted);
        let mutation =
            AgentWorkJournalMutation::transition(admitted, AgentWorkDisposition::Running).unwrap();
        FAULT.with(|fault| fault.set(Some(injected)));
        assert!(matches!(
            hub.agent_work(Request::CompareAndSet(mutation)),
            Err(Error::Uncertain)
        ));
        let expected = if injected == Fault::AfterCommit {
            mutation.next()
        } else {
            admitted
        };
        assert_eq!(
            read(journal(&mut hub), admitted.key()).unwrap(),
            Some(expected)
        );
        assert!(
            matches!(hub.agent_work(Request::CompareAndSet(mutation)), Ok(Reply::Record(Some(record))) if record == mutation.next())
        );
        assert_eq!(inventory(journal(&mut hub)).unwrap(), vec![mutation.next()]);
        drop(hub);
        simulate_process_exit();
    }
}

#[test]
fn settled_unsuccessful_terminals_survive_crash_without_reopening_or_result_bodies() {
    let _process = work_test_guard();
    for disposition in [
        AgentWorkDisposition::Failed,
        AgentWorkDisposition::Cancelled,
        AgentWorkDisposition::WaitingForHuman,
    ] {
        for injected in [Fault::BeforeWrite, Fault::AfterWrite, Fault::AfterCommit] {
            let (directory, mut hub, owner) = open();
            let admitted = initial(owner, 1);
            put(&mut hub, admitted);
            let running = transition(&mut hub, admitted, AgentWorkDisposition::Running).unwrap();
            // Exercise only the private storage transaction with historical
            // bytes. The public port still requires a proof-bearing mutation.
            let mut bytes = *running.as_bytes();
            bytes[1] = disposition as u8;
            bytes[2] = AgentWorkDebt::NONE.bits();
            if disposition == AgentWorkDisposition::WaitingForHuman {
                bytes[3] = 1;
                bytes[7] = 1;
            }
            bytes[8..16].copy_from_slice(&(running.revision() + 1).to_be_bytes());
            let terminal = AgentWorkRecord::decode(bytes).unwrap();
            FAULT.with(|fault| fault.set(Some(injected)));
            assert_eq!(
                compare_and_set_records(journal(&mut hub), Some(running), terminal, None, None),
                Err(Error::Uncertain)
            );
            assert_eq!(
                read(journal(&mut hub), running.key()).unwrap(),
                Some(if injected == Fault::AfterCommit {
                    terminal
                } else {
                    running
                })
            );
            compare_and_set_records(journal(&mut hub), Some(running), terminal, None, None)
                .unwrap();
            compare_and_set_records(journal(&mut hub), Some(running), terminal, None, None)
                .unwrap();
            assert_eq!(inventory(journal(&mut hub)).unwrap(), [terminal]);
            assert!(
                AgentWorkJournalMutation::transition(terminal, AgentWorkDisposition::Running)
                    .is_err()
            );
            let bodies: u64 = journal(&mut hub)
                .query_row("SELECT count(*) FROM agent_work_artifacts", [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(bodies, 0);
            drop(hub);
            simulate_process_exit();
            let mut reopened = open_hub(directory.path()).unwrap();
            let Reply::Claimed {
                owner: current,
                records,
            } = reopened.agent_work(Request::Claim).unwrap()
            else {
                panic!()
            };
            assert_ne!(current, owner);
            assert_eq!(records, [terminal]);
            assert_eq!(terminal.debt(), AgentWorkDebt::NONE);
            drop(reopened);
            simulate_process_exit();
        }
    }
}

#[test]
fn approval_accept_reject_stale_replay_and_cancellation_races_are_exact() {
    let _process = work_test_guard();
    for winner in [
        AgentWorkDisposition::FreshAdmissionRequired,
        AgentWorkDisposition::Rejected,
        AgentWorkDisposition::FailedClosed,
    ] {
        let (_directory, mut hub, owner) = open();
        let admitted = initial(owner, 1);
        put(&mut hub, admitted);
        let running = transition(&mut hub, admitted, AgentWorkDisposition::Running).unwrap();
        let review = transition(&mut hub, running, AgentWorkDisposition::NeedsApproval).unwrap();
        let terminal = transition(&mut hub, review, winner).unwrap();
        assert_eq!(terminal.debt(), AgentWorkDebt::UNKNOWN);
        // Transport-level retry acknowledges exact already persisted decision.
        assert_eq!(transition(&mut hub, review, winner), Ok(terminal));
        // A fresh user decision after acknowledgement cannot reopen the record.
        assert_eq!(
            transition(&mut hub, terminal, winner),
            Err(Error::Transition)
        );
        for loser in [
            AgentWorkDisposition::FreshAdmissionRequired,
            AgentWorkDisposition::Rejected,
            AgentWorkDisposition::FailedClosed,
        ] {
            if loser != winner {
                assert_eq!(transition(&mut hub, review, loser), Err(Error::Conflict));
            }
        }
        drop(hub);
        simulate_process_exit();
    }
}

#[test]
fn proof_closed_review_partial_writes_restart_and_decisions_preserve_exact_debt() {
    let _process = work_test_guard();
    for decision in [
        None,
        Some(AgentWorkDisposition::FreshAdmissionRequired),
        Some(AgentWorkDisposition::Rejected),
        Some(AgentWorkDisposition::FailedClosed),
    ] {
        for injected in [Fault::BeforeWrite, Fault::AfterWrite, Fault::AfterCommit] {
            let (directory, mut hub, owner) = open();
            let admitted = initial(owner, 1);
            put(&mut hub, admitted);
            let running = transition(&mut hub, admitted, AgentWorkDisposition::Running).unwrap();
            // Historical fixed-width facts exercise only the private storage
            // transaction. Production mutation construction still needs the
            // original failed policy/native and exact human refusal proofs.
            let mut bytes = *running.as_bytes();
            bytes[1] = AgentWorkDisposition::NeedsApproval as u8;
            bytes[2] = AgentWorkDebt::NONE.bits();
            bytes[8..16].copy_from_slice(&(running.revision() + 1).to_be_bytes());
            let review = AgentWorkRecord::decode(bytes).unwrap();
            FAULT.with(|fault| fault.set(Some(injected)));
            assert_eq!(
                compare_and_set_records(journal(&mut hub), Some(running), review, None, None),
                Err(Error::Uncertain)
            );
            assert_eq!(
                read(journal(&mut hub), running.key()).unwrap(),
                Some(if injected == Fault::AfterCommit {
                    review
                } else {
                    running
                })
            );
            compare_and_set_records(journal(&mut hub), Some(running), review, None, None).unwrap();
            compare_and_set_records(journal(&mut hub), Some(running), review, None, None).unwrap();
            let final_record = if let Some(winner) = decision {
                let mutation = AgentWorkJournalMutation::transition(review, winner).unwrap();
                FAULT.with(|fault| fault.set(Some(injected)));
                assert!(matches!(
                    hub.agent_work(Request::CompareAndSet(mutation)),
                    Err(Error::Uncertain)
                ));
                let terminal = transition(&mut hub, review, winner).unwrap();
                assert_eq!(transition(&mut hub, review, winner), Ok(terminal));
                assert_eq!(terminal.debt(), AgentWorkDebt::NONE);
                for loser in [
                    AgentWorkDisposition::FreshAdmissionRequired,
                    AgentWorkDisposition::Rejected,
                    AgentWorkDisposition::FailedClosed,
                ] {
                    if loser != winner {
                        assert_eq!(transition(&mut hub, review, loser), Err(Error::Conflict));
                    }
                }
                assert_eq!(
                    transition(&mut hub, terminal, winner),
                    Err(Error::Transition)
                );
                terminal
            } else {
                review
            };
            assert_eq!(inventory(journal(&mut hub)).unwrap(), [final_record]);
            drop(hub);
            simulate_process_exit();
            let mut reopened = open_hub(directory.path()).unwrap();
            let Reply::Claimed {
                owner: current,
                records,
            } = reopened.agent_work(Request::Claim).unwrap()
            else {
                panic!()
            };
            assert_ne!(current, owner);
            if decision.is_some() {
                assert_eq!(
                    records,
                    [final_record],
                    "review terminals are immutable even after restart"
                );
            } else {
                assert_eq!(records.len(), 1);
                assert_eq!(records[0].disposition(), AgentWorkDisposition::Interrupted);
                assert_eq!(
                    records[0].debt(),
                    AgentWorkDebt::UNKNOWN,
                    "restart cannot restore process-local closure authority"
                );
            }
            assert!(matches!(
                transition(&mut reopened, review, AgentWorkDisposition::Rejected),
                Err(Error::Fenced)
            ));
            drop(reopened);
            simulate_process_exit();
        }
    }
}

#[test]
fn terminal_persistence_is_immutable_across_restart_and_sql_update() {
    let _process = work_test_guard();
    let (directory, mut hub, owner) = open();
    let admitted = initial(owner, 1);
    put(&mut hub, admitted);
    let terminal = transition(&mut hub, admitted, AgentWorkDisposition::FailedClosed).unwrap();
    assert!(journal(&mut hub)
        .execute("UPDATE agent_work_runs SET terminal = 0", [])
        .is_err());
    assert!(journal(&mut hub)
        .execute("DELETE FROM agent_work_runs", [])
        .is_err());
    drop(hub);
    simulate_process_exit();
    let mut reopened = open_hub(directory.path()).unwrap();
    assert!(
        matches!(reopened.agent_work(Request::Claim), Ok(Reply::Claimed { records, .. }) if records == vec![terminal])
    );
}

#[test]
fn transient_store_cannot_claim_durable_admission() {
    let _process = work_test_guard();
    let mut hub = Hub::in_memory().unwrap();
    assert!(matches!(
        hub.agent_work(Request::Claim),
        Err(Error::Unavailable)
    ));
}

#[test]
fn corrupt_record_fails_closed() {
    let _process = work_test_guard();
    let (_directory, mut hub, owner) = open();
    let admitted = initial(owner, 1);
    put(&mut hub, admitted);
    let mut corrupt = *admitted.as_bytes();
    corrupt[3] = 1;
    journal(&mut hub)
        .execute(
            "UPDATE agent_work_runs SET record = ?1",
            [corrupt.as_slice()],
        )
        .unwrap();
    assert_eq!(
        read(journal(&mut hub), admitted.key()),
        Err(Error::Uncertain)
    );
    assert!(matches!(
        hub.agent_work(Request::Claim),
        Err(Error::Uncertain)
    ));
}

#[test]
fn retention_capacity_refuses_admission_without_evicting_debt() {
    let _process = work_test_guard();
    let (_directory, mut hub, owner) = open();
    let transaction = journal(&mut hub).transaction().unwrap();
    for key in 1..=1024 {
        let record = initial(owner, key);
        transaction
            .execute(
                "INSERT INTO agent_work_runs(run_key, record, terminal) VALUES (?1, ?2, 0)",
                params![record.key().as_slice(), record.as_bytes().as_slice()],
            )
            .unwrap();
    }
    transaction.commit().unwrap();
    assert_eq!(
        compare_and_set_records(journal(&mut hub), None, initial(owner, 1025), None, None),
        Err(Error::Capacity)
    );
    assert_eq!(inventory(journal(&mut hub)).unwrap().len(), 1024);
    let first = initial(owner, 1);
    assert!(transition(&mut hub, first, AgentWorkDisposition::FailedClosed).is_ok());
}

#[test]
fn process_fence_outlives_store_and_is_released_by_real_child_process_exit() {
    let _process = work_test_guard();
    let (directory, hub, _) = open();
    let child = |expected: &str| {
        let result = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "hub::agent_work::tests::process_fence_child",
                "--ignored",
            ])
            .env("ZEPHIUM_TEST_WORK_DIRECTORY", directory.path())
            .env("ZEPHIUM_TEST_WORK_EXPECTED", expected)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "bounded process-fence fixture failed"
        );
    };
    child("unavailable");
    drop(hub);
    child("unavailable");
    simulate_process_exit();
    child("claimed");
    // The child retained its production static fence until actual OS exit.
    child("claimed");
}

#[test]
#[ignore = "executed only by the process-fence parent fixture"]
fn process_fence_child() {
    let directory =
        std::path::PathBuf::from(std::env::var_os("ZEPHIUM_TEST_WORK_DIRECTORY").unwrap());
    let mut hub = open_hub(&directory).unwrap();
    let result = hub.agent_work(Request::Claim);
    match std::env::var("ZEPHIUM_TEST_WORK_EXPECTED")
        .unwrap()
        .as_str()
    {
        "unavailable" => assert!(matches!(result, Err(Error::Unavailable))),
        "claimed" => assert!(matches!(result, Ok(Reply::Claimed { .. }))),
        _ => panic!("invalid process fixture expectation"),
    }
}

/// Raw persisted history: one terminal row with an exact settlement order.
pub(super) fn insert_settled(
    connection: &Connection,
    owner: AgentWorkIncarnation,
    key: u16,
    disposition: AgentWorkDisposition,
    debt: AgentWorkDebt,
    settled: Option<i64>,
) -> AgentWorkRecord {
    let mut bytes = *initial(owner, key).as_bytes();
    bytes[1] = disposition as u8;
    bytes[2] = debt.bits();
    bytes[15] = 3;
    let record = AgentWorkRecord::decode(bytes).unwrap();
    connection
        .execute(
            "INSERT INTO agent_work_runs(run_key, record, terminal, settled) VALUES (?1, ?2, 1, ?3)",
            params![record.key().as_slice(), record.as_bytes().as_slice(), settled],
        )
        .unwrap();
    record
}

pub(super) fn clean_runs(hub: &mut Hub) -> Vec<[u8; 32]> {
    settled_clean(journal(hub))
        .unwrap()
        .into_iter()
        .map(|run| run.key)
        .collect()
}

fn debt_rows(hub: &mut Hub) -> usize {
    outstanding(journal(hub)).unwrap()
}

#[test]
fn clean_history_never_blocks_admission_and_reopening_prunes_it() {
    let _process = work_test_guard();
    let (directory, mut hub, owner) = open();
    let transaction = journal(&mut hub).transaction().unwrap();
    let mut debt = Vec::new();
    for key in 1..=1100 {
        insert_settled(
            &transaction,
            owner,
            key,
            AgentWorkDisposition::Failed,
            AgentWorkDebt::NONE,
            Some(i64::from(key)),
        );
    }
    for key in 2001..=2003 {
        debt.push(insert_settled(
            &transaction,
            owner,
            key,
            AgentWorkDisposition::FailedClosed,
            AgentWorkDebt::UNKNOWN,
            Some(i64::from(key)),
        ));
    }
    transaction.commit().unwrap();

    // 1,103 stored runs used to refuse every admission for good.
    let admitted = initial(owner, 3001);
    put(&mut hub, admitted);
    let Reply::Claimed { records, .. } = hub.agent_work(Request::Claim).unwrap() else {
        panic!()
    };
    assert_eq!(records.len(), 3 + 1 + RETAINED_CLEAN_AGENT_WORK_RUNS);
    assert!(debt.iter().all(|record| records.contains(record)));

    // A run's end prunes one bounded batch, oldest first.
    hub.prune_agent_work(0, AGENT_WORK_PRUNE_BATCH);
    assert_eq!(clean_runs(&mut hub).len(), 1100 - AGENT_WORK_PRUNE_BATCH);
    assert_eq!(
        read(journal(&mut hub), initial(owner, 1).key()).unwrap(),
        None
    );
    assert!(read(journal(&mut hub), initial(owner, 1100).key())
        .unwrap()
        .is_some());

    drop(hub);
    simulate_process_exit();
    let mut reopened = open_hub(directory.path()).unwrap();
    let kept = clean_runs(&mut reopened);
    assert_eq!(kept.len(), RETAINED_CLEAN_AGENT_WORK_RUNS);
    assert_eq!(kept[0], initial(owner, 1100).key());
    assert_eq!(kept[255], initial(owner, 845).key());
    assert_eq!(debt_rows(&mut reopened), 4);
    for record in &debt {
        assert_eq!(
            read(journal(&mut reopened), record.key()).unwrap(),
            Some(*record)
        );
    }
}

#[test]
fn recovery_history_is_never_pruned_or_deletable() {
    let _process = work_test_guard();
    let (_directory, mut hub, owner) = open();
    let transaction = journal(&mut hub).transaction().unwrap();
    let mut debt = Vec::new();
    for key in 1..=300 {
        debt.push(insert_settled(
            &transaction,
            owner,
            key,
            AgentWorkDisposition::Rejected,
            AgentWorkDebt::UNKNOWN,
            None,
        ));
    }
    let clean = insert_settled(
        &transaction,
        owner,
        301,
        AgentWorkDisposition::Cancelled,
        AgentWorkDebt::NONE,
        None,
    );
    transaction.commit().unwrap();
    let live = initial(owner, 302);
    put(&mut hub, live);

    hub.prune_agent_work(0, AGENT_WORK_PRUNE_AT_OPEN);
    assert_eq!(debt_rows(&mut hub), 301);
    assert_eq!(clean_runs(&mut hub), [clean.key()]);
    for row in [debt[0], live] {
        assert!(journal(&mut hub)
            .execute(
                "DELETE FROM agent_work_runs WHERE run_key = ?1",
                [row.key().as_slice()]
            )
            .is_err());
    }
    assert_eq!(
        journal(&mut hub)
            .execute(
                "DELETE FROM agent_work_runs WHERE run_key = ?1",
                [clean.key().as_slice()]
            )
            .unwrap(),
        1
    );
    assert_eq!(debt_rows(&mut hub), 301);
}

#[test]
fn settlement_orders_terminals_and_admission_counts_only_outstanding_runs() {
    let _process = work_test_guard();
    let (_directory, mut hub, owner) = open();
    let mut settled = Vec::new();
    for key in [9, 3, 7] {
        let admitted = initial(owner, key);
        put(&mut hub, admitted);
        let closed = transition(&mut hub, admitted, AgentWorkDisposition::FailedClosed).unwrap();
        settled.push(
            journal(&mut hub)
                .query_row(
                    "SELECT settled FROM agent_work_runs WHERE run_key = ?1",
                    [closed.key().as_slice()],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
        );
    }
    assert_eq!(settled, [1, 2, 3]);
    // A terminal write that skips the settlement order is refused.
    let admitted = initial(owner, 10);
    put(&mut hub, admitted);
    assert!(journal(&mut hub)
        .execute(
            "UPDATE agent_work_runs SET terminal = 1 WHERE settled IS NULL",
            []
        )
        .is_err());
    assert_eq!(debt_rows(&mut hub), 4);
}
