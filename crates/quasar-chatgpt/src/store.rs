//! Atomic encrypted session records, with a small encryption key in the OS vault.
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use ring::aead;
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::{random, Error, State};

const MAX_PLAIN: usize = 256 * 1024;
const MAX_FILE: usize = 512 * 1024;
const AAD: &[u8] = b"Quasar ChatGPT session v1";

/// A small native encryption key; OAuth tokens are never stored as vault blobs.
pub trait EncryptionKey: Send + Sync {
    fn load(&self, service: &str, create: bool) -> Result<Zeroizing<Vec<u8>>, Error>;
}

/// Keychain on macOS and Credential Manager on Windows.
pub struct SystemKey;

impl EncryptionKey for SystemKey {
    fn load(&self, service: &str, create: bool) -> Result<Zeroizing<Vec<u8>>, Error> {
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        {
            let _turn = zephium_credentials::turn().map_err(|_| Error::Storage)?;
            match zephium_credentials::read(service) {
                Ok(bytes) if bytes.len() == 32 => Ok(bytes),
                Ok(_) => Err(Error::Storage),
                Err(zephium_credentials::VaultError::Missing) if create => {
                    let bytes = Zeroizing::new(random::<32>()?.to_vec());
                    zephium_credentials::write(service, &bytes).map_err(|_| Error::Storage)?;
                    Ok(bytes)
                }
                Err(_) => Err(Error::Storage),
            }
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            let _ = (service, create);
            Err(Error::Unsupported)
        }
    }
}

/// Private application-owned storage. It must live under Quasar's data directory.
#[derive(Clone)]
pub struct Store {
    root: PathBuf,
    service: String,
    key: Arc<dyn EncryptionKey>,
}

#[derive(Serialize, Deserialize)]
struct Envelope {
    version: u8,
    nonce: String,
    ciphertext: String,
}

pub(crate) struct Lease {
    file: File,
}

impl Drop for Lease {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

impl Store {
    pub fn new(root: PathBuf, key: Arc<dyn EncryptionKey>) -> Result<Self, Error> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
            let mut builder = fs::DirBuilder::new();
            builder.recursive(true).mode(0o700);
            match builder.create(&root) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(_) => return Err(Error::Storage),
            }
            let meta = fs::symlink_metadata(&root).map_err(|_| Error::Storage)?;
            if meta.file_type().is_symlink()
                || !meta.is_dir()
                || meta.permissions().mode() & 0o077 != 0
            {
                return Err(Error::Storage);
            }
        }
        #[cfg(not(unix))]
        {
            fs::create_dir_all(&root).map_err(|_| Error::Storage)?;
            let meta = fs::symlink_metadata(&root).map_err(|_| Error::Storage)?;
            if meta.file_type().is_symlink() || !meta.is_dir() {
                return Err(Error::Storage);
            }
        }
        let root = root.canonicalize().map_err(|_| Error::Storage)?;
        let digest =
            ring::digest::digest(&ring::digest::SHA256, root.as_os_str().as_encoded_bytes());
        let service = format!(
            "app.quasar.chatgpt.{}",
            URL_SAFE_NO_PAD.encode(digest.as_ref())
        );
        Ok(Self { root, service, key })
    }

    pub(crate) fn lease(&self) -> Result<Lease, Error> {
        let path = self.root.join("session.lock");
        let file = private_open(&path, true)?;
        let start = Instant::now();
        loop {
            match file.try_lock() {
                Ok(()) => return Ok(Lease { file }),
                Err(std::fs::TryLockError::WouldBlock)
                    if start.elapsed() < Duration::from_secs(5) =>
                {
                    std::thread::sleep(Duration::from_millis(25));
                }
                Err(_) => return Err(Error::Busy),
            }
        }
    }

    pub(crate) fn read(&self, _lease: &Lease) -> Result<State, Error> {
        let path = self.root.join("session.json");
        if !path.try_exists().map_err(|_| Error::Storage)? {
            return State::fresh();
        }
        let file = private_open(&path, false)?;
        let mut bytes = Vec::new();
        file.take((MAX_FILE + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|_| Error::Storage)?;
        if bytes.len() > MAX_FILE {
            return Err(Error::Storage);
        }
        let envelope: Envelope = serde_json::from_slice(&bytes).map_err(|_| Error::Storage)?;
        if envelope.version != 1 {
            return Err(Error::Storage);
        }
        let nonce: [u8; 12] = URL_SAFE_NO_PAD
            .decode(envelope.nonce)
            .map_err(|_| Error::Storage)?
            .try_into()
            .map_err(|_| Error::Storage)?;
        let mut cipher = Zeroizing::new(
            URL_SAFE_NO_PAD
                .decode(envelope.ciphertext)
                .map_err(|_| Error::Storage)?,
        );
        let key = self.cipher(false)?;
        let plain = key
            .open_in_place(
                aead::Nonce::assume_unique_for_key(nonce),
                aead::Aad::from(AAD),
                &mut cipher,
            )
            .map_err(|_| Error::Storage)?;
        if plain.len() > MAX_PLAIN {
            return Err(Error::Storage);
        }
        let state: State = serde_json::from_slice(plain).map_err(|_| Error::Storage)?;
        if state.version != 1
            || state.profiles.len() > crate::MAX_PROFILES
            || !state.host_id.starts_with("urn:uuid:")
            || state.host_id.len() != 45
            || state
                .active
                .as_ref()
                .is_some_and(|id| !state.profiles.iter().any(|p| &p.id == id))
        {
            return Err(Error::Storage);
        }
        Ok(state)
    }

    pub(crate) fn write(&self, _lease: &Lease, state: &State) -> Result<(), Error> {
        let mut plain = Zeroizing::new(serde_json::to_vec(state).map_err(|_| Error::Storage)?);
        if plain.len() > MAX_PLAIN {
            return Err(Error::Storage);
        }
        // Never silently generate a replacement key for an existing session file.
        let existing = self
            .root
            .join("session.json")
            .try_exists()
            .map_err(|_| Error::Storage)?;
        let key = self.cipher(!existing)?;
        let nonce = random::<12>()?;
        key.seal_in_place_append_tag(
            aead::Nonce::assume_unique_for_key(nonce),
            aead::Aad::from(AAD),
            &mut *plain,
        )
        .map_err(|_| Error::Storage)?;
        let bytes = serde_json::to_vec(&Envelope {
            version: 1,
            nonce: URL_SAFE_NO_PAD.encode(nonce),
            ciphertext: URL_SAFE_NO_PAD.encode(&*plain),
        })
        .map_err(|_| Error::Storage)?;
        let mut temp = tempfile::NamedTempFile::new_in(&self.root).map_err(|_| Error::Storage)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            temp.as_file()
                .set_permissions(fs::Permissions::from_mode(0o600))
                .map_err(|_| Error::Storage)?;
        }
        temp.write_all(&bytes).map_err(|_| Error::Storage)?;
        temp.as_file().sync_all().map_err(|_| Error::Storage)?;
        temp.persist(self.root.join("session.json"))
            .map_err(|_| Error::Storage)?;
        #[cfg(unix)]
        File::open(&self.root)
            .and_then(|f| f.sync_all())
            .map_err(|_| Error::Storage)?;
        Ok(())
    }

    fn cipher(&self, create: bool) -> Result<aead::LessSafeKey, Error> {
        let bytes = self.key.load(&self.service, create)?;
        let key = aead::UnboundKey::new(&aead::AES_256_GCM, &bytes).map_err(|_| Error::Storage)?;
        Ok(aead::LessSafeKey::new(key))
    }
}

