//! Profile-relative native storage anchor. This never owns a WebView2 UDF.

use super::{
    admit_namespace_support, identity, lock_exclusive, native, security, sync_directory,
    RawIdentity,
};
use crate::{DirectoryIdentity, PrivateEntryName, PrivateFsError};
use std::fs::File;
use std::os::windows::ffi::OsStringExt;
use std::os::windows::fs::OpenOptionsExt;
use std::path::{Component, Path, PathBuf, Prefix};
use std::sync::atomic::{AtomicBool, Ordering};
use windows::Win32::Storage::FileSystem::{
    FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
};
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::UI::Shell::{FOLDERID_Profile, SHGetKnownFolderPath, KF_FLAG_DEFAULT};

const ANCHOR: &str = ".zephium-native-v1";
const LEASE: &str = ".zephium-native-anchor-lock-v1";

mod work;
pub use work::{NativeWorkStorageAnchor, NativeWorkStorageFile};
#[cfg(feature = "windows-work-test-fixtures")]
mod work_fixture;
#[cfg(feature = "windows-work-test-fixtures")]
pub use work_fixture::NativeWorkStorageTestSession;

/// Closed application identities; callers cannot select an arbitrary directory.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeApplication {
    /// The ordinary Zephium installation.
    Product,
    /// `pnpm dev` builds, kept apart from the installed product's profile.
    Development,
    /// The separately identified browser hardening review installation.
    ReviewQa,
    /// The separately identified extension/Work QA installation.
    ExtensionQa,
    /// File-grant integration qualification.
    FilesIntegrationQa,
    /// Performance qualification.
    Performance,
    /// Protection qualification.
    ProtectionQa,
    /// Retained Work integration qualification.
    WorkIntegration,
    /// Work navigation qualification.
    WorkNavigationProbe,
    /// Work rendering qualification.
    WorkRenderingProbe,
}

impl NativeApplication {
    fn component(self) -> &'static str {
        match self {
            Self::Product => "app.quasar",
            Self::Development => "app.quasar.dev",
            Self::ReviewQa => "app.quasar.review-qa",
            Self::ExtensionQa => "app.quasar.webext-qa",
            Self::FilesIntegrationQa => "app.quasar.files-integration-qa",
            Self::Performance => "app.quasar.performance",
            Self::ProtectionQa => "app.quasar.protection-qa",
            Self::WorkIntegration => "app.quasar.work-integration",
            Self::WorkNavigationProbe => "app.quasar.work-navigation-probe",
            Self::WorkRenderingProbe => "app.quasar.work-rendering-probe",
        }
    }
}

/// A bounded QA session label, never a pathname or product-root override.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeSession(String);

impl NativeSession {
    /// Accepts 1–48 ASCII letters, digits or hyphens.
    pub fn new(value: &str) -> Result<Self, PrivateFsError> {
        if value.is_empty()
            || value.len() > 48
            || !value
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        {
            return Err(PrivateFsError::Unsafe);
        }
        Ok(Self(value.to_owned()))
    }
}

struct HeldDirectory {
    file: File,
    identity: RawIdentity,
    name: Option<String>,
    security: security::SecuritySnapshot,
    anchor: bool,
}

/// Held, exclusively leased native storage beneath the OS Profile KnownFolder.
///
/// New anchor directories receive a protected inheritable current-user/SYSTEM/
/// Administrators ACL. Existing permissions are only inspected, never repaired.
/// Every parent is pinned without delete sharing. This capability does not admit
/// a private namespace or a WebView2 UDF, and the existing Windows debug validation
/// gate applies before KnownFolder resolution or any filesystem access.
pub struct NativeStorageAnchor {
    path: PathBuf,
    drive: u8,
    directories: Vec<HeldDirectory>,
    lock: File,
    lock_identity: RawIdentity,
    quarantined: AtomicBool,
}

/// Exact inherited file identity held without delete sharing under an anchor.
pub struct NativeStorageFile {
    file: File,
    name: PrivateEntryName,
    identity: RawIdentity,
    root: DirectoryIdentity,
    security: security::SecuritySnapshot,
}

