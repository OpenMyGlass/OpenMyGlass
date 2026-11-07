//! What the companion remembers about an app, across uninstalls.
//!
//! The first install of an id trusts its key, with the owner's consent. From then on this record
//! holds that key, the repository, the highest signed state seen and the highest version
//! installed, until the owner explicitly forgets the app. Uninstalling does not forget: otherwise
//! whoever stole a repository could wait for an uninstall, or ask for one.
//!
//! Memory is also the only defence against a host serving an older signed state, and only for
//! what this companion has already seen. A companion that never saw the newer state cannot tell.
//!
//! Every change takes an exclusive lock, reads the file again, applies the change to that, writes
//! a new file and renames it into place, and only then updates what this instance holds. So two
//! instances on one file do not lose each other's changes, and a failed write changes nothing.

use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use semver::Version;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::download::VerifiedPackage;
use super::fsutil::{create_exclusive, random_part, sync_parent, FileLock};
use super::manifest::{decode_hex, parse_version, to_hex, AppEntry, Release, Sha256Digest};
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
        self.check_source(app)?;
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

    fn check_source(&self, app: &AppEntry) -> Result<(), TrustProblem> {
        if self.id != app.id || self.repository != app.repository {
            return Err(TrustProblem::OtherSource);
        }
        if self.publisher_key != app.publisher_key {
            return Err(TrustProblem::PublisherChanged);
        }
        Ok(())
    }

    /// The rules for recording one verified release, which do not need `app` to be the newest
    /// state: memory may have moved on while it downloaded.
    fn check_release(&self, app: &AppEntry, release: &Release) -> Result<(), TrustError> {
        self.check_source(app)?;
        if app.sequence == self.sequence && app.state_digest() != self.state_digest {
            return Err(TrustProblem::StateForked.into());
        }
        if self.withdrawn.contains(&release.version) {
            return Err(TrustError::Withdrawn);
        }
        if let Some((version, sha256)) = &self.highest {
            if release.version < *version {
                return Err(TrustError::BelowFloor);
            }
            if release.version == *version && release.sha256 != *sha256 {
                return Err(TrustProblem::ReleaseChanged.into());
            }
        }
        Ok(())
    }

    fn absorb(&mut self, app: &AppEntry) {
        if app.sequence > self.sequence {
            self.sequence = app.sequence;
            self.state_digest = app.state_digest();
        }
        for version in &app.withdrawn {
            if !self.withdrawn.contains(version) {
                self.withdrawn.push(version.clone());
            }
        }
        self.withdrawn.sort_unstable();
    }
}

/// What [`TrustStore::observe`] found for one app.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Observed {
    /// Nothing is remembered about this id here.
    Unknown,
    /// Known, and this state was already seen.
    Unchanged,
    /// Known, and a newer state is now remembered.
    Updated,
    /// Known, and the manifest disagrees with the record. For [`TrustProblem::OtherSource`] this
    /// is usually another repository listing the same id, which is not news; the rest are
    /// warnings for the UI.
    Mismatch(TrustProblem),
}

/// Why the memory cannot be used, or an install cannot be recorded.
#[derive(Debug, Error)]
pub enum TrustError {
    /// The file could not be read or written. Nothing was changed.
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
    /// The release was withdrawn after the install was planned.
    #[error("the release was withdrawn")]
    Withdrawn,
    /// A higher version was installed after the install was planned.
    #[error("a higher version is already installed")]
    BelowFloor,
    /// The install was planned against a record that was forgotten before it was recorded.
    #[error("the app was forgotten while it was being installed")]
    RecordMissing,
}

/// A package that passed its checks and is recorded in the [`TrustStore`]: what a transfer to the
/// Glass needs, for `AppOffer`. Only [`TrustStore::record_install`] makes one.
#[derive(Debug, PartialEq, Eq)]
pub struct RecordedPackage {
    package: VerifiedPackage,
}

impl RecordedPackage {
    /// The app.
    pub fn app_id(&self) -> &str {
        &self.package.app_id
    }

    /// The version.
    pub fn version(&self) -> &Version {
        &self.package.version
    }

    /// The hash the bytes were checked against, which names the file in the
    /// [`super::PackageStore`].
    pub fn sha256(&self) -> &Sha256Digest {
        &self.package.sha256
    }

    /// The exact size.
    pub fn size(&self) -> u64 {
        self.package.size
    }

    /// The key that signed the release.
    pub fn publisher_key(&self) -> &[u8; 32] {
        &self.package.publisher_key
    }

    /// The publisher's signature over [`statement`](Self::statement).
    pub fn signature(&self) -> &[u8; 64] {
        &self.package.signature
    }

    /// The signed bytes, so the Glass can verify the release without the manifest.
    pub fn statement(&self) -> &[u8] {
        &self.package.statement
    }
}

/// The records, in one JSON file, with a lock file beside it.
#[derive(Debug)]
pub struct TrustStore {
    path: PathBuf,
    records: BTreeMap<String, TrustRecord>,
}

impl TrustStore {
    /// Reads `path`, which should be in the app's private storage. A missing file is an empty
    /// memory, a damaged one an error.
    pub fn open(path: impl Into<PathBuf>) -> Result<Self, TrustError> {
        let path = path.into();
        let records = read_records(&path)?;
        Ok(Self { path, records })
    }

    /// What this instance last read about this id. [`super::plan_install`] uses it, and
    /// [`record_install`](Self::record_install) checks again against the file.
    pub fn get(&self, id: &str) -> Option<&TrustRecord> {
        self.records.get(id)
    }

