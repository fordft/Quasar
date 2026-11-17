use super::*;

#[test]
fn favicon_pipeline_accepts_only_fixed_renderer_rasters_for_current_origin() {
    let engine = Arc::new(FakeEngine::default());
    let store = Arc::new(FakeStore::default());
    let screen: Screen = Arc::new(Mutex::new(ItemsState {
        projection_revision: String::new(),
        profile: None,
        spaces: Vec::new(),
        active_space_id: None,
        nodes: Vec::new(),
        tabs: Vec::new(),
        active: None,
        split_group: None,
    }));
    let sink = screen.clone();
    let mut shell = Shell::new(
        engine.clone(),
        store.clone(),
        Arc::new(FakeChrome),
        Box::new(move |p| apply_projection(&mut sink.lock().unwrap(), p)),
    );
    shell.handle(Command::SetWindowSize(Size::new(1200.0, 800.0)));
    shell.handle(Command::Bootstrap);
    let id = active_id(&screen);
    shell.handle(Command::Navigate {
        id,
        input: "example.com".into(),
    });

    shell.handle(Command::Engine(EngineEvent::UrlChanged {
        id,
        url: "https://example.com/".into(),
    }));
    present_committed(&mut shell, id, "https://example.com/");
    assert!(engine
        .calls()
        .iter()
        .any(|call| call == &format!("discover {id}")));

    shell.handle(Command::Engine(EngineEvent::FaviconPixels {
        id,
        page_url: "https://attacker.example/".into(),
        rgba: vec![1; zephium_core::icon::RGBA32_BYTES],
    }));
    assert!(store.icons.lock().unwrap().is_empty());

    shell.handle(Command::Engine(EngineEvent::FaviconPixels {
        id,
        page_url: "https://example.com/".into(),
        rgba: vec![1; zephium_core::icon::RGBA32_BYTES - 1],
    }));
    assert!(store.icons.lock().unwrap().is_empty());

    let rgba = vec![7; zephium_core::icon::RGBA32_BYTES];
    shell.handle(Command::Engine(EngineEvent::FaviconPixels {
        id,
        page_url: "https://example.com/".into(),
        rgba: rgba.clone(),
    }));
    assert_eq!(
        store.icons.lock().unwrap().as_slice(),
        &[(String::from("https://example.com"), rgba)]
    );
    let tab = last(&screen).tabs.into_iter().next().unwrap();
    assert_eq!(
        tab.icon.as_ref().map(|icon| icon.origin.as_str()),
        Some("https://example.com")
    );
    let profile = shell.windows.focused().unwrap().profile;
    assert!(shell
        .icon_ref_for_url(
            zephium_ipc::IconSurface::Chrome,
            profile,
            "https://example.com"
        )
        .is_some());
}

#[test]
fn stale_favicon_store_reply_cannot_cross_a_navigation_generation() {
    let (mut shell, _engine, screen) = setup();
    shell.handle(Command::Bootstrap);
    shell.store_reads = Some(StoreReadQueue::new());
    let id = active_id(&screen);
    let profile = shell.windows.focused().unwrap().profile;

    shell.handle(Command::Engine(EngineEvent::UrlChanged {
        id,
        url: "https://first.example/".into(),
    }));
    let first_generation = shell.favicons.store_reads.get(&id).unwrap().generation;
    shell.handle(Command::Engine(EngineEvent::UrlChanged {
        id,
        url: "https://second.example/".into(),
    }));
    let second_generation = shell.favicons.store_reads.get(&id).unwrap().generation;
    let rgba = vec![91; zephium_core::icon::RGBA32_BYTES];

    shell.handle(Command::StoreRead(StoreReadResult::Favicon {
        generation: first_generation,
        id,
        profile,
        origin: "https://first.example".into(),
        rgba: Some(rgba.clone()),
        stale: false,
    }));
    assert!(shell
        .icon_ref_for_url(
            zephium_ipc::IconSurface::Chrome,
            profile,
            "https://first.example/"
        )
        .is_none());
    assert_eq!(
        shell
            .favicons
            .store_reads
            .get(&id)
            .map(|pending| pending.generation),
        Some(second_generation)
    );

    shell.handle(Command::StoreRead(StoreReadResult::Favicon {
        generation: second_generation,
        id,
        profile,
        origin: "https://second.example".into(),
        rgba: Some(rgba),
        stale: false,
    }));
    assert!(shell
        .icon_ref_for_url(
            zephium_ipc::IconSurface::Chrome,
            profile,
            "https://second.example/"
        )
        .is_some());
}

