//! The crash-safe on-disk queue, used by both sides with their own root directory.
//!
//! ```text
//! <root>/<32 hex id>/record.json   the note's record, replaced atomically
//! <root>/<32 hex id>/att-<n>.bin   a sealed or verified attachment
//! <root>/<32 hex id>/att-<n>.part  an attachment being received (companion)
//! <root>/.staging-<id>/            a note being sealed; gone after a crash
//! <root>/.trash-<id>/              a note being deleted; gone after a crash
//! <root>/.corrupt-<id>/            an unreadable record, set aside and never deleted
//! ```
//!
//! Paths come only from a [`NoteId`] in hex and a numeric attachment id, so nothing a
//! peer sends can name a path. The root is whatever the platform gives: on a phone, a
//! directory in the app's private storage.
//!
//! Crash safety rests on three rules: a file is complete and synced before it is renamed
//! into place; a rename is followed by syncing its directory; and every multi-step change
//! leaves a state that the next [`NoteStore::load`] can recognize and finish or undo.

use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

use omg_protocol::{AttachmentId, AttachmentMeta, NoteId, NoteManifest, Sha256Digest};
use sha2::{Digest, Sha256};
use tracing::warn;

use super::{
    driver::NoteDraft,
    manifest_digest,
    record::{NoteRecord, OutgoingNote, OutgoingState, RECORD_FORMAT},
};
use crate::CoreError;

const RECORD_FILE: &str = "record.json";
const IO_BUF: usize = 64 * 1024;

/// What [`NoteStore::load`] found.
#[derive(Debug)]
pub struct Loaded<R> {
    /// Records that parsed.
    pub records: Vec<R>,
    /// Ids set aside as `.corrupt-*` because their record could not be used. Their files
    /// are kept; a UI should say so.
    pub set_aside: Vec<String>,
}

/// See the module docs.
#[derive(Debug, Clone)]
pub struct NoteStore {
    root: PathBuf,
}

impl NoteStore {
    /// Open a store, creating the directory, and clear what a crash left half done.
    pub fn open(root: impl AsRef<Path>) -> Result<NoteStore, CoreError> {
        let root = root.as_ref().to_path_buf();
        fs::create_dir_all(&root)?;
        for entry in fs::read_dir(&root)? {
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with(".staging-") || name.starts_with(".trash-") {
                fs::remove_dir_all(entry.path())?;
            }
        }
        sync_dir(&root)?;
        Ok(NoteStore { root })
    }

    /// Every note's record. A record that cannot be read is set aside, not deleted: one
    /// bad file must not stop the rest of the queue, nor destroy what it describes.
    pub fn load<R: NoteRecord>(&self) -> Result<Loaded<R>, CoreError> {
        let mut records = Vec::new();
        let mut set_aside = Vec::new();
        for entry in fs::read_dir(&self.root)? {
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_string_lossy().into_owned();
            let Some(id) = NoteId::from_hex(&name) else {
                continue;
            };
            let path = entry.path().join(RECORD_FILE);
            if !path.exists() && fs::remove_dir(entry.path()).is_ok() {
                continue; // Created, and the crash came before its first record.
            }
            match read_record::<R>(&path) {
                Ok(record) if record.note_id() == id && record.format() <= RECORD_FORMAT => {
                    records.push(record)
                }
                outcome => {
                    warn!(note = %name, ok = outcome.is_ok(), "setting aside an unusable note record");
                    fs::rename(entry.path(), self.root.join(format!(".corrupt-{name}")))?;
                    set_aside.push(name);
                }
            }
        }
        if !set_aside.is_empty() {
            sync_dir(&self.root)?;
        }
        Ok(Loaded { records, set_aside })
    }

    /// Atomically and durably replace a note's record.
    pub fn save<R: NoteRecord>(&self, record: &R) -> Result<(), CoreError> {
        let dir = self.note_dir(&record.note_id());
        if !dir.exists() {
            fs::create_dir_all(&dir)?;
            sync_dir(&self.root)?;
        }
        let json = serde_json::to_vec_pretty(record)
            .map_err(|e| CoreError::Store(format!("serialize note record: {e}")))?;
        write_durably(&dir, RECORD_FILE, &json)?;
        Ok(())
    }

    /// Where a verified or sealed attachment lives.
    pub fn attachment_path(&self, id: &NoteId, attachment: AttachmentId) -> PathBuf {
        self.note_dir(id).join(format!("att-{attachment}.bin"))
    }

    /// Where an attachment being received lives.
    pub fn partial_path(&self, id: &NoteId, attachment: AttachmentId) -> PathBuf {
        self.note_dir(id).join(format!("att-{attachment}.part"))
    }

