//! Reading manifests and packages through a fetcher the platform provides.
//!
//! The core has no network and no TLS. Kotlin and Swift implement [`Fetcher`] on their own HTTP
//! stacks; tests implement it with a table. Redirects are returned to the core rather than
//! followed, so every hop passes the rules in [`super::url`]. Bytes reach a [`PackageSink`] as
//! they arrive and count for nothing until [`PackageSink::commit`], which only happens after the
//! size and the hash match.
//!
//! A platform that cannot block a thread on a stream can drive [`DownloadVerifier`] itself and
//! get the same guarantees: it is the whole check, without the loop.

use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use semver::Version;
use sha2::{Digest, Sha256};
use thiserror::Error;

use super::manifest::{to_hex, AppEntry, Manifest, ManifestError, Release, Sha256Digest};
use super::manifest::{MAX_MANIFEST_BYTES, MAX_PACKAGE_BYTES};
use super::source::RepositorySource;
use super::url::{HttpsUrl, UrlError};

/// Most redirects followed for one request.
pub const MAX_REDIRECTS: usize = 5;

const READ_CHUNK: usize = 64 * 1024;

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
/// cache that can answer for an unverified body; set connect and read timeouts.
pub trait Fetcher {
    /// Starts the request. Only called with a URL that passed [`HttpsUrl::parse`].
    fn get(&self, url: &HttpsUrl) -> io::Result<FetchResponse>;
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
}

/// Follows redirects from `url`, each hop checked, up to a body.
fn open(fetcher: &dyn Fetcher, url: &HttpsUrl) -> Result<Box<dyn Read + Send>, DownloadError> {
    let mut current = url.clone();
    for _ in 0..=MAX_REDIRECTS {
        match fetcher.get(&current)? {
            FetchResponse::Body(body) => return Ok(body),
            FetchResponse::Status(status) => return Err(DownloadError::Status(status)),
            FetchResponse::Redirect(location) => {
                current = current
                    .redirect(&location)
                    .map_err(DownloadError::BadRedirect)?;
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

/// Reads and checks a repository's manifest. No more than [`MAX_MANIFEST_BYTES`] are ever read.
pub fn fetch_manifest(
    fetcher: &dyn Fetcher,
    source: &RepositorySource,
) -> Result<Manifest, DownloadError> {
    let mut body = open(fetcher, source.manifest_url())?;
    let mut bytes = Vec::new();
    let mut buffer = [0u8; 8 * 1024];
    loop {
        let n = read_retrying(&mut *body, &mut buffer)?;
        if n == 0 {
            break;
        }
        if bytes.len() + n > MAX_MANIFEST_BYTES {
            return Err(DownloadError::TooLarge);
        }
        bytes.extend_from_slice(&buffer[..n]);
    }
    Ok(Manifest::parse(&bytes)?)
}

/// Checks a stream of bytes against a release's size and hash, chunk by chunk.
pub struct DownloadVerifier {
    app_id: String,
    version: Version,
    expected_size: u64,
    expected_sha256: Sha256Digest,
    seen: u64,
    hasher: Sha256,
}

impl DownloadVerifier {
    /// Starts checking against `release` of `app`.
    pub fn new(app: &AppEntry, release: &Release) -> Self {
        Self {
            app_id: app.id.clone(),
            version: release.version.clone(),
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
        Ok(VerifiedPackage {
            app_id: self.app_id,
            version: self.version,
            sha256,
            size: self.seen,
        })
    }
}

/// Where package bytes go while they are unproven.
pub trait PackageSink {
    /// Takes more bytes. They must not be usable as a package yet.
    fn write(&mut self, chunk: &[u8]) -> io::Result<()>;
    /// The bytes passed the checks: make them durable and usable as a package.
    fn commit(&mut self) -> io::Result<()>;
    /// The bytes failed: throw them away.
    fn abort(&mut self);
}

/// Proof that some bytes matched a release's size and hash. Only [`DownloadVerifier::finish`]
/// makes one, and it says nothing about what the package does: see the design doc.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedPackage {
    app_id: String,
    version: Version,
    sha256: Sha256Digest,
    size: u64,
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
}

/// Downloads a release into `sink`, checking as it goes. On any failure the sink is aborted.
pub fn fetch_package(
    fetcher: &dyn Fetcher,
    app: &AppEntry,
    release: &Release,
    sink: &mut dyn PackageSink,
) -> Result<VerifiedPackage, DownloadError> {
    let result = transfer(fetcher, app, release, sink);
    if result.is_err() {
        sink.abort();
    }
    result
}

fn transfer(
    fetcher: &dyn Fetcher,
    app: &AppEntry,
    release: &Release,
    sink: &mut dyn PackageSink,
) -> Result<VerifiedPackage, DownloadError> {
    let mut body = open(fetcher, &release.url)?;
    let mut verifier = DownloadVerifier::new(app, release);
    let mut buffer = vec![0u8; READ_CHUNK];
    loop {
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
/// becomes part of a path.
#[derive(Debug, Clone)]
pub struct PackageStore {
    dir: PathBuf,
}

impl PackageStore {
    /// Uses `dir`, private to the app, creating it if needed.
    pub fn open(dir: impl Into<PathBuf>) -> io::Result<Self> {
        let dir = dir.into();
        fs::create_dir_all(&dir)?;
        Ok(Self { dir })
    }

    /// Where a verified package with this hash lives. It exists only after a commit.
    pub fn path_of(&self, sha256: &Sha256Digest) -> PathBuf {
        self.dir.join(format!("{}.omgapp", to_hex(sha256)))
    }

    /// Starts receiving the package with this hash. One download per hash at a time: a second one
    /// overwrites the first's unfinished file. A file left by a crash is overwritten the same way.
    pub fn begin(&self, sha256: &Sha256Digest) -> io::Result<FileSink> {
        let part = self.dir.join(format!("{}.part", to_hex(sha256)));
        let file = File::create(&part)?;
        Ok(FileSink {
            file: Some(file),
            part,
            target: self.path_of(sha256),
        })
    }

    /// Removes a package, for instance after the Glass confirmed the install.
    pub fn remove(&self, sha256: &Sha256Digest) -> io::Result<()> {
        match fs::remove_file(self.path_of(sha256)) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            other => other,
        }
    }
}

/// A [`PackageSink`] on a file: a `.part` until commit, then an atomic rename. Dropped without a
/// commit, it removes the `.part`.
pub struct FileSink {
    file: Option<File>,
    part: PathBuf,
    target: PathBuf,
}

impl PackageSink for FileSink {
    fn write(&mut self, chunk: &[u8]) -> io::Result<()> {
        self.file
            .as_mut()
            .ok_or_else(|| io::Error::other("sink is closed"))?
            .write_all(chunk)
    }

    fn commit(&mut self) -> io::Result<()> {
        let file = self
            .file
            .take()
            .ok_or_else(|| io::Error::other("sink is closed"))?;
        file.sync_all()?;
        drop(file);
        fs::rename(&self.part, &self.target)?;
        sync_parent(&self.target)
    }

    fn abort(&mut self) {
        self.file = None;
        let _ = fs::remove_file(&self.part);
    }
}

impl Drop for FileSink {
    fn drop(&mut self) {
        if self.file.is_some() {
            self.abort();
        }
    }
}

#[cfg(unix)]
fn sync_parent(path: &Path) -> io::Result<()> {
    match path.parent() {
        Some(parent) => File::open(parent)?.sync_all(),
        None => Ok(()),
    }
}

#[cfg(not(unix))]
fn sync_parent(_path: &Path) -> io::Result<()> {
    Ok(())
}