#[test]
fn lost_favicon_store_completion_falls_back_instead_of_sticking_pending() {
    let (mut shell, engine, screen) = setup();
    shell.handle(Command::Bootstrap);
    shell.store_reads = Some(StoreReadQueue::new());
    shell.self_queue = Some(CommandQueue::new());
    let id = active_id(&screen);
    shell.handle(Command::Engine(EngineEvent::UrlChanged {
        id,
        url: "https://fallback.example/".into(),
    }));
    assert!(shell.favicons.store_reads.contains_key(&id));
    assert!(engine
        .calls()
        .iter()
        .all(|call| call != &format!("discover {id}")));

    shell.handle(Command::FaviconPoll { id, attempt: 0 });
    assert!(!shell.favicons.store_reads.contains_key(&id));
    assert!(engine
        .calls()
        .iter()
        .any(|call| call == &format!("discover {id}")));
}

#[test]
fn bootstrap_hydrates_restored_tab_icons_without_native_callbacks() {
    let store = Arc::new(FakeStore::default());
    let (mut first, _engine, screen) = setup_with(store.clone());
    first.handle(Command::Bootstrap);
    let first_id = active_id(&screen);
    navigate_and_commit(&mut first, first_id, "restored-one.example");
    first.handle(Command::Open);
    let second_id = active_id(&screen);
    navigate_and_commit(&mut first, second_id, "restored-two.example");

    store.icons.lock().unwrap().extend([
        (
            "https://restored-one.example".to_owned(),
            vec![31; zephium_core::icon::RGBA32_BYTES],
        ),
        (
            "https://restored-two.example".to_owned(),
            vec![47; zephium_core::icon::RGBA32_BYTES],
        ),
    ]);
    drop(first);

    let (mut restored, engine, restored_screen) = setup_with(store);
    restored.handle(Command::Bootstrap);

    let state = last(&restored_screen);
    assert_eq!(state.tabs.len(), 2);
    assert!(state.tabs.iter().all(|tab| tab.icon.is_some()));
    assert!(engine
        .calls()
        .iter()
        .all(|call| !call.starts_with("discover ")));
}

#[test]
fn positive_cache_eviction_allows_later_rehydration() {
    let (mut shell, _engine, _screen) = setup();
    let profile = ProfileId::from(91_000);
    let rgba = vec![83; zephium_core::icon::RGBA32_BYTES];
    let first = (profile, "https://icon-0.example".to_owned());

    for index in 0..=ICON_CACHE_CAPACITY {
        let key = (profile, format!("https://icon-{index}.example"));
        assert!(shell.cache_icon(key.clone(), &rgba));
        shell.favicons.icons_checked.insert(key);
    }

    assert!(!shell.favicons.icon_values.contains_key(&first));
    assert!(
        !shell.favicons.icons_checked.contains(&first),
        "an evicted positive must not become a permanent negative cache entry"
    );
}

