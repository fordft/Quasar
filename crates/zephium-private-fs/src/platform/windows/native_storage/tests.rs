use super::*;

#[test]
fn activation_is_gated_before_known_folder_or_directory_creation() {
    #[cfg(not(feature = "windows-namespace-validation"))]
    assert!(matches!(
        NativeStorageAnchor::prepare(NativeApplication::Product, None),
        Err(PrivateFsError::PrimitiveUnavailable)
    ));
    assert_eq!(
        admit_namespace_support().is_ok(),
        cfg!(feature = "windows-namespace-validation")
    );
}

#[test]
fn session_labels_cannot_select_paths_or_streams() {
    assert!(NativeSession::new("qualification-20261002").is_ok());
    for value in [
        "",
        "..",
        "../product",
        "a/b",
        "a\\b",
        "a:b",
        "a b",
        "a.",
        "é",
    ] {
        assert_eq!(NativeSession::new(value), Err(PrivateFsError::Unsafe));
    }
    assert!(NativeSession::new(&"a".repeat(49)).is_err());
}

#[cfg(feature = "windows-namespace-validation")]
mod validation {
    use super::*;
    use crate::LockedPrivateNamespace;
    use tempfile::TempDir;

    fn fixture() -> TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn product_qa_and_sessions_are_exclusive_and_inherit_only_trusted_rights() {
        let fixture = fixture();
        let product =
            NativeStorageAnchor::prepare_under(fixture.path(), NativeApplication::Product, None)
                .unwrap();
        assert_eq!(
            product.directory(),
            fixture.path().join(ANCHOR).join("app.quasar")
        );
        let expected = product.identity();
        let review =
            NativeStorageAnchor::prepare_under(fixture.path(), NativeApplication::ReviewQa, None)
                .unwrap();
        assert_eq!(
            review.directory(),
            fixture.path().join(ANCHOR).join("app.quasar.review-qa")
        );
        assert_ne!(review.identity(), expected);
        assert!(matches!(
            NativeStorageAnchor::prepare_under(fixture.path(), NativeApplication::ReviewQa, None),
            Err(PrivateFsError::LockUnavailable)
        ));
        assert!(matches!(
            NativeStorageAnchor::prepare_under(fixture.path(), NativeApplication::Product, None),
            Err(PrivateFsError::LockUnavailable)
        ));
        let qa = NativeStorageAnchor::prepare_under(
            fixture.path(),
            NativeApplication::ExtensionQa,
            None,
        )
        .unwrap();
        let session = NativeSession::new("one").unwrap();
        let session_anchor = NativeStorageAnchor::prepare_under(
            fixture.path(),
            NativeApplication::ExtensionQa,
            Some(&session),
        )
        .unwrap();
        assert_ne!(product.directory(), qa.directory());
        assert_eq!(
            session_anchor.directory(),
            qa.directory().join("qa-sessions/session-one")
        );
        let payload = product.directory().join("native.sqlite");
        std::fs::write(&payload, b"nonsecret fixture").unwrap();
        let file = File::open(&payload).unwrap();
        security::native_storage::verify_inherited(&file, false).unwrap();
        let child = product.directory().join("sqlite-parent");
        std::fs::create_dir(&child).unwrap();
        let directory = native::open(
            &product.directories.last().unwrap().file,
            "sqlite-parent",
            Some(true),
            native::READ,
            7,
            native::OPEN,
            None,
        )
        .unwrap();
        security::native_storage::verify_inherited(&directory, true).unwrap();
        // The ancestor's inheritable contract does not silently become a private
        // leaf contract; owned private namespaces retain their existing ACL mode.
        assert!(security::mode(&product.directories.last().unwrap().file).is_err());
        let private =
            LockedPrivateNamespace::open_or_create(product.directory().join("work-execution"))
                .unwrap();
        private
            .directory()
            .with_verified_path(|_| Ok::<_, PrivateFsError>(()))
            .unwrap()
            .unwrap();
        product.verify().unwrap();
        drop(private);
        drop(product);
        let reopened =
            NativeStorageAnchor::prepare_under(fixture.path(), NativeApplication::Product, None)
                .unwrap();
        assert_eq!(reopened.identity(), expected);
        reopened.verify().unwrap();
    }