impl NativeStorageAnchor {
    /// Creates or admits the fixed product/QA anchor and takes its exclusive lease.
    ///
    /// A session is valid only for QA. Product and QA never share the application
    /// directory or lock. No missing Profile ancestor is created.
    pub fn prepare(
        application: NativeApplication,
        session: Option<&NativeSession>,
    ) -> Result<Self, PrivateFsError> {
        admit_namespace_support()?;
        let profile = known_profile()?;
        Self::prepare_under(&profile, application, session)
    }

    fn prepare_under(
        profile: &Path,
        application: NativeApplication,
        session: Option<&NativeSession>,
    ) -> Result<Self, PrivateFsError> {
        admit_namespace_support()?;
        Self::prepare_under_impl(profile, application, session)
    }

    // Only the gated generic and opaque KnownFolder Work factories enter this
    // common proof. No global admission flag is changed or consumed here.
    fn prepare_under_impl(
        profile: &Path,
        application: NativeApplication,
        session: Option<&NativeSession>,
    ) -> Result<Self, PrivateFsError> {
        Self::prepare_under_impl_inner(profile, application, session, false, false)
    }

    #[cfg(feature = "windows-work-test-fixtures")]
    fn prepare_under_fresh_session(
        profile: &Path,
        session: &NativeSession,
    ) -> Result<Self, PrivateFsError> {
        Self::prepare_under_impl_inner(
            profile,
            NativeApplication::ExtensionQa,
            Some(session),
            true,
            false,
        )
    }

    #[cfg(any(test, feature = "windows-work-test-fixtures"))]
    fn prepare_under_existing_session(
        profile: &Path,
        session: &NativeSession,
    ) -> Result<Self, PrivateFsError> {
        Self::prepare_under_impl_inner(
            profile,
            NativeApplication::ExtensionQa,
            Some(session),
            false,
            true,
        )
    }

    fn prepare_under_impl_inner(
        profile: &Path,
        application: NativeApplication,
        session: Option<&NativeSession>,
        fresh_session: bool,
        existing_only: bool,
    ) -> Result<Self, PrivateFsError> {
        if application != NativeApplication::ExtensionQa && session.is_some() {
            return Err(PrivateFsError::Unsafe);
        }
        let (drive, mut directories) = pin_profile(profile)?;
        let mut mutated = false;
        let result = (|| {
            let mut path = profile.to_path_buf();
            for name in [ANCHOR, application.component()] {
                append_anchor(&mut directories, name, &mut mutated, false, existing_only)?;
                path.push(name);
            }
            if let Some(session) = session {
                append_anchor(
                    &mut directories,
                    "qa-sessions",
                    &mut mutated,
                    false,
                    existing_only,
                )?;
                path.push("qa-sessions");
                let name = format!("session-{}", session.0);
                append_anchor(
                    &mut directories,
                    &name,
                    &mut mutated,
                    fresh_session,
                    existing_only,
                )?;
                path.push(name);
            }
            let parent = &directories.last().ok_or(PrivateFsError::Unsafe)?.file;
            let sd = security::descriptor(false, false)?;
            let lock = if existing_only {
                native::open(
                    parent,
                    LEASE,
                    Some(false),
                    native::READ | native::WRITE,
                    3,
                    native::OPEN,
                    None,
                )?
            } else {
                match native::open(
                    parent,
                    LEASE,
                    Some(false),
                    native::READ | native::WRITE,
                    3,
                    native::CREATE,
                    Some(sd.0),
                ) {
                    Ok(file) => {
                        mutated = true;
                        file
                    }
                    Err(PrivateFsError::AlreadyExists) => native::open(
                        parent,
                        LEASE,
                        Some(false),
                        native::READ | native::WRITE,
                        3,
                        native::OPEN,
                        None,
                    )?,
                    Err(error) => return Err(error),
                }
            };
            let lock_identity = identity(&lock, Some(false))?.0;
            verify_lock(&lock)?;
            lock_exclusive(&lock)?;
            // A concurrent creator may have lost the lock before flushing its
            // publication. The successful owner settles either exact empty lock.
            lock.sync_all()
                .map_err(|_| PrivateFsError::SettlementUnknown)?;
            sync_directory(parent).map_err(|_| PrivateFsError::SettlementUnknown)?;
            let anchor = Self {
                path,
                drive,
                directories,
                lock,
                lock_identity,
                quarantined: AtomicBool::new(false),
            };
            anchor.verify()?;
            Ok(anchor)
        })();
        match result {
            Err(error)
                if mutated
                    && !matches!(
                        error,
                        PrivateFsError::LockUnavailable | PrivateFsError::SettlementUnknown
                    ) =>
            {
                Err(PrivateFsError::SettlementUnknown)
            }
            result => result,
        }
    }

