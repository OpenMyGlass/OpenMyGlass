//! Reading manifests and packages through a fetcher the platform provides.
//!
//! The core has no network and no TLS. Kotlin and Swift implement [`Fetcher`] on their own HTTP
//! stacks; tests implement it with a table. Redirects are returned to the core rather than
//! followed, so every hop passes the rules in [`super::url`]. Bytes reach a [`PackageSink`] as
//! they arrive and count for nothing until [`PackageSink::commit`], which only happens after the
//! size and the hash match. A download needs an [`InstallTicket`], so it cannot skip the rules
//! in [`super::select`].
//!
//! A platform that cannot block a thread on a stream can drive [`DownloadVerifier`] itself and
//! get the same guarantees: it is the whole check, without the loop.

use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::marker::PhantomData;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use semver::Version;
use sha2::{Digest, Sha256};
use thiserror::Error;

use super::fsutil::{create_exclusive, random_part, sync_parent, FileLock};
use super::manifest::{
    release_statement, to_hex, Manifest, ManifestError, Sha256Digest, MAX_MANIFEST_BYTES,
    MAX_PACKAGE_BYTES,
};
use super::select::InstallTicket;
use super::source::RepositorySource;
use super::url::{HttpsUrl, UrlError};

/// Most redirects followed for one request.
pub const MAX_REDIRECTS: usize = 5;
/// Total time allowed for reading a manifest.
pub const MANIFEST_TIMEOUT: Duration = Duration::from_secs(60);
/// Fixed part of the total time allowed for a package.
pub const PACKAGE_BASE_TIMEOUT: Duration = Duration::from_secs(60);
/// The slowest average rate accepted for a package on top of the fixed part, in bytes per second.
pub const PACKAGE_MIN_RATE: u64 = 16 * 1024;

const READ_CHUNK: usize = 64 * 1024;

/// A total deadline and a cancel switch for one download. Clones share the switch, so the UI can
/// keep one to cancel from another thread.
///
/// The core checks both between reads. It cannot interrupt a `read` that is blocked inside the
/// fetcher, so a fetcher must use [`remaining`](Self::remaining) for its timeouts.
#[derive(Debug, Clone)]
pub struct FetchControl {
    deadline: Instant,
    cancelled: Arc<AtomicBool>,
}

impl FetchControl {
    /// A control with this total time.
    pub fn with_timeout(timeout: Duration) -> Self {
        Self {
            deadline: Instant::now() + timeout,
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }

    /// The usual control for a manifest.
    pub fn for_manifest() -> Self {
        Self::with_timeout(MANIFEST_TIMEOUT)
    }

    /// The usual control for a package of this size: a server may not hold the thread for longer
    /// than the size takes at [`PACKAGE_MIN_RATE`].
    pub fn for_package(size: u64) -> Self {
        Self::with_timeout(PACKAGE_BASE_TIMEOUT + Duration::from_secs(size / PACKAGE_MIN_RATE))
    }

    /// Asks the download to stop.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }

    /// Whether [`cancel`](Self::cancel) was called.
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }

    /// The time left, for a fetcher's connect and read timeouts.
    pub fn remaining(&self) -> Duration {
        self.deadline.saturating_duration_since(Instant::now())
    }

    fn check(&self) -> Result<(), DownloadError> {
        if self.is_cancelled() {
            Err(DownloadError::Cancelled)
        } else if self.remaining().is_zero() {
            Err(DownloadError::DeadlineExceeded)
        } else {
            Ok(())
        }
    }
}

/// What a fetcher got for one request.
pub enum FetchResponse {
    /// A 200 response. The stream ends at the end of the body and fails on a broken connection or
    /// a body shorter than the server announced.
    Body(Box<dyn Read + Send>),
    /// A 301, 302, 303, 307 or 308, with its `Location`, not followed.
    Redirect(String),
    /// Any other status.
    Status(u16),
}

/// A GET over HTTPS on the platform's stack.
///
/// The implementation must: validate certificates as the platform does by default, with no way
/// to turn that off; not follow redirects; send no cookies, credentials or `Referer`; not use a
/// cache that can answer for an unverified body; keep its connect and read timeouts within
/// [`FetchControl::remaining`] and at most 30 seconds per read, and stop on
/// [`FetchControl::is_cancelled`]; and, where the platform allows it, refuse a name that resolves
/// to a private or loopback address. The core cannot check any of this.
pub trait Fetcher {
    /// Starts the request. Only called with a URL that passed [`HttpsUrl::parse`].
    fn get(&self, url: &HttpsUrl, control: &FetchControl) -> io::Result<FetchResponse>;
}