    /// Copy the draft's files in and seal it as one atomic step: the note appears in the
    /// queue complete, or not at all. The sources are left untouched. Hashes are taken
    /// over the bytes written, so a source changing mid-copy cannot seal a wrong digest.
    pub fn seal(&self, id: NoteId, seq: u64, draft: &NoteDraft) -> Result<OutgoingNote, CoreError> {
        for source in &draft.attachments {
            let size = fs::metadata(&source.path)?.len();
            if size > source.kind.max_bytes() {
                return Err(CoreError::InvalidNote(format!(
                    "{} is {size} bytes, over the limit",
                    source.path.display()
                )));
            }
        }
        let staging = self.root.join(format!(".staging-{}", id.to_hex()));
        fs::create_dir_all(&staging)?;
        let sealed = self.fill_staging(&staging, id, seq, draft);
        let record = match sealed {
            Ok(record) => record,
            Err(e) => {
                let _ = fs::remove_dir_all(&staging);
                return Err(e);
            }
        };
        fs::rename(&staging, self.note_dir(&id))?;
        sync_dir(&self.root)?;
        Ok(record)
    }

    fn fill_staging(
        &self,
        staging: &Path,
        id: NoteId,
        seq: u64,
        draft: &NoteDraft,
    ) -> Result<OutgoingNote, CoreError> {
        let mut attachments = Vec::with_capacity(draft.attachments.len());
        for (i, source) in draft.attachments.iter().enumerate() {
            let attachment_id = AttachmentId::try_from(i)
                .map_err(|_| CoreError::InvalidNote("too many attachments".into()))?;
            let target = staging.join(format!("att-{attachment_id}.bin"));
            let (size, sha256) = copy_and_hash(&source.path, &target)?;
            attachments.push(AttachmentMeta {
                id: attachment_id,
                kind: source.kind,
                mime: source.mime.clone(),
                size,
                sha256,
                duration_ms: source.duration_ms,
            });
        }
        let manifest = NoteManifest {
            id,
            created_at_ms: draft.created_at_ms,
            locale: draft.locale,
            text: draft.text.clone(),
            attachments,
        };
        manifest
            .validate()
            .map_err(|e| CoreError::InvalidNote(format!("{e:?}")))?;
        let record = OutgoingNote {
            format: RECORD_FORMAT,
            seq,
            manifest_sha256: manifest_digest(&manifest),
            manifest,
            verified: Default::default(),
            state: OutgoingState::Queued,
            transcripts: Default::default(),
        };
        let json = serde_json::to_vec_pretty(&record)
            .map_err(|e| CoreError::Store(format!("serialize note record: {e}")))?;
        write_durably(staging, RECORD_FILE, &json)?;
        Ok(record)
    }

    /// Read a piece of a sealed attachment.
    pub fn read_chunk(
        &self,
        id: &NoteId,
        attachment: AttachmentId,
        offset: u64,
        len: u32,
    ) -> io::Result<Vec<u8>> {
        let mut file = File::open(self.attachment_path(id, attachment))?;
        file.seek(SeekFrom::Start(offset))?;
        let mut data = vec![0; len as usize];
        file.read_exact(&mut data)?;
        Ok(data)
    }

    /// Set the partial file to exactly `len` bytes, creating it if needed.
    pub fn truncate_partial(
        &self,
        id: &NoteId,
        attachment: AttachmentId,
        len: u64,
    ) -> io::Result<()> {
        fs::create_dir_all(self.note_dir(id))?;
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(self.partial_path(id, attachment))?;
        file.set_len(len)
    }

    /// Write into the partial file at `offset`. Writing the same bytes twice is harmless.
    pub fn write_partial(
        &self,
        id: &NoteId,
        attachment: AttachmentId,
        offset: u64,
        data: &[u8],
    ) -> io::Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .open(self.partial_path(id, attachment))?;
        file.seek(SeekFrom::Start(offset))?;
        file.write_all(data)
    }

    /// Make the partial file's contents durable.
    pub fn sync_partial(&self, id: &NoteId, attachment: AttachmentId) -> io::Result<()> {
        sync_file(&self.partial_path(id, attachment))
    }

    /// SHA-256 of the partial file.
    pub fn hash_partial(&self, id: &NoteId, attachment: AttachmentId) -> io::Result<Sha256Digest> {
        hash_file(&self.partial_path(id, attachment))
    }

    /// SHA-256 of the sealed or verified file.
    pub fn hash_attachment(
        &self,
        id: &NoteId,
        attachment: AttachmentId,
    ) -> io::Result<Sha256Digest> {
        hash_file(&self.attachment_path(id, attachment))
    }

    /// Durably move the partial file to its final place. Repeating this is harmless.
    pub fn commit_partial(&self, id: &NoteId, attachment: AttachmentId) -> io::Result<()> {
        let partial = self.partial_path(id, attachment);
        let target = self.attachment_path(id, attachment);
        if !partial.exists() && target.exists() {
            return Ok(());
        }
        sync_file(&partial)?;
        fs::rename(&partial, &target)?;
        sync_dir(&self.note_dir(id))
    }

    /// Throw away a final file that a crash left without its record's agreement.
    pub fn remove_attachment(&self, id: &NoteId, attachment: AttachmentId) -> io::Result<()> {
        fs::remove_file(self.attachment_path(id, attachment))?;
        sync_dir(&self.note_dir(id))
    }

    /// Remove a note and everything in it. The rename makes it atomic: once it is out of
    /// the queue's namespace a crash can only finish the job. Repeating this is harmless.
    pub fn delete_note(&self, id: &NoteId) -> io::Result<()> {
        let dir = self.note_dir(id);
        if !dir.exists() {
            return Ok(());
        }
        let trash = self.root.join(format!(".trash-{}", id.to_hex()));
        fs::rename(&dir, &trash)?;
        sync_dir(&self.root)?;
        fs::remove_dir_all(&trash)
    }

    fn note_dir(&self, id: &NoteId) -> PathBuf {
        self.root.join(id.to_hex())
    }
}

