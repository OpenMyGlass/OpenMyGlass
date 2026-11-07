//! What the companion remembers about an app, across uninstalls.
//!
//! The first install of an id trusts its key, with the owner's consent. From then on this record
//! holds that key, the repository, the highest signed state seen and the highest version
//! installed, until the owner explicitly forgets the app. Uninstalling does not forget: otherwise
//! whoever stole a repository could wait for an uninstall, or ask for one.
//!
//! Memory is also the only defence against a host serving an older signed state, and only for
//! what this companion has already seen. A companion that never saw the newer state cannot tell.

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use semver::Version;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::download::{sync_parent, VerifiedPackage};
use super::manifest::{decode_hex, parse_version, to_hex, AppEntry, Sha256Digest};
use super::select::InstallTicket;
use super::url::HttpsUrl;

/// A disagreement between what is remembered and what a repository now says. Nothing is offered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum TrustProblem {
    /// This manifest is not the repository the app came from. The same id elsewhere is another
    /// claim, even under the same key. To switch, the owner forgets the app first.
    #[error("this app was installed from another repository")]
    OtherSource,
    /// The publisher key changed. This is what a stolen repository looks like.
    #[error("the publisher key of this app changed")]
    PublisherChanged,
    /// A state older than one already seen: a host hiding a fix or restoring a withdrawn release.
    #[error("the repository is serving an older state of this app")]
    StateRolledBack,
    /// Two different states with one sequence number, both signed: the key holder did this, so the
    /// author made a mistake or the key is stolen.
    #[error("the repository has two different states with one sequence number")]
    StateForked,
    /// A version already installed is now listed with a different hash. Also needs the key.
    #[error("a published version of this app changed")]
    ReleaseChanged,
}

/// What is remembered about one app id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustRecord {
    /// The app id.
    pub id: String,
    /// The key pinned at first install.
    pub publisher_key: [u8; 32],
    /// The manifest URL of the first install.
    pub repository: HttpsUrl,
    /// The highest state seen.
    pub sequence: u64,
    /// That state's digest.
    pub state_digest: Sha256Digest,
    /// Every version seen withdrawn, sorted. Stays withdrawn whatever a later file says.
    pub withdrawn: Vec<Version>,
    /// The highest version installed and its hash: the floor for later installs.
    pub highest: Option<(Version, Sha256Digest)>,
}

impl TrustRecord {
    /// Compares `app` with this record. Does not change anything.
    pub fn check(&self, app: &AppEntry) -> Result<(), TrustProblem> {
        if self.id != app.id || self.repository != app.repository {
            return Err(TrustProblem::OtherSource);
        }
        if self.publisher_key != app.publisher_key {
            return Err(TrustProblem::PublisherChanged);
        }
        if app.sequence < self.sequence {
            return Err(TrustProblem::StateRolledBack);
        }
        if app.sequence == self.sequence && app.state_digest() != self.state_digest {
            return Err(TrustProblem::StateForked);
        }
        if let Some((version, sha256)) = &self.highest {
            let changed = app
                .releases
                .iter()
                .any(|r| r.version == *version && r.sha256 != *sha256);
            if changed {
                return Err(TrustProblem::ReleaseChanged);
            }
        }
        Ok(())
    }

    fn absorb(&mut self, app: &AppEntry) {
        self.sequence = app.sequence;
        self.state_digest = app.state_digest();
        for version in &app.withdrawn {
            if !self.withdrawn.contains(version) {
                self.withdrawn.push(version.clone());
            }
        }
        self.withdrawn.sort_unstable();
    }
}