    /// Returns the fixed native directory; the lease must outlive its consumers.
    ///
    /// Consumers must call [`Self::verify`] before admitting persistent state.
    /// This is deliberately not a caller-selected WebView2 data-directory path.
    #[must_use]
    pub fn directory(&self) -> &Path {
        &self.path
    }

    /// Opens one existing inherited file, pinning its exact native identity.
    /// Reparse points, hard links, alternate spellings and non-anchor ACLs refuse.
    pub fn hold_file(&self, name: &PrivateEntryName) -> Result<NativeStorageFile, PrivateFsError> {
        self.verify()?;
        let parent = &self.directories.last().ok_or(PrivateFsError::Unsafe)?.file;
        let file = native::open(
            parent,
            name.as_str(),
            Some(false),
            native::READ,
            3,
            native::OPEN,
            None,
        )?;
        native::exact_name(&file, name.as_str())?;
        security::native_storage::verify_inherited(&file, false)?;
        let held = NativeStorageFile {
            identity: identity(&file, Some(false))?.0,
            security: security::snapshot(&file)?,
            file,
            name: name.clone(),
            root: self.identity(),
        };
        held.verify(self)?;
        Ok(held)
    }

    /// Settles native child publication under the original held directory.
    pub fn sync(&self) -> Result<(), PrivateFsError> {
        self.verify()?;
        let parent = &self.directories.last().ok_or(PrivateFsError::Unsafe)?.file;
        if sync_directory(parent).is_err() {
            self.quarantined.store(true, Ordering::Release);
            return Err(PrivateFsError::SettlementUnknown);
        }
        self.verify()
    }

    /// Creates one new empty inherited file. Existing files are never truncated.
    pub fn create_file(
        &self,
        name: &PrivateEntryName,
    ) -> Result<NativeStorageFile, PrivateFsError> {
        self.verify()?;
        let parent = &self.directories.last().ok_or(PrivateFsError::Unsafe)?.file;
        let file = native::open(
            parent,
            name.as_str(),
            Some(false),
            native::READ | native::WRITE,
            3,
            native::CREATE,
            None,
        )?;
        // Keep the original no-delete handle through publication and return it
        // as the guard. Closing/reopening here could admit a replacement file.
        let result = (|| {
            native::exact_name(&file, name.as_str())?;
            security::native_storage::verify_inherited(&file, false)?;
            let held = NativeStorageFile {
                identity: identity(&file, Some(false))?.0,
                security: security::snapshot(&file)?,
                file,
                name: name.clone(),
                root: self.identity(),
            };
            held.file
                .sync_all()
                .map_err(|_| PrivateFsError::SettlementUnknown)?;
            sync_directory(parent).map_err(|_| PrivateFsError::SettlementUnknown)?;
            held.verify(self)?;
            Ok(held)
        })();
        if result.is_err() {
            // Creation already committed. Failed proof may leave a residue and
            // cannot be retried through this same original capability.
            self.quarantined.store(true, Ordering::Release);
            return Err(PrivateFsError::SettlementUnknown);
        }
        result
    }

    /// Disposable native validation fixture; unavailable in shipping builds.
    #[cfg(all(debug_assertions, feature = "windows-namespace-validation"))]
    #[doc(hidden)]
    pub fn prepare_validation_fixture(
        profile: &Path,
        application: NativeApplication,
        session: Option<&NativeSession>,
    ) -> Result<Self, PrivateFsError> {
        Self::prepare_under(profile, application, session)
    }

    /// Returns the opaque full native identity of the held application/session root.
    #[must_use]
    pub fn identity(&self) -> DirectoryIdentity {
        DirectoryIdentity(self.directories[self.directories.len() - 1].identity)
    }

    /// Revalidates every held parent, root ACL, exact spelling and lease identity.
    ///
    /// Any observed disagreement stickily quarantines this capability while its
    /// original lock and parent handles remain held until it is dropped.
    pub fn verify(&self) -> Result<(), PrivateFsError> {
        if self.quarantined.load(Ordering::Acquire) {
            return Err(PrivateFsError::Quarantined);
        }
        let result = self.verify_inner();
        if result.is_err() {
            self.quarantined.store(true, Ordering::Release);
        }
        result?;
        if self.quarantined.load(Ordering::Acquire) {
            Err(PrivateFsError::Quarantined)
        } else {
            Ok(())
        }
    }