fn private_open(path: &Path, create: bool) -> Result<File, Error> {
    if let Ok(meta) = fs::symlink_metadata(path) {
        if meta.file_type().is_symlink() || !meta.is_file() {
            return Err(Error::Storage);
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::{MetadataExt, PermissionsExt};
            if meta.nlink() != 1 || meta.permissions().mode() & 0o077 != 0 {
                return Err(Error::Storage);
            }
        }
    }
    let mut options = OpenOptions::new();
    options.read(true);
    if create {
        options.write(true).create(true);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path).map_err(|_| Error::Storage)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MemoryKey;
    impl EncryptionKey for MemoryKey {
        fn load(&self, _: &str, _: bool) -> Result<Zeroizing<Vec<u8>>, Error> {
            Ok(Zeroizing::new(vec![7; 32]))
        }
    }

    #[test]
    fn session_is_encrypted_and_atomic_and_tampering_fails_closed() {
        let directory = tempfile::tempdir().expect("temp");
        let store =
            Store::new(directory.path().join("chatgpt"), Arc::new(MemoryKey)).expect("store");
        let lease = store.lease().expect("lease");
        let state = State::fresh().expect("state");
        store.write(&lease, &state).expect("save");
        let path = store.root.join("session.json");
        let bytes = fs::read(&path).expect("bytes");
        assert!(!String::from_utf8_lossy(&bytes).contains(&state.host_id));
        assert_eq!(store.read(&lease).expect("load").host_id, state.host_id);
        let mut envelope: Envelope = serde_json::from_slice(&bytes).expect("envelope");
        let mut ciphertext = URL_SAFE_NO_PAD
            .decode(&envelope.ciphertext)
            .expect("cipher");
        ciphertext[0] ^= 1;
        envelope.ciphertext = URL_SAFE_NO_PAD.encode(ciphertext);
        fs::write(path, serde_json::to_vec(&envelope).expect("json")).expect("tamper");
        assert!(matches!(store.read(&lease), Err(Error::Storage)));
    }

    #[cfg(unix)]
    #[test]
    fn credential_directory_and_records_are_private_and_links_are_refused() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let directory = tempfile::tempdir().expect("temp");
        let root = directory.path().join("chatgpt");
        let store = Store::new(root.clone(), Arc::new(MemoryKey)).expect("store");
        let lease = store.lease().expect("lease");
        store
            .write(&lease, &State::fresh().expect("state"))
            .expect("save");
        assert_eq!(
            fs::metadata(&root).expect("dir").permissions().mode() & 0o777,
            0o700
        );
        let file = root.join("session.json");
        assert_eq!(
            fs::metadata(&file).expect("file").permissions().mode() & 0o777,
            0o600
        );
        fs::rename(&file, root.join("real.json")).expect("move");
        symlink(root.join("real.json"), &file).expect("link");
        assert!(matches!(store.read(&lease), Err(Error::Storage)));
    }
}
