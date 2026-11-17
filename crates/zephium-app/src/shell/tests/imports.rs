use super::*;
use crate::{ImportCompletion, ImportWork};
use std::sync::mpsc::{sync_channel, Receiver};
use zephium_core::bookmarks::{BookmarkReply, BookmarkRequest, ImportNode};
use zephium_core::ports::store::ImportedVisit;

fn destination(shell: &mut Shell) -> Option<ProfileId> {
    let (reply, receive) = sync_channel(1);
    shell.handle(Command::ImportDestination { reply });
    receive.try_recv().expect("admission always answers")
}

fn write(shell: &mut Shell, destination: ProfileId, work: ImportWork) -> Receiver<Option<u32>> {
    let (send, receive) = std::sync::mpsc::channel();
    shell.handle(Command::Import {
        destination,
        work: Box::new(work),
        done: ImportCompletion::new(move |added| {
            let _ = send.send(added);
        }),
    });
    receive
}

fn bookmarks(folder: &str) -> ImportWork {
    ImportWork::Bookmarks {
        folder: folder.into(),
        nodes: vec![ImportNode::Link {
            title: "Imported bookmark".into(),
            url: "https://imported.example/bookmark".into(),
        }],
    }
}

fn history() -> ImportWork {
    ImportWork::History(vec![ImportedVisit {
        url: "https://imported.example/history".into(),
        title: "Imported visit".into(),
        visited_at: 1_800_000_000,
    }])
}

fn essentials() -> ImportWork {
    ImportWork::Essentials(vec![crate::ImportedSite {
        url: "https://imported.example/essential".into(),
        title: "Imported essential".into(),
    }])
}

fn icons() -> ImportWork {
    ImportWork::Icons(vec![(
        "https://imported.example".into(),
        vec![6; zephium_core::icon::RGBA32_BYTES],
    )])
}

fn reader(shell: &mut Shell) -> crate::store_reads::StoreReadQueue {
    let queue = crate::store_reads::StoreReadQueue::new();
    shell.store_reads = Some(queue.clone());
    queue
}

/// Run the production worker off the shell, then deliver all its replies.
fn finish_reads(
    shell: &mut Shell,
    store: SharedStore,
    queue: crate::store_reads::StoreReadQueue,
    count: usize,
) {
    let (send, receive) = std::sync::mpsc::channel();
    let worker_queue = queue.clone();
    let worker = std::thread::spawn(move || {
        crate::store_reads::run_for_test(store, worker_queue, send);
    });
    let replies: Vec<_> = (0..count)
        .map(|_| receive.recv_timeout(std::time::Duration::from_secs(5)))
        .collect();
    queue.stop();
    worker.join().unwrap();
    for reply in replies {
        shell.handle(Command::StoreRead(reply.expect("every import must finish")));
    }
}

#[test]
fn import_admission_restores_onboarding_and_refuses_private_or_degraded_destinations() {
    let (mut shell, _engine, _screen) = setup();
    assert!(!shell.bootstrapped);
    let regular = destination(&mut shell).expect("onboarding gets its native destination");
    assert!(shell.bootstrapped);
    assert_eq!(regular, shell.windows.focused().unwrap().profile);

    shell.degraded_storage_profiles.insert(regular);
    assert_eq!(destination(&mut shell), None);
    for work in [essentials(), bookmarks("Imported"), history(), icons()] {
        assert_eq!(write(&mut shell, regular, work).try_recv().unwrap(), None);
    }
    shell.degraded_storage_profiles.remove(&regular);

    shell.handle(Command::Run("window.newPrivate".into()));
    let private = shell.windows.focused().unwrap().profile;
    assert_ne!(private, regular);
    assert_eq!(destination(&mut shell), None, "no regular-profile fallback");
    for work in [essentials(), bookmarks("Imported"), history(), icons()] {
        assert_eq!(write(&mut shell, private, work).try_recv().unwrap(), None);
    }
    assert!(shell
        .items
        .roots(Placement::Favorites { profile: regular })
        .is_empty());
}

