//! Drives the real `omg` binary: a simulated Glass in one process, a companion in another,
//! over TCP on localhost, with the Glass told to damage a chunk and drop the link.

use std::{
    net::TcpListener,
    path::Path,
    process::{Child, Command, Output, Stdio},
};

const OMG: &str = env!("CARGO_BIN_EXE_omg");

struct KillOnDrop(Child);

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn omg(store: &Path, args: &[&str]) -> Output {
    Command::new(OMG)
        .arg("--store")
        .arg(store)
        .args(args)
        .env("RUST_LOG", "off")
        .output()
        .unwrap()
}

fn text(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[test]
fn deferred_note_survives_faults_and_arrives_byte_identical() {
    let tmp = tempfile::tempdir().unwrap();
    let path = |name: &str| tmp.path().join(name);
    let s = |p: &Path| p.to_str().unwrap().to_string();
    let (glass_dir, companion_dir) = (path("glass-notes"), path("companion-notes"));
    let (glass_store, companion_store) = (path("glass.json"), path("companion.json"));

    let voice: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
    let photo: Vec<u8> = (0..500_000u32).map(|i| (i % 241) as u8).collect();
    std::fs::write(path("voice.wav"), &voice).unwrap();
    std::fs::write(path("photo.jpg"), &photo).unwrap();

    // The note is taken with no companion and no sim running at all.
    let added = omg(
        &glass_store,
        &[
            "note-add",
            "--notes-dir",
            &s(&glass_dir),
            "--voice",
            &s(&path("voice.wav")),
            "--photo",
            &s(&path("photo.jpg")),
            "--locale",
            "it",
        ],
    );
    assert!(added.status.success(), "{}", text(&added));
    let added_out = text(&added);
    assert!(
        added_out.contains("waiting on Glass: 1 note"),
        "{added_out}"
    );
    let id = added_out
        .lines()
        .find_map(|l| l.strip_prefix("note "))
        .and_then(|l| l.split(' ').next())
        .unwrap()
        .to_string();

    // An empty attachment breaks the note rules, and leaves nothing behind.
    std::fs::write(path("v.mp4"), b"").unwrap();
    let refused = omg(
        &glass_store,
        &[
            "note-add",
            "--notes-dir",
            &s(&glass_dir),
            "--photo",
            &s(&path("photo.jpg")),
            "--video",
            &s(&path("v.mp4")),
        ],
    );
    assert!(!refused.status.success());

    // Find a free port for the Glass to listen on.
    let port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let addr = format!("127.0.0.1:{port}");
    let _glass = KillOnDrop(
        Command::new(OMG)
            .arg("--store")
            .arg(&glass_store)
            .args(["sim-glass", "--listen", &addr, "--notes-dir"])
            .arg(&glass_dir)
            .args([
                "--read-back",
                "--corrupt-chunk",
                "2",
                "--drop-after-chunks",
                "5",
            ])
            .env("RUST_LOG", "off")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );

    // The companion retries until the Glass process is up, then until the sync is done. The
    // injected drop forces at least one more connection.
    let synced = omg(
        &companion_store,
        &[
            "notes-sync",
            "--connect",
            &addr,
            "--notes-dir",
            &s(&companion_dir),
            "--fake-speech",
            "--reconnect",
            "50",
        ],
    );
    let out = text(&synced);
    assert!(synced.status.success(), "{out}");
    assert!(out.contains("rejected (Corrupt)"), "{out}");
    assert!(out.contains("reconnecting"), "{out}");
    assert!(out.contains("complete"), "{out}");
    assert!(out.contains("transcript: [fake transcript"), "{out}");
    assert!(out.contains("still on Glass: 0 note(s)"), "{out}");

    let note_dir = companion_dir.join(&id);
    assert_eq!(std::fs::read(note_dir.join("att-0.bin")).unwrap(), voice);
    assert_eq!(std::fs::read(note_dir.join("att-1.bin")).unwrap(), photo);
    assert!(!note_dir.join("att-0.part").exists());

    // Kept on the Glass: the default, and this sim was not told otherwise.
    assert!(glass_dir.join(&id).join("att-0.bin").exists());
}