/// Why a download failed.
#[derive(Debug, Error)]
pub enum DownloadError {
    /// The connection or the sink failed.
    #[error("i/o: {0}")]
    Io(#[from] io::Error),
    /// The server answered with an error.
    #[error("the server answered {0}")]
    Status(u16),
    /// A redirect pointed somewhere that is not allowed.
    #[error("refused redirect: {0}")]
    BadRedirect(UrlError),
    /// A manifest redirected outside its repository: another origin, or on GitHub another
    /// repository. It would be shown under a label it did not come from, so the owner has to add
    /// the final address instead.
    #[error("the manifest redirects to another site: add that address instead")]
    CrossOriginManifest,
    /// More than [`MAX_REDIRECTS`] hops.
    #[error("too many redirects")]
    TooManyRedirects,
    /// More bytes than allowed. The transfer stops as soon as this is known.
    #[error("the download is larger than expected")]
    TooLarge,
    /// Fewer bytes than the manifest said.
    #[error("the download is smaller than expected")]
    TooShort,
    /// The bytes are not the ones the manifest names.
    #[error("the download does not match its hash")]
    HashMismatch,
    /// The manifest was refused.
    #[error("manifest: {0}")]
    Manifest(#[from] ManifestError),
    /// The owner cancelled.
    #[error("cancelled")]
    Cancelled,
    /// The total time ran out, which also catches a server sending too slowly.
    #[error("the download took too long")]
    DeadlineExceeded,
}

/// Follows redirects from `url`, each hop checked, up to a body.
fn open(
    fetcher: &dyn Fetcher,
    url: &HttpsUrl,
    control: &FetchControl,
    manifest_of: Option<&RepositorySource>,
) -> Result<Box<dyn Read + Send>, DownloadError> {
    let mut current = url.clone();
    for _ in 0..=MAX_REDIRECTS {
        control.check()?;
        match fetcher.get(&current, control)? {
            FetchResponse::Body(body) => return Ok(body),
            FetchResponse::Status(status) => return Err(DownloadError::Status(status)),
            FetchResponse::Redirect(location) => {
                current = current
                    .redirect(&location)
                    .map_err(DownloadError::BadRedirect)?;
                if manifest_of.is_some_and(|source| !source.covers(&current)) {
                    return Err(DownloadError::CrossOriginManifest);
                }
            }
        }
    }
    Err(DownloadError::TooManyRedirects)
}

fn read_retrying(body: &mut dyn Read, buffer: &mut [u8]) -> io::Result<usize> {
    loop {
        match body.read(buffer) {
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            other => return other,
        }
    }
}

/// Reads and checks a repository's manifest. No more than [`MAX_MANIFEST_BYTES`] are ever read,
/// and only from where the repository's label says: see [`RepositorySource::covers`].
pub fn fetch_manifest(
    fetcher: &dyn Fetcher,
    source: &RepositorySource,
    control: &FetchControl,
) -> Result<Manifest, DownloadError> {
    let mut body = open(fetcher, source.manifest_url(), control, Some(source))?;
    let mut bytes = Vec::new();
    let mut buffer = [0u8; 8 * 1024];
    loop {
        control.check()?;
        let n = read_retrying(&mut *body, &mut buffer)?;
        if n == 0 {
            break;
        }
        if bytes.len() + n > MAX_MANIFEST_BYTES {
            return Err(DownloadError::TooLarge);
        }
        bytes.extend_from_slice(&buffer[..n]);
    }
    Ok(Manifest::parse(&bytes, source)?)
}

/// Checks a stream of bytes against a release's size and hash, chunk by chunk.
pub struct DownloadVerifier {
    package: VerifiedPackage,
    expected_size: u64,
    expected_sha256: Sha256Digest,
    seen: u64,
    hasher: Sha256,
}

impl DownloadVerifier {
    /// Starts checking against the ticket's release.
    pub fn new(ticket: &InstallTicket<'_>) -> Self {
        let (app, release) = (ticket.app, ticket.release);
        Self {
            package: VerifiedPackage {
                app_id: app.id.clone(),
                version: release.version.clone(),
                sha256: release.sha256,
                size: release.size,
                publisher_key: app.publisher_key,
                signature: release.signature,
                statement: release_statement(&app.id, release),
            },
            // Already limited by the manifest; the second bound keeps this type safe on its own.
            expected_size: release.size.min(MAX_PACKAGE_BYTES),
            expected_sha256: release.sha256,
            seen: 0,
            hasher: Sha256::new(),
        }
    }

    /// Takes the next chunk. An error means: stop reading now and throw away what you have. The
    /// chunk that causes the error must not be stored.
    pub fn push(&mut self, chunk: &[u8]) -> Result<(), DownloadError> {
        let seen = self
            .seen
            .checked_add(chunk.len() as u64)
            .filter(|seen| *seen <= self.expected_size)
            .ok_or(DownloadError::TooLarge)?;
        self.seen = seen;
        self.hasher.update(chunk);
        Ok(())
    }

    /// Ends the stream: the size and the hash must match exactly. A platform driving this type
    /// itself makes its stored bytes usable only after this succeeds.
    pub fn finish(self) -> Result<VerifiedPackage, DownloadError> {
        if self.seen != self.expected_size {
            return Err(DownloadError::TooShort);
        }
        let sha256: Sha256Digest = self.hasher.finalize().into();
        if sha256 != self.expected_sha256 {
            return Err(DownloadError::HashMismatch);
        }
        Ok(self.package)
    }
}

/// Where package bytes go while they are unproven.
pub trait PackageSink {
    /// Takes more bytes. They must not be usable as a package yet.
    fn write(&mut self, chunk: &[u8]) -> io::Result<()>;
    /// The stream passed the checks: confirm what is stored, make it durable and usable.
    fn commit(&mut self) -> io::Result<()>;
    /// The bytes failed: throw them away.
    fn abort(&mut self);
}

/// Proof that a stream of bytes matched a release's size and hash. Only
/// [`DownloadVerifier::finish`] makes one. It is not yet allowed onto the Glass: that takes
/// [`super::TrustStore::record_install`], which consumes it and returns the
/// [`super::RecordedPackage`] the transfer needs. It says nothing about what the package does: see
/// the design doc.
#[derive(PartialEq, Eq)]
pub struct VerifiedPackage {
    pub(super) app_id: String,
    pub(super) version: Version,
    pub(super) sha256: Sha256Digest,
    pub(super) size: u64,
    pub(super) publisher_key: [u8; 32],
    pub(super) signature: [u8; 64],
    pub(super) statement: Vec<u8>,
}

/// Leaves out the statement and the signature, so a log line cannot become an `AppOffer`.
impl std::fmt::Debug for VerifiedPackage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VerifiedPackage")
            .field("app_id", &self.app_id)
            .field("version", &self.version)
            .field("sha256", &to_hex(&self.sha256))
            .field("size", &self.size)
            .finish_non_exhaustive()
    }
}