fn read_record<R: NoteRecord>(path: &Path) -> Result<R, String> {
    let bytes = fs::read(path).map_err(|e| e.to_string())?;
    serde_json::from_slice(&bytes).map_err(|e| e.to_string())
}

/// Write a file so that it either keeps its old content or has all of the new, across a
/// power cut: temp file, sync, rename, sync the directory.
fn write_durably(dir: &Path, name: &str, data: &[u8]) -> io::Result<()> {
    let tmp = dir.join(format!("{name}.tmp"));
    {
        let mut file = File::create(&tmp)?;
        file.write_all(data)?;
        file.sync_all()?;
    }
    fs::rename(&tmp, dir.join(name))?;
    sync_dir(dir)
}

fn copy_and_hash(source: &Path, target: &Path) -> io::Result<(u64, Sha256Digest)> {
    let mut input = File::open(source)?;
    let mut output = File::create(target)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0; IO_BUF];
    let mut size = 0u64;
    loop {
        let n = input.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        output.write_all(&buf[..n])?;
        size += n as u64;
    }
    output.sync_all()?;
    Ok((size, hasher.finalize().into()))
}

fn hash_file(path: &Path) -> io::Result<Sha256Digest> {
    let mut input = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0; IO_BUF];
    loop {
        let n = input.read(&mut buf)?;
        if n == 0 {
            return Ok(hasher.finalize().into());
        }
        hasher.update(&buf[..n]);
    }
}

/// Flush a file. Opened for writing because Windows refuses to flush a read-only handle.
fn sync_file(path: &Path) -> io::Result<()> {
    OpenOptions::new().write(true).open(path)?.sync_all()
}