#[test]
fn slow_load_gets_one_fresh_bounded_favicon_pass_after_completion() {
    let (mut shell, engine, screen) = setup();
    shell.handle(Command::Bootstrap);
    let id = active_id(&screen);
    shell.handle(Command::Navigate {
        id,
        input: "slow-icon.example".into(),
    });
    shell.handle(Command::Engine(EngineEvent::LoadingChanged {
        id,
        loading: true,
    }));
    shell.handle(Command::Engine(EngineEvent::UrlChanged {
        id,
        url: "https://slow-icon.example/".into(),
    }));
    present_committed(&mut shell, id, "https://slow-icon.example/");

    for attempt in 1..=FAVICON_POLL_DELAYS.len() as u8 {
        shell.handle(Command::FaviconPoll { id, attempt });
    }
    let key = (
        shell.profile_of_item(id).unwrap(),
        "https://slow-icon.example".to_owned(),
    );
    assert!(!shell.favicons.icon_attempts.contains_key(&id));
    assert_eq!(
        shell.favicons.icon_load_completion_pending.get(&id),
        Some(&key)
    );
    assert!(!shell.favicons.icons_checked.contains(&key));
    let discover = format!("discover {id}");
    let before_completion = engine
        .calls()
        .iter()
        .filter(|call| call.as_str() == discover)
        .count();

    shell.handle(Command::Engine(EngineEvent::LoadingChanged {
        id,
        loading: false,
    }));

    assert_eq!(
        engine
            .calls()
            .iter()
            .filter(|call| call.as_str() == discover)
            .count(),
        before_completion + 1
    );
    assert!(shell.favicons.icon_attempts.contains_key(&id));
    assert!(!shell
        .favicons
        .icon_load_completion_pending
        .contains_key(&id));

    shell.handle(Command::Engine(EngineEvent::FaviconPixels {
        id,
        page_url: "https://slow-icon.example/".into(),
        rgba: vec![17; zephium_core::icon::RGBA32_BYTES],
    }));

    assert!(last(&screen)
        .tabs
        .iter()
        .find(|tab| tab.id == id.to_string())
        .is_some_and(|tab| tab.icon.is_some()));
}

#[test]
fn completed_no_icon_origin_is_terminal_for_duplicate_load_events() {
    let (mut shell, engine, screen) = setup();
    shell.handle(Command::Bootstrap);
    let id = active_id(&screen);
    shell.handle(Command::Navigate {
        id,
        input: "no-icon.example".into(),
    });
    shell.handle(Command::Engine(EngineEvent::UrlChanged {
        id,
        url: "https://no-icon.example/".into(),
    }));
    for attempt in 1..=FAVICON_POLL_DELAYS.len() as u8 {
        shell.handle(Command::FaviconPoll { id, attempt });
    }
    let discover = format!("discover {id}");
    let before_completion = engine
        .calls()
        .iter()
        .filter(|call| call.as_str() == discover)
        .count();

    for _ in 0..2 {
        shell.handle(Command::Engine(EngineEvent::LoadingChanged {
            id,
            loading: false,
        }));
    }

    assert_eq!(
        engine
            .calls()
            .iter()
            .filter(|call| call.as_str() == discover)
            .count(),
        before_completion,
        "a completed negative result must not restart on duplicate completion events"
    );
}

#[test]
fn private_favicon_is_visible_but_never_written_to_persistent_storage() {
    let engine = Arc::new(FakeEngine::default());
    let store = Arc::new(FakeStore::default());
    let screen: Screen = Arc::new(Mutex::new(ItemsState {
        projection_revision: String::new(),
        profile: None,
        spaces: Vec::new(),
        active_space_id: None,
        nodes: Vec::new(),
        tabs: Vec::new(),
        active: None,
        split_group: None,
    }));
    let sink = screen.clone();
    let mut shell = Shell::new(
        engine,
        store.clone(),
        Arc::new(FakeChrome),
        Box::new(move |projection| {
            apply_projection(&mut sink.lock().unwrap(), projection);
        }),
    );
    shell.handle(Command::SetWindowSize(Size::new(1200.0, 800.0)));
    shell.handle(Command::Bootstrap);

    let profile = ProfileId::from(9001);
    let space = SpaceId::from(9002);
    assert!(shell.profiles.insert(Profile {
        id: profile,
        name: "Private".into(),
        kind: ProfileKind::Incognito,
    }));
    assert!(shell.initialize_new_blocker_profile(profile));
    assert!(shell.spaces.insert(Space {
        id: space,
        profile,
        name: "Private".into(),
        look: Default::default(),
    }));
    shell
        .windows
        .create(WindowKind::Main, profile, space, Size::new(1200.0, 800.0));
    shell.handle(Command::Open);
    let id = active_id(&screen);
    shell.handle(Command::Navigate {
        id,
        input: "private.example".into(),
    });
    shell.handle(Command::Engine(EngineEvent::UrlChanged {
        id,
        url: "https://private.example/".into(),
    }));
    present_committed(&mut shell, id, "https://private.example/");
    shell.handle(Command::Engine(EngineEvent::FaviconPixels {
        id,
        page_url: "https://private.example/".into(),
        rgba: vec![9; zephium_core::icon::RGBA32_BYTES],
    }));

    assert!(store.icons.lock().unwrap().is_empty());
    assert!(last(&screen)
        .tabs
        .iter()
        .find(|tab| tab.id == id.to_string())
        .is_some_and(|tab| tab.icon.is_some()));
}