/// Why the memory cannot be used.
#[derive(Debug, Error)]
pub enum TrustError {
    /// The file could not be read or written.
    #[error("i/o: {0}")]
    Io(#[from] io::Error),
    /// The file exists and is damaged. It is never treated as empty: that would drop every pin.
    #[error("the record of trusted apps is damaged")]
    Corrupt,
    /// The manifest disagrees with the record.
    #[error("{0}")]
    Problem(#[from] TrustProblem),
    /// The package is not the one the ticket is for.
    #[error("the package does not match the install ticket")]
    PackageMismatch,
}

/// The records, in one JSON file written atomically.
#[derive(Debug)]
pub struct TrustStore {
    path: PathBuf,
    records: BTreeMap<String, TrustRecord>,
}

impl TrustStore {
    /// Reads `path`, private to the app. A missing file is an empty memory, a damaged one an error.
    pub fn open(path: impl Into<PathBuf>) -> Result<Self, TrustError> {
        let path = path.into();
        let records = match fs::read(&path) {
            Ok(bytes) => decode(&bytes).ok_or(TrustError::Corrupt)?,
            Err(e) if e.kind() == io::ErrorKind::NotFound => BTreeMap::new(),
            Err(e) => return Err(e.into()),
        };
        Ok(Self { path, records })
    }

    /// What is remembered about this id. Pass it to [`super::plan_install`].
    pub fn get(&self, id: &str) -> Option<&TrustRecord> {
        self.records.get(id)
    }

    /// Takes in a freshly read app, so its newest state and withdrawals are remembered even if
    /// nothing is installed from it. Call for every known app after every manifest read. An
    /// unknown id is ignored: an app is only remembered once the owner installs it.
    pub fn observe(&mut self, app: &AppEntry) -> Result<(), TrustError> {
        let Some(record) = self.records.get_mut(&app.id) else {
            return Ok(());
        };
        record.check(app)?;
        if record.sequence != app.sequence {
            record.absorb(app);
            self.save()?;
        }
        Ok(())
    }

    /// Remembers a finished install, durably. Call before telling the owner it is done.
    pub fn record_install(
        &mut self,
        ticket: &InstallTicket<'_>,
        package: &VerifiedPackage,
    ) -> Result<(), TrustError> {
        let (app, release) = (ticket.app(), ticket.release());
        if package.app_id() != app.id
            || *package.version() != release.version
            || *package.sha256() != release.sha256
        {
            return Err(TrustError::PackageMismatch);
        }
        match self.records.get_mut(&app.id) {
            Some(record) => {
                record.check(app)?;
                record.absorb(app);
            }
            None => {
                self.records.insert(
                    app.id.clone(),
                    TrustRecord {
                        id: app.id.clone(),
                        publisher_key: app.publisher_key,
                        repository: app.repository.clone(),
                        sequence: app.sequence,
                        state_digest: app.state_digest(),
                        withdrawn: app.withdrawn.clone(),
                        highest: None,
                    },
                );
            }
        }
        if let Some(record) = self.records.get_mut(&app.id) {
            let higher = record
                .highest
                .as_ref()
                .is_none_or(|(version, _)| release.version > *version);
            if higher {
                record.highest = Some((release.version.clone(), release.sha256));
            }
        }
        self.save()
    }

    /// Drops everything about this id. The next install of it is a first use, under any key, from
    /// any repository. Only for an explicit choice of the owner, with the warning: this is how
    /// an author's lost key or moved repository is handled, and it is also what an attacker
    /// would ask for.
    pub fn forget(&mut self, id: &str) -> Result<(), TrustError> {
        if self.records.remove(id).is_some() {
            self.save()?;
        }
        Ok(())
    }

    fn save(&self) -> Result<(), TrustError> {
        let bytes = encode(&self.records);
        let temporary = temporary_path(&self.path);
        let mut file = File::create(&temporary)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, &self.path)?;
        sync_parent(&self.path)?;
        Ok(())
    }
}

fn temporary_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".tmp");
    PathBuf::from(name)
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawStore {
    schema: u32,
    apps: Vec<RawRecord>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRecord {
    id: String,
    publisher_key: String,
    repository: String,
    sequence: u64,
    state_digest: String,
    withdrawn: Vec<String>,
    highest_version: Option<String>,
    highest_sha256: Option<String>,
}

fn encode(records: &BTreeMap<String, TrustRecord>) -> Vec<u8> {
    let raw = RawStore {
        schema: 1,
        apps: records
            .values()
            .map(|r| RawRecord {
                id: r.id.clone(),
                publisher_key: to_hex(&r.publisher_key),
                repository: r.repository.to_string(),
                sequence: r.sequence,
                state_digest: to_hex(&r.state_digest),
                withdrawn: r.withdrawn.iter().map(Version::to_string).collect(),
                highest_version: r.highest.as_ref().map(|(v, _)| v.to_string()),
                highest_sha256: r.highest.as_ref().map(|(_, sha)| to_hex(sha)),
            })
            .collect(),
    };
    // Strings and integers only, so this cannot fail.
    serde_json::to_vec_pretty(&raw).unwrap_or_default()
}

fn decode(bytes: &[u8]) -> Option<BTreeMap<String, TrustRecord>> {
    let raw: RawStore = serde_json::from_slice(bytes).ok()?;
    if raw.schema != 1 {
        return None;
    }
    let mut records = BTreeMap::new();
    for r in raw.apps {
        let withdrawn = r
            .withdrawn
            .iter()
            .map(|v| parse_version(v))
            .collect::<Option<Vec<Version>>>()?;
        let highest = match (r.highest_version, r.highest_sha256) {
            (Some(version), Some(sha256)) => Some((parse_version(&version)?, decode_hex(&sha256)?)),
            (None, None) => None,
            _ => return None,
        };
        let record = TrustRecord {
            id: r.id.clone(),
            publisher_key: decode_hex(&r.publisher_key)?,
            repository: HttpsUrl::parse(&r.repository).ok()?,
            sequence: r.sequence,
            state_digest: decode_hex(&r.state_digest)?,
            withdrawn,
            highest,
        };
        if records.insert(r.id, record).is_some() {
            return None;
        }
    }
    Some(records)
}
