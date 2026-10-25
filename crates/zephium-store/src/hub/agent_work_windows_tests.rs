//! Native protected-backend fixtures carry storage facts, not runtime authority.

use super::tests::{initial, simulate_process_exit, work_test_guard, Fault, FAULT};
use super::*;
use sha2::{Digest, Sha256};
use zephium_core::work::{artifact::WorkEvidenceLink, WorkError};

fn fixture() -> (tempfile::TempDir, tempfile::TempDir, Hub) {
    let profile = tempfile::tempdir().unwrap();
    let ordinary = tempfile::tempdir().unwrap();
    let selector = super::super::WindowsWorkStorage::for_validation_fixture(
        profile.path(),
        "app.quasar.work-integration",
        None,
    )
    .unwrap();
    let hub = Hub::open_with_windows_work_storage(ordinary.path().into(), selector).unwrap();
    (profile, ordinary, hub)
}

fn archived(
    owner: AgentWorkIncarnation,
) -> (
    AgentWorkRecord,
    AgentWorkRecord,
    AgentWorkArtifactDescriptor,
    Vec<u8>,
) {
    let running = initial(owner, 1)
        .transition(AgentWorkDisposition::Running)
        .unwrap();
    let mut bytes = *running.as_bytes();
    bytes[1] = 6;
    bytes[2] = 0;
    bytes[15] = 3;
    let succeeded = AgentWorkRecord::decode(bytes).unwrap();
    let body = br#"{"version":1,"id":[1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1],"profile":"00000000000000000000000001","key":[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,1,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,1],"schema":1,"observation":1,"generation":1,"captured_millis":1,"fields":[],"sources":[{"id":1,"origin":"https://fixture.invalid/","role":"paragraph","field":1,"context":[1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1],"context_generation":1,"navigation_epoch":1,"frame":1,"frame_generation":1,"invocation":1,"snapshot":1,"reference":1,"browser_derived":true,"content":{"kind":"text","value":"Protected historical fixture"}}]}"#.to_vec();
    let descriptor = AgentWorkArtifactDescriptor::decode(
        [1_u8; 16],
        1_u128.into(),
        running.key(),
        Sha256::digest(&body).into(),
        body.len() as u32,
    )
    .unwrap();
    AgentWorkArchivedExtraction::decode(descriptor, &body).unwrap();
    (running, succeeded, descriptor, body)
}

fn seed(
    hub: &mut Hub,
) -> (
    AgentWorkRecord,
    AgentWorkRecord,
    AgentWorkArtifactDescriptor,
    Vec<u8>,
) {
    let owner = AgentWorkIncarnation::generate();
    let value = archived(owner);
    let admitted = initial(owner, 1);
    hub.with_work_connection(|connection| {
        compare_and_set_records(connection, None, admitted, Some(1_u128.into()), None)?;
        compare_and_set_records(connection, Some(admitted), value.0, None, None)
    })
    .unwrap();
    value
}

fn publish(
    hub: &mut Hub,
    value: &(
        AgentWorkRecord,
        AgentWorkRecord,
        AgentWorkArtifactDescriptor,
        Vec<u8>,
    ),
) -> Result<(), Error> {
    hub.with_work_connection(|connection| {
        compare_and_set_records(
            connection,
            Some(value.0),
            value.1,
            None,
            Some(ArtifactStorageValue {
                descriptor: value.2,
                body: &value.3,
            }),
        )
    })
}