#[test]
fn a_favourite_tab_restores_its_icon_like_any_other() {
    let store = Arc::new(FakeStore::default());
    let (mut first, _engine, screen) = setup_with(store.clone());
    first.handle(Command::Bootstrap);
    let id = active_id(&screen);
    navigate_and_commit(&mut first, id, "kept.example");
    first.handle(Command::SetTabEssential {
        id,
        essential: true,
        before: None,
    });
    store.icons.lock().unwrap().push((
        "https://kept.example".to_owned(),
        vec![19; zephium_core::icon::RGBA32_BYTES],
    ));
    drop(first);

    let (mut restored, _engine, restored_screen) = setup_with(store);
    restored.handle(Command::Bootstrap);

    let state = last(&restored_screen);
    let kept = state
        .tabs
        .iter()
        .find(|tab| tab.url.as_deref() == Some("https://kept.example/"))
        .expect("the favourite survives the restart");
    assert_eq!(
        kept.icon.as_ref().map(|icon| icon.origin.as_str()),
        Some("https://kept.example")
    );
}

#[test]
fn a_stored_icon_is_drawn_whatever_its_age_while_a_newer_one_is_fetched() {
    let store = Arc::new(FakeStore::default());
    store.icons.lock().unwrap().push((
        "https://ancient.example".to_owned(),
        vec![23; zephium_core::icon::RGBA32_BYTES],
    ));
    store
        .icon_ages
        .lock()
        .unwrap()
        .insert("https://ancient.example".to_owned(), 400 * 24 * 3600);

    let (mut shell, engine, screen) = setup_with(store);
    shell.handle(Command::Bootstrap);
    let id = active_id(&screen);
    navigate_and_commit(&mut shell, id, "ancient.example");

    let profile = shell.windows.focused().unwrap().profile;
    assert!(shell
        .icon_ref_for_url(
            zephium_ipc::IconSurface::Chrome,
            profile,
            "https://ancient.example/"
        )
        .is_some());
    assert!(engine
        .calls()
        .iter()
        .any(|call| call == &format!("discover {id}")));
}

#[test]
fn identical_pixels_are_sent_once_and_a_reattached_surface_gets_them_again() {
    let (mut shell, _engine, screen, icons) = setup_with_icon_log(Arc::new(FakeStore::default()));
    shell.handle(Command::Bootstrap);
    let id = active_id(&screen);
    navigate_and_commit(&mut shell, id, "once.example");
    let rgba = vec![55; zephium_core::icon::RGBA32_BYTES];

    shell.handle(Command::Engine(EngineEvent::FaviconPixels {
        id,
        page_url: "https://once.example/".into(),
        rgba: rgba.clone(),
    }));
    let delivered: usize = icons.lock().unwrap().iter().map(|v| v.entries.len()).sum();
    assert_eq!(delivered, 1);

    // The renderer rediscovering the same pixels must not resend them.
    shell.handle(Command::Engine(EngineEvent::FaviconPixels {
        id,
        page_url: "https://once.example/".into(),
        rgba,
    }));
    let delivered: usize = icons.lock().unwrap().iter().map(|v| v.entries.len()).sum();
    assert_eq!(delivered, 1);

    // A chrome reload leaves the webview with an empty raster cache.
    shell.handle(Command::Bootstrap);
    let entries: Vec<_> = icons
        .lock()
        .unwrap()
        .iter()
        .flat_map(|view| view.entries.iter().map(|entry| entry.origin.clone()))
        .collect();
    assert_eq!(entries, ["https://once.example", "https://once.example"]);
    assert!(icons
        .lock()
        .unwrap()
        .iter()
        .all(|view| view.surface == zephium_ipc::IconSurface::Chrome));
}

fn delivered_origins(icons: &IconLog) -> Vec<String> {
    icons
        .lock()
        .unwrap()
        .iter()
        .filter(|view| view.surface == zephium_ipc::IconSurface::Chrome)
        .flat_map(|view| view.entries.iter().map(|entry| entry.origin.clone()))
        .collect()
}