    fn verify_inner(&self) -> Result<(), PrivateFsError> {
        let ambient = open_drive(self.drive)?;
        if identity(&ambient, Some(true))?.0 != self.directories[0].identity {
            return Err(PrivateFsError::IdentityAmbiguous);
        }
        for (index, directory) in self.directories.iter().enumerate() {
            if identity(&directory.file, Some(true))?.0 != directory.identity {
                return Err(PrivateFsError::IdentityAmbiguous);
            }
            if directory.anchor {
                security::native_storage::verify(&directory.file)?;
            } else {
                security::ancestor(&directory.file)?;
            }
            if security::snapshot(&directory.file)? != directory.security {
                return Err(PrivateFsError::IdentityAmbiguous);
            }
            if let Some(name) = &directory.name {
                native::exact_name(&directory.file, name)?;
                let reopened = native::open(
                    &self.directories[index - 1].file,
                    name,
                    Some(true),
                    native::READ,
                    3,
                    native::OPEN,
                    None,
                )?;
                if identity(&reopened, Some(true))?.0 != directory.identity {
                    return Err(PrivateFsError::IdentityAmbiguous);
                }
            }
        }
        let parent = &self.directories.last().ok_or(PrivateFsError::Unsafe)?.file;
        verify_lock(&self.lock)?;
        if identity(&self.lock, Some(false))?.0 != self.lock_identity {
            return Err(PrivateFsError::IdentityAmbiguous);
        }
        let current = native::open(
            parent,
            LEASE,
            Some(false),
            native::READ,
            3,
            native::OPEN,
            None,
        )?;
        if identity(&current, Some(false))?.0 != self.lock_identity {
            return Err(PrivateFsError::IdentityAmbiguous);
        }
        Ok(())
    }
}

impl NativeStorageFile {
    /// Revalidates the original held file and its exact binding to this anchor.
    pub fn verify(&self, anchor: &NativeStorageAnchor) -> Result<(), PrivateFsError> {
        anchor.verify()?;
        let result = (|| {
            if self.root != anchor.identity()
                || identity(&self.file, Some(false))?.0 != self.identity
            {
                return Err(PrivateFsError::IdentityAmbiguous);
            }
            security::native_storage::verify_inherited(&self.file, false)?;
            if security::snapshot(&self.file)? != self.security {
                return Err(PrivateFsError::IdentityAmbiguous);
            }
            let parent = &anchor
                .directories
                .last()
                .ok_or(PrivateFsError::Unsafe)?
                .file;
            let current = native::open(
                parent,
                self.name.as_str(),
                Some(false),
                native::READ,
                3,
                native::OPEN,
                None,
            )?;
            native::exact_name(&current, self.name.as_str())?;
            if identity(&current, Some(false))?.0 != self.identity {
                return Err(PrivateFsError::IdentityAmbiguous);
            }
            anchor.verify()
        })();
        if result.is_err() {
            anchor.quarantined.store(true, Ordering::Release);
        }
        result
    }
}

fn known_profile() -> Result<PathBuf, PrivateFsError> {
    // SAFETY: fixed KnownFolder, no token override; Windows owns the returned allocation.
    let value = unsafe { SHGetKnownFolderPath(&FOLDERID_Profile, KF_FLAG_DEFAULT, None) }
        .map_err(|_| PrivateFsError::PrimitiveUnavailable)?;
    if value.is_null() {
        return Err(PrivateFsError::PrimitiveUnavailable);
    }
    // SAFETY: successful KnownFolder result is terminated UTF-16 until CoTaskMemFree.
    let profile = PathBuf::from(std::ffi::OsString::from_wide(unsafe { value.as_wide() }));
    unsafe { CoTaskMemFree(Some(value.0.cast())) };
    Ok(profile)
}

fn open_drive(drive: u8) -> Result<File, PrivateFsError> {
    let file = File::options()
        .read(true)
        .access_mode(native::READ | native::TRAVERSE)
        .share_mode(3)
        .custom_flags((FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT).0)
        .open(format!("{}:\\", char::from(drive)))
        .map_err(|_| PrivateFsError::Unsafe)?;
    identity(&file, Some(true))?;
    native::require_local_ntfs(&file)?;
    native::require_drive_root(&file, drive)?;
    security::ancestor(&file)?;
    Ok(file)
}