#[test]
fn protected_cas_artifact_transaction_rolls_back_together_and_reopens_exactly() {
    let (_profile, _ordinary, mut hub) = fixture();
    let value = seed(&mut hub);
    FAULT.with(|fault| fault.set(Some(Fault::AfterArtifactWrite)));
    assert!(matches!(publish(&mut hub, &value), Err(Error::Uncertain)));
    hub.with_work_connection(|connection| {
        assert_eq!(read(connection, value.0.key())?, Some(value.0));
        assert!(read_artifact(connection, value.0.key(), 1_u128.into())?.is_none());
        Ok(())
    })
    .unwrap();
    publish(&mut hub, &value).unwrap();
    publish(&mut hub, &value).unwrap();
    let selector = hub.windows_work_storage.clone().unwrap();
    let dir = hub.dir.clone().unwrap();
    drop(hub);
    let mut reopened = Hub::open_with_windows_work_storage(dir, selector).unwrap();
    reopened
        .with_work_connection(|connection| {
            assert_eq!(read(connection, value.1.key())?, Some(value.1));
            assert_eq!(
                read_artifact(connection, value.1.key(), 1_u128.into())?
                    .unwrap()
                    .descriptor(),
                value.2
            );
            Ok(())
        })
        .unwrap();
    assert_eq!(
        reopened
            .meta
            .query_row("SELECT count(*) FROM agent_work_runs", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn protected_history_never_claims_and_intent_fences_reads_and_publication() {
    let (_profile, _ordinary, mut hub) = fixture();
    let profile: ProfileId = 1_u128.into();
    let value = seed(&mut hub);
    publish(&mut hub, &value).unwrap();
    let link = WorkEvidenceLink {
        extraction_id: u128::from_be_bytes(value.2.id()).into(),
        source_id: 1,
    };
    assert_eq!(
        hub.read_agent_work_evidence(profile, link.clone())
            .unwrap()
            .text,
        "Protected historical fixture"
    );
    assert!(hub.work.is_none());
    assert!(hub.purge_windows_work_artifacts(profile).is_err());
    hub.registry.insert(profile);
    hub.authorize_windows_work_artifact_deletion(profile)
        .unwrap();
    hub.authorize_windows_work_artifact_deletion(profile)
        .unwrap();
    // An ambiguous meta commit must not erase active-profile bodies.
    assert!(hub.purge_windows_work_artifacts(profile).is_err());
    assert!(matches!(
        hub.read_agent_work_evidence(profile, link.clone()),
        Err(WorkError::NotFound)
    ));
    hub.registry.remove(&profile);
    hub.purge_windows_work_artifacts(profile).unwrap();
    hub.purge_windows_work_artifacts(profile).unwrap();
    let path = hub
        .windows_work_database
        .as_ref()
        .unwrap()
        .anchor
        .database_path();
    for file in [
        path.clone(),
        path.with_file_name("work.sqlite-wal"),
        path.with_file_name("work.sqlite-shm"),
    ] {
        match std::fs::read(file) {
            Ok(bytes) => assert!(!bytes
                .windows(b"Protected historical fixture".len())
                .any(|window| window == b"Protected historical fixture")),
            Err(error) => assert_eq!(error.kind(), std::io::ErrorKind::NotFound),
        }
    }
    hub.registry.insert(profile);
    assert!(matches!(
        hub.read_agent_work_evidence(profile, link),
        Err(WorkError::NotFound)
    ));
    hub.with_work_connection(|connection| {
        assert_eq!(read(connection, value.1.key())?, Some(value.1));
        assert!(read_artifact(connection, value.1.key(), profile)?.is_none());
        let forged = connection.execute("INSERT INTO agent_work_artifacts(run_key, profile_id, artifact_id, digest, body) VALUES (?1, ?2, ?3, ?4, ?5)", params![value.1.key().as_slice(), profile.to_string(), value.2.id().as_slice(), value.2.digest().as_slice(), value.3]);
        assert!(forged.is_err());
        Ok(())
    }).unwrap();
}

#[test]
fn protected_claim_ignores_appdata_rows_and_retains_lease_until_process_exit() {
    let _guard = work_test_guard();
    let (_profile, _ordinary, mut hub) = fixture();
    let old = initial(AgentWorkIncarnation::generate(), 2);
    compare_and_set_records(&mut hub.meta, None, old, None, None).unwrap();
    let selector = hub.windows_work_storage.clone().unwrap();
    let dir = hub.dir.clone().unwrap();
    let Reply::Claimed { records, .. } = hub.agent_work(Request::Claim).unwrap() else {
        panic!()
    };
    assert!(records.is_empty());
    drop(hub);
    let mut replacement =
        Hub::open_with_windows_work_storage(dir.clone(), selector.clone()).unwrap();
    assert!(matches!(
        replacement.agent_work(Request::Claim),
        Err(Error::Fenced)
    ));
    assert!(replacement
        .read_agent_work_evidence(
            1_u128.into(),
            WorkEvidenceLink {
                extraction_id: 1_u128.into(),
                source_id: 1
            }
        )
        .is_err());
    drop(replacement);
    simulate_process_exit();
    let mut restarted = Hub::open_with_windows_work_storage(dir, selector).unwrap();
    assert!(matches!(
        restarted.agent_work(Request::Claim),
        Ok(Reply::Claimed { .. })
    ));
    drop(restarted);
    drop(_guard);
}