#[test]
fn returning_from_a_private_profile_resends_unchanged_favicons() {
    let (mut shell, _engine, screen, icons) = setup_with_icon_log(Arc::new(FakeStore::default()));
    shell.handle(Command::Bootstrap);
    let regular = active_id(&screen);
    navigate_and_commit(&mut shell, regular, "kept.example");
    shell.handle(Command::Engine(EngineEvent::FaviconPixels {
        id: regular,
        page_url: "https://kept.example/".into(),
        rgba: vec![5; zephium_core::icon::RGBA32_BYTES],
    }));
    shell.handle(Command::Run("window.newPrivate".into()));
    let private = active_id(&screen);
    let private_profile = shell.windows.focused().unwrap().profile;
    assert!(icons
        .lock()
        .unwrap()
        .iter()
        .any(|view| { view.profile_id == private_profile.to_string() && view.entries.is_empty() }));
    shell.handle(Command::Close(private));
    assert_eq!(
        delivered_origins(&icons),
        ["https://kept.example", "https://kept.example"]
    );
}

#[test]
fn a_background_profiles_favicon_does_not_replace_foreground_rasters() {
    let (mut shell, _engine, _screen, icons) = setup_with_icon_log(Arc::new(FakeStore::default()));
    shell.handle(Command::Bootstrap);
    let regular_profile = shell.windows.focused().unwrap().profile;
    shell.handle(Command::Run("window.newPrivate".into()));
    icons.lock().unwrap().clear();
    shell.handle(Command::Engine(EngineEvent::WorkPageFavicon {
        profile: regular_profile,
        page_url: "https://background.example/".into(),
        rgba: vec![6; zephium_core::icon::RGBA32_BYTES],
    }));
    assert!(shell
        .favicons
        .icon_values
        .contains_key(&(regular_profile, "https://background.example".into())));
    assert!(icons.lock().unwrap().is_empty());
}

#[test]
fn work_page_read_feeds_the_favicon_cache_by_origin() {
    let store = Arc::new(FakeStore::default());
    let (mut shell, _engine, _screen, icons) = setup_with_icon_log(store.clone());
    shell.handle(Command::Bootstrap);
    let profile = shell.windows.focused().unwrap().profile;

    shell.handle(Command::Engine(EngineEvent::WorkPageFavicon {
        profile,
        page_url: "https://docs.example/guide".into(),
        rgba: vec![3; zephium_core::icon::RGBA32_BYTES - 1],
    }));
    shell.handle(Command::Engine(EngineEvent::WorkPageFavicon {
        profile: ProfileId::from(0xdead_u128),
        page_url: "https://other.example/".into(),
        rgba: vec![3; zephium_core::icon::RGBA32_BYTES],
    }));
    assert!(shell.favicons.icon_values.is_empty());

    let rgba = vec![3; zephium_core::icon::RGBA32_BYTES];
    shell.handle(Command::Engine(EngineEvent::WorkPageFavicon {
        profile,
        page_url: "https://docs.example/guide".into(),
        rgba: rgba.clone(),
    }));
    assert!(shell
        .favicons
        .icon_values
        .contains_key(&(profile, "https://docs.example".to_owned())));
    assert_eq!(
        store.icons.lock().unwrap().as_slice(),
        &[(String::from("https://docs.example"), rgba)]
    );
    assert_eq!(delivered_origins(&icons), ["https://docs.example"]);
}

type ProbeLog = Arc<Mutex<Vec<String>>>;

fn attach_prober(shell: &mut Shell) -> ProbeLog {
    let log: ProbeLog = Arc::new(Mutex::new(Vec::new()));
    let sink = log.clone();
    shell.handle(Command::AttachFaviconProber(
        crate::FaviconProberAttachment(Arc::new(move |_, origin| {
            sink.lock().unwrap().push(origin)
        })),
    ));
    log
}

