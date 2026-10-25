//! Versioned connection secrets stored exclusively in Windows generic credentials.
use super::KeychainError;
use sha2::{Digest, Sha256};
use zephium_credentials::VaultError;
use zeroize::Zeroizing;

const CHUNK: usize = 2560;
const MAX_SECRET: usize = 64 * 1024;
const MAGIC: &[u8; 8] = b"ZPMCP001";

trait Store {
    fn read(&self, target: &str) -> Result<Zeroizing<Vec<u8>>, VaultError>;
    fn write(&self, target: &str, bytes: &[u8]) -> Result<(), VaultError>;
    fn delete(&self, target: &str) -> Result<(), VaultError>;
    fn list(&self, prefix: &str) -> Result<Vec<String>, VaultError>;
}
struct Native;
impl Store for Native {
    fn read(&self, target: &str) -> Result<Zeroizing<Vec<u8>>, VaultError> {
        zephium_credentials::read(target)
    }
    fn write(&self, target: &str, bytes: &[u8]) -> Result<(), VaultError> {
        zephium_credentials::write(target, bytes)
    }
    fn delete(&self, target: &str) -> Result<(), VaultError> {
        zephium_credentials::delete(target)
    }
    fn list(&self, prefix: &str) -> Result<Vec<String>, VaultError> {
        zephium_credentials::list(prefix)
    }
}

fn map(error: VaultError) -> KeychainError {
    match error {
        VaultError::Missing => KeychainError::Missing,
        _ => KeychainError::Inaccessible,
    }
}

fn prefix(service: &str, account: &str) -> String {
    let encoded: String = account.bytes().map(|byte| format!("{byte:02x}")).collect();
    format!("{service}/{encoded}/")
}

struct Manifest {
    generation: String,
    size: usize,
    digest: [u8; 32],
}
impl Manifest {
    fn parse(bytes: &[u8]) -> Result<Self, VaultError> {
        if bytes.len() != 70 || &bytes[..8] != MAGIC {
            return Err(VaultError::Invalid);
        }
        let generation = std::str::from_utf8(&bytes[8..34]).map_err(|_| VaultError::Invalid)?;
        if generation.parse::<ulid::Ulid>().is_err()
            || generation != generation.to_ascii_uppercase()
        {
            return Err(VaultError::Invalid);
        }
        let size =
            u32::from_le_bytes(bytes[34..38].try_into().map_err(|_| VaultError::Invalid)?) as usize;
        if size > MAX_SECRET {
            return Err(VaultError::Invalid);
        }
        Ok(Self {
            generation: generation.to_owned(),
            size,
            digest: bytes[38..70].try_into().map_err(|_| VaultError::Invalid)?,
        })
    }
    fn encode(&self) -> Vec<u8> {
        let mut bytes = MAGIC.to_vec();
        bytes.extend_from_slice(self.generation.as_bytes());
        bytes.extend_from_slice(&(self.size as u32).to_le_bytes());
        bytes.extend_from_slice(&self.digest);
        bytes
    }
}

fn read_from(store: &impl Store, prefix: &str) -> Result<Zeroizing<Vec<u8>>, VaultError> {
    let manifest = Manifest::parse(&store.read(&format!("{prefix}manifest"))?)?;
    let targets = store.list(&format!("{prefix}{}", manifest.generation))?;
    if targets.len() != manifest.size.div_ceil(CHUNK)
        || targets.iter().any(|target| {
            !(0..manifest.size.div_ceil(CHUNK))
                .any(|index| target == &format!("{prefix}{}{index:03}", manifest.generation))
        })
    {
        return Err(VaultError::Invalid);
    }
    let mut result = Zeroizing::new(Vec::new());
    result
        .try_reserve_exact(manifest.size)
        .map_err(|_| VaultError::Capacity)?;
    for index in 0..manifest.size.div_ceil(CHUNK) {
        let chunk = store.read(&format!("{prefix}{}{index:03}", manifest.generation))?;
        let expected = (manifest.size - index * CHUNK).min(CHUNK);
        if chunk.len() != expected {
            return Err(VaultError::Invalid);
        }
        result.extend_from_slice(&chunk);
    }
    let digest: [u8; 32] = Sha256::digest(&*result).into();
    if result.len() != manifest.size || digest != manifest.digest {
        return Err(VaultError::Invalid);
    }
    Ok(result)
}