impl VerifiedPackage {
    /// The app.
    pub fn app_id(&self) -> &str {
        &self.app_id
    }

    /// The version.
    pub fn version(&self) -> &Version {
        &self.version
    }

    /// The hash the bytes were checked against.
    pub fn sha256(&self) -> &Sha256Digest {
        &self.sha256
    }

    /// The exact size.
    pub fn size(&self) -> u64 {
        self.size
    }

    /// The key that signed the release.
    pub fn publisher_key(&self) -> &[u8; 32] {
        &self.publisher_key
    }
}

/// Downloads the ticket's release into `sink`, checking as it goes. On any failure the sink is
/// aborted.
pub fn fetch_package(
    fetcher: &dyn Fetcher,
    ticket: &InstallTicket<'_>,
    sink: &mut dyn PackageSink,
    control: &FetchControl,
) -> Result<VerifiedPackage, DownloadError> {
    let result = transfer(fetcher, ticket, sink, control);
    if result.is_err() {
        sink.abort();
    }
    result
}

fn transfer(
    fetcher: &dyn Fetcher,
    ticket: &InstallTicket<'_>,
    sink: &mut dyn PackageSink,
    control: &FetchControl,
) -> Result<VerifiedPackage, DownloadError> {
    let mut body = open(fetcher, &ticket.release.url, control, None)?;
    let mut verifier = DownloadVerifier::new(ticket);
    let mut buffer = vec![0u8; READ_CHUNK];
    loop {
        control.check()?;
        let n = read_retrying(&mut *body, &mut buffer)?;
        if n == 0 {
            break;
        }
        verifier.push(&buffer[..n])?;
        sink.write(&buffer[..n])?;
    }
    let package = verifier.finish()?;
    sink.commit()?;
    Ok(package)
}

/// A directory of packages named only by their verified hash, so nothing from a manifest ever
/// becomes part of a path. One `PackageStore` per directory at a time: it holds a lock on the
/// directory for its whole life, so clearing what a crash left cannot touch a running download.
pub struct PackageStore {
    dir: PathBuf,
    _lock: FileLock,
}