#[test]
fn a_page_that_gave_no_pixels_in_its_budget_is_probed_for_its_icon() {
    let (mut shell, _engine, screen) = setup();
    shell.handle(Command::Bootstrap);
    let probed = attach_prober(&mut shell);
    let id = active_id(&screen);
    shell.handle(Command::Navigate {
        id,
        input: "busy-app.example".into(),
    });
    shell.handle(Command::Engine(EngineEvent::UrlChanged {
        id,
        url: "https://busy-app.example/".into(),
    }));
    present_committed(&mut shell, id, "https://busy-app.example/");
    for attempt in 1..=FAVICON_POLL_DELAYS.len() as u8 {
        shell.handle(Command::FaviconPoll { id, attempt });
    }
    assert_eq!(
        probed.lock().unwrap().as_slice(),
        ["https://busy-app.example"]
    );

    let profile = shell.profile_of_item(id).unwrap();
    shell.handle(Command::FaviconProbed {
        profile,
        origin: "https://busy-app.example".into(),
        rgba: Some(vec![23; zephium_core::icon::RGBA32_BYTES]),
    });
    assert!(last(&screen)
        .tabs
        .iter()
        .find(|tab| tab.id == id.to_string())
        .is_some_and(|tab| tab.icon.is_some()));
}

#[test]
fn favicon_probe_delivers_held_icons_and_asks_four_origins_at_a_time() {
    let store = Arc::new(FakeStore::default());
    let held = vec![5; zephium_core::icon::RGBA32_BYTES];
    store.icons.lock().unwrap().extend([
        ("https://held.example".to_owned(), held.clone()),
        ("https://old.example".to_owned(), held.clone()),
    ]);
    store.icon_ages.lock().unwrap().extend([
        ("https://held.example".to_owned(), 60),
        (
            "https://old.example".to_owned(),
            FAVICON_CACHE_MAX_AGE_SECONDS + 1,
        ),
    ]);
    let (mut shell, _engine, _screen, icons) = setup_with_icon_log(store.clone());
    shell.handle(Command::Bootstrap);
    let profile = shell.windows.focused().unwrap().profile;
    let probed = attach_prober(&mut shell);

    let origins = [
        "https://held.example/a",
        "https://old.example/",
        "https://a.example/x",
        "https://b.example/",
        "https://c.example/",
        "https://d.example/",
        "https://a.example/y",
        "http://plain.example/",
    ];
    shell.handle(Command::ProbeFavicons {
        profile,
        origins: origins.iter().map(|origin| origin.to_string()).collect(),
    });
    let delivered = delivered_origins(&icons);
    assert!(delivered.contains(&"https://held.example".to_owned()));
    assert!(delivered.contains(&"https://old.example".to_owned()));
    assert_eq!(
        probed.lock().unwrap().as_slice(),
        [
            "https://old.example",
            "https://a.example",
            "https://b.example",
            "https://c.example"
        ]
    );

    let rgba = vec![9; zephium_core::icon::RGBA32_BYTES];
    shell.handle(Command::FaviconProbed {
        profile,
        origin: "https://a.example".into(),
        rgba: Some(rgba.clone()),
    });
    assert!(shell
        .favicons
        .icon_values
        .contains_key(&(profile, "https://a.example".to_owned())));
    assert!(store
        .icons
        .lock()
        .unwrap()
        .contains(&("https://a.example".to_owned(), rgba)));
    assert!(delivered_origins(&icons).contains(&"https://a.example".to_owned()));
    assert_eq!(probed.lock().unwrap().last().unwrap(), "https://d.example");

    // An answer nobody asked for is ignored.
    shell.handle(Command::FaviconProbed {
        profile,
        origin: "https://unasked.example".into(),
        rgba: Some(vec![1; zephium_core::icon::RGBA32_BYTES]),
    });
    assert!(!shell
        .favicons
        .icon_values
        .contains_key(&(profile, "https://unasked.example".to_owned())));
}