#[test]
fn all_import_kinds_keep_the_admitted_destination_across_profile_switches() {
    let store = Arc::new(zephium_store::SqliteStore::in_memory().unwrap());
    let mut shell = Shell::new(
        Arc::new(FakeEngine::default()),
        store.clone(),
        Arc::new(FakeChrome),
        Box::new(|_| {}),
    );
    shell.handle(Command::SetWindowSize(Size::new(1200.0, 800.0)));
    let admitted = destination(&mut shell).unwrap();
    let created = shell.handle_operation(Command::CreateProfile {
        name: "Other destination".into(),
        avatar: zephium_core::profiles::ProfileAvatar::initial(ProfileId::from(2)),
        photo: None,
        switch: true,
    });
    assert!(matches!(
        created.outcome,
        OperationOutcome::Applied | OperationOutcome::Deferred
    ));
    let other = shell.windows.focused().unwrap().profile;
    assert_ne!(admitted, other);

    // The source read finishes only after the person switched away. All
    // batches, including the later icon read, retain the original admission.
    let queue = reader(&mut shell);
    assert_eq!(
        write(&mut shell, admitted, essentials())
            .try_recv()
            .unwrap(),
        Some(1)
    );
    let saved_bookmarks = write(
        &mut shell,
        admitted,
        bookmarks("Imported from Chrome (Default)"),
    );
    let saved_history = write(&mut shell, admitted, history());
    assert!(matches!(
        saved_bookmarks.try_recv(),
        Err(std::sync::mpsc::TryRecvError::Empty)
    ));
    assert!(matches!(
        saved_history.try_recv(),
        Err(std::sync::mpsc::TryRecvError::Empty)
    ));
    finish_reads(&mut shell, store.clone(), queue, 2);
    assert_eq!(saved_bookmarks.try_recv().unwrap(), Some(1));
    assert_eq!(saved_history.try_recv().unwrap(), Some(1));

    // Icons learned from websites stay owned by their profiles and take
    // precedence over the source browser's cached copy.
    store.save_favicon(
        admitted,
        "https://owned.example".into(),
        Some(zephium_core::icon::RGBA32_MIME.into()),
        vec![3; zephium_core::icon::RGBA32_BYTES],
    );
    store.save_favicon(
        other,
        "https://imported.example".into(),
        Some(zephium_core::icon::RGBA32_MIME.into()),
        vec![9; zephium_core::icon::RGBA32_BYTES],
    );
    let queue = reader(&mut shell);
    let ImportWork::Icons(mut rasters) = icons() else {
        unreachable!()
    };
    rasters.push((
        "https://owned.example".into(),
        vec![7; zephium_core::icon::RGBA32_BYTES],
    ));
    let saved_icons = write(&mut shell, admitted, ImportWork::Icons(rasters));
    assert!(matches!(
        saved_icons.try_recv(),
        Err(std::sync::mpsc::TryRecvError::Empty)
    ));
    finish_reads(&mut shell, store.clone(), queue, 1);
    assert_eq!(saved_icons.try_recv().unwrap(), Some(1));
    assert_eq!(shell.windows.focused().unwrap().profile, other);

    assert_eq!(
        shell
            .items
            .roots(Placement::Favorites { profile: admitted })
            .len(),
        1
    );
    assert!(shell
        .items
        .roots(Placement::Favorites { profile: other })
        .is_empty());
    assert_eq!(store.history_page(admitted, "", None, None, 10).len(), 1);
    assert!(store.history_page(other, "", None, None, 10).is_empty());
    assert!(matches!(
        store.bookmarks(admitted, BookmarkRequest::Search { query: "Imported bookmark".into() }),
        BookmarkReply::Nodes(nodes) if nodes.len() == 1
    ));
    assert_eq!(
        store.bookmarks(other, BookmarkRequest::Children { parent: None }),
        BookmarkReply::Nodes(Vec::new())
    );
    assert_eq!(
        store
            .favicon_raster_with_age(admitted, "https://imported.example")
            .map(|(bytes, _)| bytes),
        Some(vec![6; zephium_core::icon::RGBA32_BYTES])
    );
    assert_eq!(
        store
            .favicon_raster_with_age(other, "https://imported.example")
            .map(|(bytes, _)| bytes),
        Some(vec![9; zephium_core::icon::RGBA32_BYTES])
    );
    assert_eq!(
        store
            .favicon_raster_with_age(admitted, "https://owned.example")
            .map(|(bytes, _)| bytes),
        Some(vec![3; zephium_core::icon::RGBA32_BYTES])
    );
    assert!(!shell
        .favicons
        .icon_values
        .contains_key(&(other, "https://imported.example".into())));

    // A retry stays deduplicated, and another external source profile keeps
    // its own folder even if it contains the same bookmark address.
    let queue = reader(&mut shell);
    let duplicate_bookmarks = write(
        &mut shell,
        admitted,
        bookmarks("Imported from Chrome (Default)"),
    );
    let duplicate_history = write(&mut shell, admitted, history());
    let duplicate_icons = write(&mut shell, admitted, icons());
    let other_source = write(
        &mut shell,
        admitted,
        bookmarks("Imported from Chrome (Work)"),
    );
    finish_reads(&mut shell, store.clone(), queue, 4);
    for duplicate in [duplicate_bookmarks, duplicate_history, duplicate_icons] {
        assert_eq!(duplicate.try_recv().unwrap(), Some(0));
    }
    assert_eq!(other_source.try_recv().unwrap(), Some(1));
    assert_eq!(
        write(&mut shell, admitted, essentials())
            .try_recv()
            .unwrap(),
        Some(0)
    );
    assert_eq!(
        store.shutdown_until(std::time::Instant::now() + std::time::Duration::from_secs(5)),
        StoreShutdownOutcome::Clean
    );
}

#[test]
fn invalid_quarantined_and_retired_destinations_never_fall_back_to_focus() {
    let (mut shell, _engine, _screen) = setup();
    let focused = destination(&mut shell).unwrap();
    let retiring = add_inactive_named_profile(&mut shell, 51_000);
    let admitted = shell.begin_profile_deletion(retiring, None);
    assert_eq!(admitted.outcome, OperationOutcome::Deferred);
    assert!(
        shell.profiles.get(retiring).is_some(),
        "quarantine precedes removal"
    );

    let _queue = reader(&mut shell);
    for refused in [ProfileId::from(99_999), retiring] {
        for work in [essentials(), bookmarks("Imported"), history(), icons()] {
            assert_eq!(write(&mut shell, refused, work).try_recv().unwrap(), None);
        }
    }
    shell.apply_profile_tombstone(retiring);
    assert!(shell.profiles.get(retiring).is_none());
    for work in [essentials(), bookmarks("Imported"), history(), icons()] {
        assert_eq!(write(&mut shell, retiring, work).try_recv().unwrap(), None);
    }
    assert_eq!(shell.windows.focused().unwrap().profile, focused);
    assert!(shell
        .items
        .roots(Placement::Favorites { profile: focused })
        .is_empty());
}