/// Make a rename or a new file in this directory durable. On Windows a directory cannot be
/// opened like this and the file system journals renames, so there it is a no-op.
fn sync_dir(dir: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        File::open(dir)?.sync_all()
    }
    #[cfg(not(unix))]
    {
        let _ = dir;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notes::driver::{AttachmentSource, NoteDraft};
    use crate::notes::sha256;
    use omg_protocol::{AttachmentKind, Locale};

    const ID: NoteId = NoteId([0xab; 16]);

    fn source(dir: &Path, name: &str, data: &[u8], kind: AttachmentKind) -> AttachmentSource {
        let path = dir.join(name);
        fs::write(&path, data).unwrap();
        AttachmentSource {
            kind,
            mime: "x/y".into(),
            path,
            duration_ms: None,
        }
    }

    fn draft(attachments: Vec<AttachmentSource>) -> NoteDraft {
        NoteDraft {
            created_at_ms: 1,
            locale: Locale::It,
            text: Some("hello".into()),
            attachments,
        }
    }

    #[test]
    fn seal_is_all_or_nothing_and_leaves_the_sources_alone() {
        let tmp = tempfile::tempdir().unwrap();
        let store = NoteStore::open(tmp.path().join("q")).unwrap();
        let photo = source(tmp.path(), "p.jpg", b"photo", AttachmentKind::Photo);
        let video = source(tmp.path(), "v.mp4", b"video", AttachmentKind::Video);

        // Two videos on one note break the limits: nothing appears.
        let bad = store.seal(ID, 0, &draft(vec![photo.clone(), video.clone(), video]));
        assert!(matches!(bad, Err(CoreError::InvalidNote(_))));
        assert!(store.load::<OutgoingNote>().unwrap().records.is_empty());
        assert_eq!(fs::read_dir(tmp.path().join("q")).unwrap().count(), 0);

        let record = store.seal(ID, 0, &draft(vec![photo.clone()])).unwrap();
        assert_eq!(record.manifest.attachments[0].sha256, sha256(b"photo"));
        assert_eq!(record.manifest.attachments[0].size, 5);
        assert_eq!(fs::read(&photo.path).unwrap(), b"photo");
        assert_eq!(store.load::<OutgoingNote>().unwrap().records, vec![record]);
        assert_eq!(store.read_chunk(&ID, 0, 1, 3).unwrap(), b"hot");
        assert!(store.read_chunk(&ID, 0, 3, 3).is_err(), "past the end");
    }

    #[test]
    fn a_missing_source_seals_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let store = NoteStore::open(tmp.path().join("q")).unwrap();
        let mut gone = source(tmp.path(), "p.jpg", b"photo", AttachmentKind::Photo);
        gone.path = tmp.path().join("nope.jpg");
        assert!(store.seal(ID, 0, &draft(vec![gone])).is_err());
        assert!(store.load::<OutgoingNote>().unwrap().records.is_empty());
    }

    #[test]
    fn open_clears_half_done_work_and_keeps_the_queue() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("q");
        let record = {
            let store = NoteStore::open(&root).unwrap();
            store.seal(ID, 0, &draft(vec![])).unwrap()
        };
        fs::create_dir_all(root.join(".staging-dead")).unwrap();
        fs::write(root.join(".staging-dead/att-0.bin"), b"x").unwrap();
        fs::create_dir_all(root.join(".trash-dead")).unwrap();
        // The temp file of a record write interrupted before its rename.
        fs::write(
            root.join(ID.to_hex()).join("record.json.tmp"),
            b"{half a rec",
        )
        .unwrap();

        let store = NoteStore::open(&root).unwrap();
        assert!(!root.join(".staging-dead").exists());
        assert!(!root.join(".trash-dead").exists());
        assert_eq!(store.load::<OutgoingNote>().unwrap().records, vec![record]);
    }

    #[test]
    fn a_bad_record_is_set_aside_with_its_files_and_the_rest_still_loads() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("q");
        let store = NoteStore::open(&root).unwrap();
        let photo = source(tmp.path(), "p.jpg", b"photo", AttachmentKind::Photo);
        store.seal(ID, 0, &draft(vec![photo])).unwrap();
        let good = store.seal(NoteId([1; 16]), 1, &draft(vec![])).unwrap();

        let bad_dir = root.join(ID.to_hex());
        fs::write(bad_dir.join("record.json"), b"not json").unwrap();
        let loaded = store.load::<OutgoingNote>().unwrap();
        assert_eq!(loaded.records, vec![good.clone()]);
        assert_eq!(loaded.set_aside, vec![ID.to_hex()]);
        let kept = root.join(format!(".corrupt-{}", ID.to_hex()));
        assert_eq!(fs::read(kept.join("att-0.bin")).unwrap(), b"photo");

        // A record from a future format is left alone in the same way.
        let mut future = good.clone();
        future.format = RECORD_FORMAT + 1;
        store.save(&future).unwrap();
        let loaded = store.load::<OutgoingNote>().unwrap();
        assert!(loaded.records.is_empty());
        assert_eq!(loaded.set_aside, vec![good.manifest.id.to_hex()]);
    }

    #[test]
    fn an_empty_note_dir_from_a_crash_is_cleared_and_junk_names_are_ignored() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("q");
        let store = NoteStore::open(&root).unwrap();
        fs::create_dir_all(root.join(ID.to_hex())).unwrap();
        fs::create_dir_all(root.join("not-a-note")).unwrap();
        let loaded = store.load::<OutgoingNote>().unwrap();
        assert!(loaded.records.is_empty() && loaded.set_aside.is_empty());
        assert!(!root.join(ID.to_hex()).exists());
        assert!(root.join("not-a-note").exists());
    }

    #[test]
    fn partial_file_lifecycle() {
        let tmp = tempfile::tempdir().unwrap();
        let store = NoteStore::open(tmp.path().join("q")).unwrap();
        store.truncate_partial(&ID, 0, 0).unwrap();
        store.write_partial(&ID, 0, 0, b"hello").unwrap();
        store.write_partial(&ID, 0, 0, b"hello").unwrap();
        store.write_partial(&ID, 0, 5, b" tail").unwrap();
        store.truncate_partial(&ID, 0, 5).unwrap();
        store.sync_partial(&ID, 0).unwrap();
        assert_eq!(store.hash_partial(&ID, 0).unwrap(), sha256(b"hello"));

        store.commit_partial(&ID, 0).unwrap();
        store.commit_partial(&ID, 0).unwrap();
        assert_eq!(store.hash_attachment(&ID, 0).unwrap(), sha256(b"hello"));
        assert!(store.hash_partial(&ID, 0).is_err());

        store.delete_note(&ID).unwrap();
        store.delete_note(&ID).unwrap();
        assert!(store.hash_attachment(&ID, 0).is_err());
    }
}