#[test]
fn a_failed_origin_is_not_probed_again_within_the_hour() {
    let (mut shell, _engine, _screen, _icons) = setup_with_icon_log(Arc::new(FakeStore::default()));
    shell.handle(Command::Bootstrap);
    let profile = shell.windows.focused().unwrap().profile;
    let probed = attach_prober(&mut shell);
    let ask = |shell: &mut Shell| {
        shell.handle(Command::ProbeFavicons {
            profile,
            origins: vec!["https://quiet.example/".into()],
        })
    };

    ask(&mut shell);
    // Still in flight: a second request does not start another probe.
    ask(&mut shell);
    assert_eq!(probed.lock().unwrap().len(), 1);
    shell.handle(Command::FaviconProbed {
        profile,
        origin: "https://quiet.example".into(),
        rgba: None,
    });
    ask(&mut shell);
    assert_eq!(probed.lock().unwrap().len(), 1);

    // A raster that is not the fixed shape counts as a failure too.
    shell.handle(Command::ProbeFavicons {
        profile,
        origins: vec!["https://odd.example/".into()],
    });
    shell.handle(Command::FaviconProbed {
        profile,
        origin: "https://odd.example".into(),
        rgba: Some(vec![1; 16]),
    });
    shell.handle(Command::ProbeFavicons {
        profile,
        origins: vec!["https://odd.example/".into()],
    });
    assert_eq!(probed.lock().unwrap().len(), 2);
    assert!(shell.favicons.icon_values.is_empty());
}

#[test]
fn a_listed_address_without_an_icon_is_fetched_once_and_sent_to_the_surface_that_asked() {
    let store = Arc::new(FakeStore::default());
    let stored = vec![4; zephium_core::icon::RGBA32_BYTES];
    store
        .icons
        .lock()
        .unwrap()
        .push(("https://stored.example".to_owned(), stored));
    store
        .icon_ages
        .lock()
        .unwrap()
        .insert("https://stored.example".to_owned(), 60);
    let (mut shell, _engine, _screen, icons) = setup_with_icon_log(store);
    shell.handle(Command::Bootstrap);
    let profile = shell.windows.focused().unwrap().profile;
    let probed = attach_prober(&mut shell);
    let panel_origins = |icons: &IconLog| -> Vec<String> {
        icons
            .lock()
            .unwrap()
            .iter()
            .filter(|view| view.surface == zephium_ipc::IconSurface::Panel)
            .flat_map(|view| view.entries.iter().map(|entry| entry.origin.clone()))
            .collect()
    };

    shell.want_icons(
        zephium_ipc::IconSurface::Panel,
        profile,
        [
            "https://stored.example/page",
            "https://music.example/album/1",
            "https://music.example/album/2",
            "http://plain.example/",
        ],
    );
    shell.publish_icons();
    // The store answers first; only what it lacks goes to the network, once.
    assert_eq!(panel_origins(&icons), ["https://stored.example"]);
    assert_eq!(probed.lock().unwrap().as_slice(), ["https://music.example"]);

    shell.handle(Command::FaviconProbed {
        profile,
        origin: "https://music.example".into(),
        rgba: Some(vec![7; zephium_core::icon::RGBA32_BYTES]),
    });
    assert!(panel_origins(&icons).contains(&"https://music.example".to_owned()));

    // Now held: listing it again asks nothing.
    shell.want_icons(
        zephium_ipc::IconSurface::Panel,
        profile,
        ["https://music.example/album/3"],
    );
    assert_eq!(probed.lock().unwrap().len(), 1);
}

#[test]
fn an_essential_never_opened_is_given_its_icon_from_the_store_or_the_network() {
    let store = Arc::new(FakeStore::default());
    store.icons.lock().unwrap().push((
        "https://kept.example".to_owned(),
        vec![6; zephium_core::icon::RGBA32_BYTES],
    ));
    let (mut shell, _engine, _screen, icons) = setup_with_icon_log(store);
    shell.handle(Command::Bootstrap);
    let window = shell.windows.focused().unwrap();
    let (profile, space) = (window.profile, window.space);
    let probed = attach_prober(&mut shell);
    let site = |url: &str| crate::ImportedSite {
        url: url.into(),
        title: String::new(),
    };
    shell.handle(Command::Import {
        destination: profile,
        work: Box::new(crate::ImportWork::Essentials(vec![
            site("https://kept.example/inbox"),
            site("https://bare.example/"),
        ])),
        done: crate::ImportCompletion::new(|_| {}),
    });

    shell.hydrate_favicon_cache(profile, space);
    assert!(delivered_origins(&icons).contains(&"https://kept.example".to_owned()));
    assert_eq!(probed.lock().unwrap().as_slice(), ["https://bare.example"]);
}

