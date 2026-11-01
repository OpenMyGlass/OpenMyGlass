//! Persistent companion/glass state: our identity and the single bonded peer.
//!
//! This is what makes pairing **persistent and one-time**: once a peer is bonded it is
//! remembered across reboots, re-used on every reconnect, and any other peer is rejected
//! until [`Store::clear_paired`] (unpair).
//!
//! NOTE (from the persistent-pairing decision): on Model B (RAM-only Linux) this file
//! must live on persistent storage (`/var/lib/glassd`), or the Glass forgets its
//! companion on reboot. On a phone/PC companion it is just a file in the config dir.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{identity::Identity, now_ms, CoreError};

/// The bonded peer. Its public key is the trust anchor checked on every reconnect.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairingRecord {
    /// Base64 of the peer's ed25519 public key.
    pub peer_pub: String,
    /// Friendly name captured at pairing.
    pub peer_name: String,
    /// When the bond was established, Unix ms.
    pub bonded_at_ms: u64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct StateFile {
    identity_secret: String,
    paired_peer: Option<PairingRecord>,
}

/// On-disk companion/glass state with an in-memory view.
pub struct Store {
    path: PathBuf,
    identity: Identity,
    paired: Option<PairingRecord>,
}

impl Store {
    /// Load the store from `path`, or create a fresh identity and persist it if the
    /// file does not exist.
    pub fn load_or_create(path: impl AsRef<Path>) -> Result<Store, CoreError> {
        let path = path.as_ref().to_path_buf();
        if path.exists() {
            let data = std::fs::read_to_string(&path)?;
            let sf: StateFile = serde_json::from_str(&data)
                .map_err(|e| CoreError::Store(format!("parse {}: {e}", path.display())))?;
            let identity = Identity::from_secret_b64(&sf.identity_secret)?;
            Ok(Store {
                path,
                identity,
                paired: sf.paired_peer,
            })
        } else {
            if let Some(parent) = path.parent() {
                if !parent.as_os_str().is_empty() {
                    std::fs::create_dir_all(parent)?;
                }
            }
            let store = Store {
                path,
                identity: Identity::generate(),
                paired: None,
            };
            store.save()?;
            Ok(store)
        }
    }

    /// Our long-term identity.
    pub fn identity(&self) -> &Identity {
        &self.identity
    }

    /// The bonded peer, if any.
    pub fn paired(&self) -> Option<&PairingRecord> {
        self.paired.as_ref()
    }

    /// Record a newly bonded peer and persist.
    pub fn set_paired(&mut self, peer_pub: String, peer_name: String) -> Result<(), CoreError> {
        self.paired = Some(PairingRecord {
            peer_pub,
            peer_name,
            bonded_at_ms: now_ms(),
        });
        self.save()
    }

    /// Forget the bonded peer (unpair) and persist.
    pub fn clear_paired(&mut self) -> Result<(), CoreError> {
        self.paired = None;
        self.save()
    }

    fn save(&self) -> Result<(), CoreError> {
        let sf = StateFile {
            identity_secret: self.identity.secret_b64(),
            paired_peer: self.paired.clone(),
        };
        let json = serde_json::to_string_pretty(&sf)
            .map_err(|e| CoreError::Store(format!("serialize: {e}")))?;
        // Write to a temp file then rename, so a crash never truncates the identity.
        let tmp = self.path.with_extension("tmp");
        std::fs::write(&tmp, json)?;
        std::fs::rename(&tmp, &self.path)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persists_identity_and_pairing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub").join("state.json");

        let pub_a;
        {
            let mut s = Store::load_or_create(&path).unwrap();
            pub_a = s.identity().public_b64();
            assert!(s.paired().is_none());
            s.set_paired("peerpub".into(), "Glass".into()).unwrap();
        }
        // Reload: same identity, remembered peer.
        {
            let s = Store::load_or_create(&path).unwrap();
            assert_eq!(s.identity().public_b64(), pub_a);
            assert_eq!(s.paired().unwrap().peer_pub, "peerpub");
            assert_eq!(s.paired().unwrap().peer_name, "Glass");
        }
    }

    #[test]
    fn unpair_clears() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        let mut s = Store::load_or_create(&path).unwrap();
        s.set_paired("p".into(), "n".into()).unwrap();
        s.clear_paired().unwrap();
        let s2 = Store::load_or_create(&path).unwrap();
        assert!(s2.paired().is_none());
    }
}
