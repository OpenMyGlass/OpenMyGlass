//! Small file helpers for the stores in this module: exclusive creation, a random name part, an
//! advisory lock, and syncing a directory.

use std::fs::{File, OpenOptions};
use std::io;
use std::path::Path;

use super::manifest::to_hex;

/// Creates a new file that did not exist, readable only by this user. Fails on any existing path,
/// a symbolic link included, so a planted link cannot redirect the write.
pub(super) fn create_exclusive(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

/// 16 random hex digits, for temporary names.
pub(super) fn random_part() -> io::Result<String> {
    let mut unique = [0u8; 8];
    getrandom::getrandom(&mut unique).map_err(|e| io::Error::other(e.to_string()))?;
    Ok(to_hex(&unique))
}

/// An exclusive advisory lock on a file, held until dropped. Advisory: it only orders code that
/// takes it too, which every store here does. On a platform without `flock` it is a no-op, and
/// the store must then be used from one place only.
pub(super) struct FileLock {
    _file: File,
}

impl FileLock {
    /// Takes the lock on `path`, creating the file if needed, without following a symbolic link.
    /// `wait` false fails with [`io::ErrorKind::WouldBlock`] instead of waiting.
    pub(super) fn acquire(path: &Path, wait: bool) -> io::Result<Self> {
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
        }
        let file = options.open(path)?;
        #[cfg(unix)]
        {
            use std::os::unix::io::AsRawFd;
            let operation = if wait {
                libc::LOCK_EX
            } else {
                libc::LOCK_EX | libc::LOCK_NB
            };
            loop {
                // SAFETY: flock on a file descriptor this function owns, with constant flags.
                if unsafe { libc::flock(file.as_raw_fd(), operation) } == 0 {
                    break;
                }
                let error = io::Error::last_os_error();
                if error.kind() != io::ErrorKind::Interrupted {
                    return Err(error);
                }
            }
        }
        #[cfg(not(unix))]
        let _ = wait;
        Ok(Self { _file: file })
    }
}

#[cfg(unix)]
pub(super) fn sync_parent(path: &Path) -> io::Result<()> {
    match path.parent() {
        Some(parent) => File::open(parent)?.sync_all(),
        None => Ok(()),
    }
}

#[cfg(not(unix))]
pub(super) fn sync_parent(_path: &Path) -> io::Result<()> {
    Ok(())
}