#[derive(Default)]
struct Surface {
    owner: Option<String>,
    held: std::collections::HashSet<(String, String)>,
}

impl Surface {
    fn replay(icons: &IconLog) -> Self {
        let mut surface = Self::default();
        for view in icons
            .lock()
            .unwrap()
            .iter()
            .filter(|view| view.surface == zephium_ipc::IconSurface::Chrome)
        {
            surface.owner = Some(view.profile_id.clone());
            for entry in &view.entries {
                surface
                    .held
                    .insert((entry.origin.clone(), entry.revision.clone()));
            }
        }
        surface
    }

    fn draws(&self, state: &ItemsState) -> bool {
        let icons: Vec<_> = state
            .tabs
            .iter()
            .filter_map(|tab| tab.icon.as_ref())
            .collect();
        !icons.is_empty()
            && icons.iter().all(|icon| {
                self.held
                    .contains(&(icon.origin.clone(), icon.revision.clone()))
            })
    }
}

fn paint(shell: &mut Shell, id: ItemId, host: &str, fill: u8) {
    navigate_and_commit(shell, id, host);
    shell.handle(Command::Engine(EngineEvent::FaviconPixels {
        id,
        page_url: format!("https://{host}/"),
        rgba: vec![fill; zephium_core::icon::RGBA32_BYTES],
    }));
}

#[test]
fn icons_survive_switching_profiles_and_coming_back() {
    let (mut shell, _engine, screen, icons) = setup_with_icon_log(Arc::new(FakeStore::default()));
    shell.handle(Command::Bootstrap);
    let personal = shell.windows.focused().unwrap().profile;
    paint(&mut shell, active_id(&screen), "shared.example", 21);

    shell.handle_operation(Command::CreateProfile {
        name: "Work".into(),
        avatar: zephium_core::profiles::ProfileAvatar {
            style: zephium_core::profiles::ProfileAvatarStyle::Symbol,
            color: 2,
            symbol: 5,
            revision: 0,
        },
        photo: None,
        switch: false,
    });
    let work = shell
        .profiles
        .iter()
        .map(|profile| profile.id)
        .find(|id| *id != personal)
        .unwrap();
    shell.handle_operation(Command::SwitchProfile(work));
    paint(&mut shell, active_id(&screen), "shared.example", 99);
    let surface = Surface::replay(&icons);
    assert_eq!(surface.owner.as_deref(), Some(&*work.to_string()));
    assert!(surface.draws(&last(&screen)));

    shell.handle_operation(Command::SwitchProfile(personal));
    let surface = Surface::replay(&icons);
    assert_eq!(surface.owner.as_deref(), Some(&*personal.to_string()));
    assert!(surface.draws(&last(&screen)));

    shell.handle_operation(Command::SwitchProfile(work));
    shell.handle_operation(Command::SwitchProfile(personal));
    assert!(Surface::replay(&icons).draws(&last(&screen)));
}

#[test]
fn a_returning_profile_is_announced_even_when_every_icon_is_already_held() {
    let (mut shell, _engine, screen, icons) = setup_with_icon_log(Arc::new(FakeStore::default()));
    shell.handle(Command::Bootstrap);
    let personal = shell.windows.focused().unwrap().profile;
    paint(&mut shell, active_id(&screen), "home.example", 5);
    let work = {
        shell.handle_operation(Command::CreateProfile {
            name: "Work".into(),
            avatar: zephium_core::profiles::ProfileAvatar {
                style: zephium_core::profiles::ProfileAvatarStyle::Symbol,
                color: 2,
                symbol: 5,
                revision: 0,
            },
            photo: None,
            switch: false,
        });
        shell
            .profiles
            .iter()
            .map(|profile| profile.id)
            .find(|id| *id != personal)
            .unwrap()
    };
    shell.handle_operation(Command::SwitchProfile(work));
    let before = icons.lock().unwrap().len();
    shell.handle_operation(Command::SwitchProfile(personal));
    let views = icons.lock().unwrap();
    let announced = views[before..]
        .iter()
        .any(|view| view.profile_id == personal.to_string() && view.entries.is_empty());
    assert!(announced);
}