fn clean(store: &impl Store, prefix: &str, keep: Option<&Manifest>) -> Result<(), VaultError> {
    for target in store.list(prefix)? {
        let retained = keep.is_some_and(|manifest| {
            target == format!("{prefix}manifest")
                || (0..manifest.size.div_ceil(CHUNK))
                    .any(|index| target == format!("{prefix}{}{index:03}", manifest.generation))
        });
        if !retained {
            store.delete(&target)?;
        }
    }
    Ok(())
}

fn write_to(store: &impl Store, prefix: &str, bytes: &[u8]) -> Result<(), VaultError> {
    if bytes.len() > MAX_SECRET {
        return Err(VaultError::Invalid);
    }
    let previous = match store.read(&format!("{prefix}manifest")) {
        Ok(bytes) => Some(Manifest::parse(&bytes)?),
        Err(VaultError::Missing) => None,
        Err(error) => return Err(error),
    };
    // Retire incomplete generations left by an interrupted earlier write.
    clean(store, prefix, previous.as_ref())?;
    let manifest = Manifest {
        generation: ulid::Ulid::new().to_string(),
        size: bytes.len(),
        digest: Sha256::digest(bytes).into(),
    };
    let mut written: Vec<String> = Vec::new();
    for (index, chunk) in bytes.chunks(CHUNK).enumerate() {
        let target = format!("{prefix}{}{index:03}", manifest.generation);
        if let Err(error) = store.write(&target, chunk) {
            for target in written {
                let _ = store.delete(&target);
            }
            return Err(error);
        }
        written.push(target);
    }
    // Publishing the manifest last keeps an earlier complete secret readable on failure.
    if let Err(error) = store.write(&format!("{prefix}manifest"), &manifest.encode()) {
        for target in written {
            let _ = store.delete(&target);
        }
        return Err(error);
    }
    clean(store, prefix, Some(&manifest))
}