    /// Reads the file again, for changes made by another instance.
    pub fn reload(&mut self) -> Result<(), TrustError> {
        self.records = read_records(&self.path)?;
        Ok(())
    }

    /// Takes in a freshly read app, so its newest state and withdrawals are remembered even if
    /// nothing is installed from it. Call for every app after every manifest read. Only I/O is an
    /// error: what the app says is reported, so one odd app does not stop a refresh.
    pub fn observe(&mut self, app: &AppEntry) -> Result<Observed, TrustError> {
        let mut observed = Observed::Unknown;
        self.change(|records| {
            let Some(record) = records.get_mut(&app.id) else {
                return Ok(false);
            };
            if let Err(problem) = record.check(app) {
                observed = Observed::Mismatch(problem);
                return Ok(false);
            }
            if app.sequence == record.sequence {
                observed = Observed::Unchanged;
                return Ok(false);
            }
            record.absorb(app);
            observed = Observed::Updated;
            Ok(true)
        })?;
        Ok(observed)
    }

    /// Remembers a verified download, durably, and returns what may be handed to the Glass. Call
    /// it before the transfer: the [`RecordedPackage`] it returns is the only way to the signed
    /// statement the Glass needs. Checks the release again against the file as it is now: the
    /// key, the repository, the hash, the withdrawals and the floor. It does not need the ticket's
    /// state to be the newest one.
    pub fn record_install(
        &mut self,
        ticket: &InstallTicket<'_>,
        package: VerifiedPackage,
    ) -> Result<RecordedPackage, TrustError> {
        let (app, release) = (ticket.app(), ticket.release());
        if package.app_id() != app.id
            || *package.version() != release.version
            || *package.sha256() != release.sha256
            || package.publisher_key() != &app.publisher_key
        {
            return Err(TrustError::PackageMismatch);
        }
        self.change(|records| {
            let record = match records.get_mut(&app.id) {
                Some(record) => {
                    record.check_release(app, release)?;
                    record.absorb(app);
                    record
                }
                // Planned against a record that is gone: the owner forgot the app meanwhile. Not
                // turned into a first use nobody consented to.
                None if !ticket.first_use() => return Err(TrustError::RecordMissing),
                None => {
                    let record = TrustRecord {
                        id: app.id.clone(),
                        publisher_key: app.publisher_key,
                        repository: app.repository.clone(),
                        sequence: app.sequence,
                        state_digest: app.state_digest(),
                        withdrawn: app.withdrawn.clone(),
                        highest: None,
                    };
                    record.check_release(app, release)?;
                    records.entry(app.id.clone()).or_insert(record)
                }
            };
            let higher = record
                .highest
                .as_ref()
                .is_none_or(|(version, _)| release.version > *version);
            if higher {
                record.highest = Some((release.version.clone(), release.sha256));
            }
            Ok(true)
        })?;
        Ok(RecordedPackage { package })
    }

    /// Drops everything about this id. The next install of it is a first use, under any key, from
    /// any repository. Only for an explicit choice of the owner, with the warning: this is how
    /// an author's lost key or moved repository is handled, and it is also what an attacker
    /// would ask for.
    pub fn forget(&mut self, id: &str) -> Result<(), TrustError> {
        self.change(|records| Ok(records.remove(id).is_some()))
    }

    /// Under the lock: reads the file, applies `edit` to that, and writes it if `edit` says it
    /// changed something. This instance's view is updated only after a successful write.
    fn change(
        &mut self,
        edit: impl FnOnce(&mut BTreeMap<String, TrustRecord>) -> Result<bool, TrustError>,
    ) -> Result<(), TrustError> {
        let _lock = FileLock::acquire(&sibling(&self.path, ".lock"), true)?;
        let mut records = read_records(&self.path)?;
        if edit(&mut records)? {
            write_records(&self.path, &records)?;
        }
        self.records = records;
        Ok(())
    }
}

fn sibling(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

fn read_records(path: &Path) -> Result<BTreeMap<String, TrustRecord>, TrustError> {
    match fs::read(path) {
        Ok(bytes) => decode(&bytes).ok_or(TrustError::Corrupt),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(BTreeMap::new()),
        Err(e) => Err(e.into()),
    }
}

/// A new file with a random name, created exclusively and readable only by this user, then
/// renamed over the old one. Called under the lock, so leftovers of a crashed write are removed
/// first. On POSIX the rename replaces the file in one step; the fsyncs make it durable, as far
/// as the filesystem honours them.
fn write_records(path: &Path, records: &BTreeMap<String, TrustRecord>) -> Result<(), TrustError> {
    remove_leftovers(path)?;
    let temporary = sibling(path, &format!(".{}.tmp", random_part()?));
    let result = (|| -> io::Result<()> {
        let mut file = create_exclusive(&temporary)?;
        file.write_all(&encode(records))?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, path)?;
        sync_parent(path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    Ok(result?)
}

fn remove_leftovers(path: &Path) -> io::Result<()> {
    let (Some(dir), Some(name)) = (path.parent(), path.file_name()) else {
        return Ok(());
    };
    let prefix = format!("{}.", name.to_string_lossy());
    let dir = if dir.as_os_str().is_empty() {
        Path::new(".")
    } else {
        dir
    };
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let file_name = entry.file_name().to_string_lossy().into_owned();
        if file_name.starts_with(&prefix) && file_name.ends_with(".tmp") {
            fs::remove_file(entry.path())?;
        }
    }
    Ok(())
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