impl PackageStore {
    /// Uses `dir`, private to the app, creating it if needed. Fails with
    /// [`io::ErrorKind::WouldBlock`] if another `PackageStore` has it open. Removes the unfinished
    /// files a crash left, which is safe because no other store can be writing here.
    pub fn open(dir: impl Into<PathBuf>) -> io::Result<Self> {
        let dir = dir.into();
        fs::create_dir_all(&dir)?;
        let lock = FileLock::acquire(&dir.join(".lock"), false)?;
        for entry in fs::read_dir(&dir)? {
            let path = entry?.path();
            if path.extension().is_some_and(|e| e == "part") {
                fs::remove_file(path)?;
            }
        }
        Ok(Self { dir, _lock: lock })
    }

    /// Where a verified package with this hash lives. It exists only after a commit.
    pub fn path_of(&self, sha256: &Sha256Digest) -> PathBuf {
        self.dir.join(format!("{}.omgapp", to_hex(sha256)))
    }

    /// Starts receiving the package with this hash, in a new file of its own, so concurrent
    /// downloads of one hash cannot touch each other's bytes.
    pub fn begin(&self, sha256: &Sha256Digest) -> io::Result<FileSink<'_>> {
        let hash = to_hex(sha256);
        for _ in 0..8 {
            let part = self.dir.join(format!("{hash}.{}.part", random_part()?));
            match create_exclusive(&part) {
                Ok(file) => {
                    return Ok(FileSink {
                        file: Some(file),
                        part,
                        target: self.path_of(sha256),
                        expected: *sha256,
                        _store: PhantomData,
                    })
                }
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e),
            }
        }
        Err(io::Error::other("no unique temporary name"))
    }

    /// Removes a package, for instance after the Glass confirmed the install.
    pub fn remove(&self, sha256: &Sha256Digest) -> io::Result<()> {
        match fs::remove_file(self.path_of(sha256)) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            other => other,
        }
    }
}

/// A [`PackageSink`] on a file of its own: a unique `.part` until commit, which hashes the file
/// itself and only then renames it. Dropped without a commit, it removes the `.part`. It borrows
/// its [`PackageStore`], so the store, and with it the directory lock, outlive every running
/// download: no new store can be opened on the directory and clear this `.part`.
///
/// ```compile_fail
/// use omg_core::apps::PackageStore;
/// let dir = std::env::temp_dir().join("omg-filesink-doc");
/// let sink = {
///     let store = PackageStore::open(&dir).unwrap();
///     store.begin(&[0; 32]).unwrap()
/// }; // the store, and its lock, would end here while the sink lives on
/// drop(sink);
/// ```
pub struct FileSink<'a> {
    file: Option<File>,
    part: PathBuf,
    target: PathBuf,
    expected: Sha256Digest,
    _store: PhantomData<&'a PackageStore>,
}

impl FileSink<'_> {
    fn file_matches(file: &mut File, expected: &Sha256Digest) -> io::Result<bool> {
        file.seek(SeekFrom::Start(0))?;
        let mut hasher = Sha256::new();
        let mut buffer = vec![0u8; READ_CHUNK];
        loop {
            let n = read_retrying(file, &mut buffer)?;
            if n == 0 {
                break;
            }
            hasher.update(&buffer[..n]);
        }
        let digest: Sha256Digest = hasher.finalize().into();
        Ok(digest == *expected)
    }
}

impl PackageSink for FileSink<'_> {
    fn write(&mut self, chunk: &[u8]) -> io::Result<()> {
        self.file
            .as_mut()
            .ok_or_else(|| io::Error::other("sink is closed"))?
            .write_all(chunk)
    }

    fn commit(&mut self) -> io::Result<()> {
        let file = self
            .file
            .as_mut()
            .ok_or_else(|| io::Error::other("sink is closed"))?;
        if !Self::file_matches(file, &self.expected)? {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "the stored file does not match its hash",
            ));
        }
        file.sync_all()?;
        // Until the rename succeeds the sink still owns the `.part`, so a failure here leaves
        // nothing behind once the sink is aborted or dropped.
        fs::rename(&self.part, &self.target)?;
        self.file = None;
        sync_parent(&self.target)
    }

    fn abort(&mut self) {
        self.file = None;
        let _ = fs::remove_file(&self.part);
    }
}

impl Drop for FileSink<'_> {
    fn drop(&mut self) {
        if self.file.is_some() {
            self.abort();
        }
    }
}