pub(super) fn read(service: &str, account: &str) -> Result<String, KeychainError> {
    let _turn = zephium_credentials::turn().map_err(map)?;
    let prefix = prefix(service, account);
    let loaded = match read_from(&Native, &prefix) {
        Err(VaultError::Inaccessible) => {
            std::thread::sleep(std::time::Duration::from_millis(40));
            read_from(&Native, &prefix)
        }
        loaded => loaded,
    };
    let bytes = loaded.map_err(map)?;
    std::str::from_utf8(&bytes)
        .map(str::to_owned)
        .map_err(|_| KeychainError::Inaccessible)
}
pub(super) fn write(service: &str, account: &str, secret: &str) -> Result<(), KeychainError> {
    let _turn = zephium_credentials::turn().map_err(map)?;
    write_to(&Native, &prefix(service, account), secret.as_bytes()).map_err(map)
}
pub(super) fn delete(service: &str, account: &str) -> Result<(), KeychainError> {
    let _turn = zephium_credentials::turn().map_err(map)?;
    let prefix = prefix(service, account);
    // Withdraw publication before retiring chunks, so interrupted deletion fails closed.
    Native.delete(&format!("{prefix}manifest")).map_err(map)?;
    clean(&Native, &prefix, None).map_err(map)
}
pub(super) fn delete_server(service: &str) -> Result<(), KeychainError> {
    let _turn = zephium_credentials::turn().map_err(map)?;
    let prefix = format!("{service}/");
    let targets = Native.list(&prefix).map_err(map)?;
    for target in targets
        .iter()
        .filter(|target| target.ends_with("/manifest"))
    {
        Native.delete(target).map_err(map)?;
    }
    for target in targets {
        Native.delete(&target).map_err(map)?;
    }
    Ok(())
}
pub(super) fn present(service: &str, account: &str) -> bool {
    let Ok(_turn) = zephium_credentials::turn() else {
        return false;
    };
    zephium_credentials::present(&format!("{}manifest", prefix(service, account))).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        cell::{Cell, RefCell},
        collections::BTreeMap,
    };
    #[derive(Default)]
    struct Memory {
        items: RefCell<BTreeMap<String, Vec<u8>>>,
        writes: Cell<usize>,
        fail: Cell<Option<usize>>,
    }
    impl Store for Memory {
        fn read(&self, target: &str) -> Result<Zeroizing<Vec<u8>>, VaultError> {
            self.items
                .borrow()
                .get(target)
                .cloned()
                .map(Zeroizing::new)
                .ok_or(VaultError::Missing)
        }
        fn write(&self, target: &str, bytes: &[u8]) -> Result<(), VaultError> {
            let count = self.writes.get() + 1;
            self.writes.set(count);
            if self.fail.get() == Some(count) {
                return Err(VaultError::Inaccessible);
            }
            self.items.borrow_mut().insert(target.into(), bytes.into());
            Ok(())
        }
        fn delete(&self, target: &str) -> Result<(), VaultError> {
            self.items.borrow_mut().remove(target);
            Ok(())
        }
        fn list(&self, prefix: &str) -> Result<Vec<String>, VaultError> {
            Ok(self
                .items
                .borrow()
                .keys()
                .filter(|key| key.starts_with(prefix))
                .cloned()
                .collect())
        }
    }
    #[test]
    fn complete_large_secret_replaces_and_retires_old_chunks() {
        let store = Memory::default();
        let namespace = prefix("app.quasar.connection.profile.server", "oauth");
        let bytes = vec![b'x'; MAX_SECRET];
        write_to(&store, &namespace, &bytes).unwrap();
        assert_eq!(&*read_from(&store, &namespace).unwrap(), &bytes);
        write_to(&store, &namespace, b"new").unwrap();
        assert_eq!(&*read_from(&store, &namespace).unwrap(), b"new");
        assert_eq!(store.items.borrow().len(), 2);
    }
    #[test]
    fn each_failed_chunk_or_manifest_keeps_previous_secret_and_cleans_partial_generation() {
        for offset in 1..=4 {
            let store = Memory::default();
            let namespace = prefix("app.quasar.connection.p.s", "oauth");
            write_to(&store, &namespace, b"previous").unwrap();
            store.fail.set(Some(store.writes.get() + offset));
            assert_eq!(
                write_to(&store, &namespace, &vec![b'x'; CHUNK * 3]),
                Err(VaultError::Inaccessible)
            );
            assert_eq!(&*read_from(&store, &namespace).unwrap(), b"previous");
            assert_eq!(store.items.borrow().len(), 2);
        }
    }
    #[test]
    fn corrupt_missing_and_wrong_length_chunks_fail_closed() {
        let store = Memory::default();
        let namespace = prefix("app.quasar.connection.p.s", "oauth");
        write_to(&store, &namespace, &vec![b'x'; CHUNK + 1]).unwrap();
        let target = store
            .items
            .borrow()
            .keys()
            .find(|key| !key.ends_with("manifest"))
            .unwrap()
            .clone();
        store.items.borrow_mut().get_mut(&target).unwrap()[0] = b'z';
        assert_eq!(
            read_from(&store, &namespace).unwrap_err(),
            VaultError::Invalid
        );
        store.items.borrow_mut().get_mut(&target).unwrap().push(0);
        assert_eq!(
            read_from(&store, &namespace).unwrap_err(),
            VaultError::Invalid
        );
        store.items.borrow_mut().remove(&target);
        assert_eq!(
            read_from(&store, &namespace).unwrap_err(),
            VaultError::Invalid
        );
    }
    #[test]
    fn accounts_and_server_prefixes_do_not_overlap() {
        assert_ne!(
            prefix("app.quasar.connection.p.s", "env.Key"),
            prefix("app.quasar.connection.p.s", "env.key")
        );
        assert!(!prefix("app.quasar.connection.p.server2", "oauth")
            .starts_with("app.quasar.connection.p.server/"));
        assert_eq!(
            write_to(
                &Memory::default(),
                "app.quasar.test/",
                &vec![0; MAX_SECRET + 1]
            ),
            Err(VaultError::Invalid)
        );
    }
}