    #[test]
    fn existing_wrong_acl_is_refused_without_repair_or_payload_loss() {
        let fixture = fixture();
        let parent = super::super::super::absolute_directory(fixture.path(), false).unwrap();
        let descriptor = security::descriptor(false, true).unwrap();
        let directory = native::open(
            &parent,
            ANCHOR,
            Some(true),
            native::READ | native::WRITE,
            7,
            native::CREATE,
            Some(descriptor.0),
        )
        .unwrap();
        let before = security::snapshot(&directory).unwrap();
        let payload = fixture.path().join(ANCHOR).join("preserved");
        std::fs::write(&payload, b"exact fixture bytes").unwrap();
        assert!(matches!(
            NativeStorageAnchor::prepare_under(fixture.path(), NativeApplication::Product, None),
            Err(PrivateFsError::Unsafe)
        ));
        assert!(security::snapshot(&directory).unwrap() == before);
        assert_eq!(std::fs::read(payload).unwrap(), b"exact fixture bytes");
        assert!(!fixture.path().join(ANCHOR).join("app.quasar").exists());
    }

    #[test]
    fn held_parent_blocks_replacement_and_acl_drift_quarantines_original_lease() {
        let fixture = fixture();
        let anchor =
            NativeStorageAnchor::prepare_under(fixture.path(), NativeApplication::Product, None)
                .unwrap();
        assert!(std::fs::rename(anchor.directory(), fixture.path().join("replacement")).is_err());
        let lock = anchor.directory().join(LEASE);
        assert!(std::fs::remove_file(&lock).is_err());
        assert!(std::fs::rename(&lock, anchor.directory().join("replacement-lock")).is_err());
        anchor.verify().unwrap();
        let parent = &anchor.directories[anchor.directories.len() - 2].file;
        let acl_handle = native::open(
            parent,
            "app.quasar",
            Some(true),
            native::READ | 0x0004_0000,
            3,
            native::OPEN,
            None,
        )
        .unwrap();
        security::set_mode(&acl_handle, false).unwrap();
        assert_eq!(anchor.verify(), Err(PrivateFsError::Unsafe));
        assert_eq!(anchor.verify(), Err(PrivateFsError::Quarantined));
        assert!(matches!(
            NativeStorageAnchor::prepare_under(fixture.path(), NativeApplication::Product, None),
            Err(PrivateFsError::Unsafe)
        ));
    }

    #[test]
    fn product_session_refusal_does_not_create_any_anchor() {
        let fixture = fixture();
        let session = NativeSession::new("one").unwrap();
        assert!(matches!(
            NativeStorageAnchor::prepare_under(
                fixture.path(),
                NativeApplication::Product,
                Some(&session)
            ),
            Err(PrivateFsError::Unsafe)
        ));
        assert!(!fixture.path().join(ANCHOR).exists());
    }

    #[test]
    fn corrupt_existing_lock_is_preserved_and_never_truncated() {
        let fixture = fixture();
        let anchor =
            NativeStorageAnchor::prepare_under(fixture.path(), NativeApplication::Product, None)
                .unwrap();
        let path = anchor.directory().join(LEASE);
        drop(anchor);
        std::fs::write(&path, b"foreign lock bytes").unwrap();
        assert!(matches!(
            NativeStorageAnchor::prepare_under(fixture.path(), NativeApplication::Product, None),
            Err(PrivateFsError::Unsafe)
        ));
        assert_eq!(std::fs::read(path).unwrap(), b"foreign lock bytes");
    }

    #[test]
    fn case_colliding_session_is_refused_without_retargeting_existing_session() {
        let fixture = fixture();
        let original = NativeSession::new("Case").unwrap();
        let anchor = NativeStorageAnchor::prepare_under(
            fixture.path(),
            NativeApplication::ExtensionQa,
            Some(&original),
        )
        .unwrap();
        let collision = NativeSession::new("case").unwrap();
        assert!(matches!(
            NativeStorageAnchor::prepare_under(
                fixture.path(),
                NativeApplication::ExtensionQa,
                Some(&collision)
            ),
            Err(PrivateFsError::IdentityAmbiguous)
        ));
        anchor.verify().unwrap();
        assert!(anchor.directory().ends_with("session-Case"));
    }

    #[test]
    #[ignore = "explicit read-only admission of this account's OS Profile KnownFolder"]
    fn known_folder_profile_chain_is_admitted_without_creating_native_storage() {
        let profile = known_profile().unwrap();
        let existed = profile.join(ANCHOR).try_exists().unwrap();
        let (_drive, directories) = pin_profile(&profile).unwrap();
        for directory in &directories {
            assert_eq!(
                identity(&directory.file, Some(true)).unwrap().0,
                directory.identity
            );
            security::ancestor(&directory.file).unwrap();
            assert!(security::snapshot(&directory.file).unwrap() == directory.security);
        }
        assert_eq!(profile.join(ANCHOR).try_exists().unwrap(), existed);
    }
}