fn held(file: File, name: Option<String>, anchor: bool) -> Result<HeldDirectory, PrivateFsError> {
    let id = identity(&file, Some(true))?.0;
    if anchor {
        security::native_storage::verify(&file)?;
    } else {
        security::ancestor(&file)?;
    }
    let security = security::snapshot(&file)?;
    Ok(HeldDirectory {
        file,
        identity: id,
        name,
        security,
        anchor,
    })
}

fn pin_profile(profile: &Path) -> Result<(u8, Vec<HeldDirectory>), PrivateFsError> {
    let mut components = profile.components();
    let drive = match components.next() {
        Some(Component::Prefix(prefix)) => match prefix.kind() {
            Prefix::Disk(drive) | Prefix::VerbatimDisk(drive) => drive,
            _ => return Err(PrivateFsError::Unsafe),
        },
        _ => return Err(PrivateFsError::Unsafe),
    };
    if components.next() != Some(Component::RootDir) {
        return Err(PrivateFsError::Unsafe);
    }
    let names = components
        .map(|component| match component {
            Component::Normal(name) => name.to_str().ok_or(PrivateFsError::Unsafe),
            _ => Err(PrivateFsError::Unsafe),
        })
        .collect::<Result<Vec<_>, _>>()?;
    if names.is_empty() {
        return Err(PrivateFsError::Unsafe);
    }
    let mut directories = vec![held(open_drive(drive)?, None, false)?];
    for (index, name) in names.iter().enumerate() {
        let parent = &directories.last().ok_or(PrivateFsError::Unsafe)?.file;
        let access = native::READ
            | if index + 1 == names.len() {
                native::WRITE
            } else {
                0
            };
        let file = native::open(parent, name, Some(true), access, 3, native::OPEN, None)?;
        native::exact_name(&file, name)?;
        directories.push(held(file, Some((*name).to_owned()), false)?);
    }
    Ok((drive, directories))
}

fn append_anchor(
    directories: &mut Vec<HeldDirectory>,
    name: &str,
    mutated: &mut bool,
    fresh: bool,
    existing_only: bool,
) -> Result<(), PrivateFsError> {
    // The top container holds only closed application directories. Payload
    // confidentiality begins at the protected application/session roots.
    // Its identity and complete ACL snapshot are still pinned and rechecked.
    let private_root = name != ANCHOR;
    let parent = &directories.last().ok_or(PrivateFsError::Unsafe)?.file;
    let sd = security::native_storage::descriptor()?;
    let (file, created) = match native::open(
        parent,
        name,
        Some(true),
        native::READ | native::WRITE,
        3,
        if existing_only {
            native::OPEN
        } else {
            native::CREATE
        },
        if existing_only { None } else { Some(sd.0) },
    ) {
        Ok(file) => {
            if !existing_only {
                *mutated = true;
            }
            (file, !existing_only)
        }
        Err(PrivateFsError::AlreadyExists) if fresh => return Err(PrivateFsError::AlreadyExists),
        Err(PrivateFsError::AlreadyExists) => (
            native::open(
                parent,
                name,
                Some(true),
                native::READ | native::WRITE,
                3,
                native::OPEN,
                None,
            )?,
            false,
        ),
        Err(error) => return Err(error),
    };
    let verified = (|| {
        native::exact_name(&file, name)?;
        if private_root {
            security::native_storage::verify(&file)?;
        } else {
            security::ancestor(&file)?;
        }
        if created {
            sync_directory(parent)?;
        }
        Ok(())
    })();
    if created && verified.is_err() {
        return Err(PrivateFsError::SettlementUnknown);
    }
    verified?;
    directories.push(held(file, Some(name.to_owned()), private_root)?);
    Ok(())
}

fn verify_lock(file: &File) -> Result<(), PrivateFsError> {
    identity(file, Some(false))?;
    native::exact_name(file, LEASE)?;
    if security::mode(file)? || file.metadata().map_err(|_| PrivateFsError::Unsafe)?.len() != 0 {
        return Err(PrivateFsError::Unsafe);
    }
    Ok(())
}

#[cfg(test)]
mod tests;
