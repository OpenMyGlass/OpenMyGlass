//! The Applications section against hostile and broken repositories, with no network: the fetcher
//! is a table of canned responses, and it records what it was asked for.

use std::collections::HashMap;
use std::io::{self, Cursor, Read};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use ed25519_dalek::{Signer, SigningKey};
use omg_core::apps::manifest::{
    to_hex, MAX_MANIFEST_BYTES, MAX_PACKAGE_BYTES, MAX_RELEASES_PER_APP,
};
use omg_core::apps::*;
use semver::Version;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

const MANIFEST_URL: &str = "https://example.org/glass/omg-apps.json";
const APP: &str = "com.example.tally";

fn key() -> SigningKey {
    SigningKey::from_bytes(&[7; 32])
}

fn other_key() -> SigningKey {
    SigningKey::from_bytes(&[9; 32])
}

fn sha(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

/// A release as JSON, unsigned: `app_json` signs it.
fn release_json(version: &str, package: &[u8]) -> Value {
    json!({
        "version": version,
        "target": "armv7-linux-musleabihf",
        "min_api": 1,
        "url": format!("https://example.org/glass/tally-{version}.omgapp"),
        "size": package.len(),
        "sha256": to_hex(&sha(package)),
        "permissions": [],
    })
}

/// Fills `signature` the way the tool for authors will: statement of the release as parsed.
fn sign(app_id: &str, signer: &SigningKey, release: &mut Value) {
    let statement = release_statement(app_id, &parsed_release(release));
    release["signature"] = json!(to_hex(&signer.sign(&statement).to_bytes()));
}

fn parsed_release(release: &Value) -> Release {
    let hex32 = |s: &str| -> [u8; 32] {
        let mut out = [0u8; 32];
        for (i, slot) in out.iter_mut().enumerate() {
            *slot = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).unwrap();
        }
        out
    };
    let mut permissions: Vec<String> = release["permissions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p.as_str().unwrap().to_owned())
        .collect();
    permissions.sort();
    Release {
        version: Version::parse(release["version"].as_str().unwrap()).unwrap(),
        target: release["target"].as_str().unwrap().to_owned(),
        min_api: release["min_api"].as_u64().unwrap() as u32,
        url: HttpsUrl::parse(release["url"].as_str().unwrap()).unwrap(),
        size: release["size"].as_u64().unwrap(),
        sha256: hex32(release["sha256"].as_str().unwrap()),
        permissions,
        yanked: false,
    }
}

fn app_json(id: &str, signer: &SigningKey, mut releases: Vec<Value>) -> Value {
    for release in &mut releases {
        sign(id, signer, release);
    }
    json!({
        "id": id,
        "name": "Tally",
        "summary": "Counts things on a card.",
        "license": "Apache-2.0",
        "homepage": "https://example.org/tally",
        "publisher_key": to_hex(signer.verifying_key().as_bytes()),
        "releases": releases,
    })
}

fn manifest_json(apps: Vec<Value>) -> Value {
    json!({"schema": 1, "name": "Example apps", "apps": apps})
}

fn one_release() -> Value {
    manifest_json(vec![app_json(
        APP,
        &key(),
        vec![release_json("1.0.0", b"package")],
    )])
}

fn parse(value: &Value) -> Result<Manifest, ManifestError> {
    Manifest::parse(value.to_string().as_bytes())
}

fn profile() -> GlassProfile {
    GlassProfile {
        api_level: 1,
        targets: vec!["armv7-linux-musleabihf".to_owned()],
    }
}

// ------------------------------- manifest -------------------------------

#[test]
fn a_good_manifest_parses() {
    let manifest = parse(&one_release()).unwrap();
    assert_eq!(manifest.name, "Example apps");
    let app = manifest.app(APP).unwrap();
    assert_eq!(app.releases[0].version, Version::new(1, 0, 0));
    assert_eq!(app.releases[0].size, 7);
    assert_eq!(app.publisher_key, *key().verifying_key().as_bytes());
    assert!(manifest.app("com.example.other").is_none());
}

#[test]
fn the_format_in_the_design_doc_stays_valid_up_to_the_signature() {
    let doc = include_str!("../../../../docs/plans/2026-11-07-companion-applications.md");
    let start = doc.find("```json\n").expect("the doc has a json example") + 8;
    let end = start + doc[start..].find("```").unwrap();
    let mut example: Value = serde_json::from_str(&doc[start..end]).unwrap();
    // The example's key and signature are shortened for the page: put real ones in.
    let id = example["apps"][0]["id"].as_str().unwrap().to_owned();
    example["apps"][0]["publisher_key"] = json!(to_hex(key().verifying_key().as_bytes()));
    for release in example["apps"][0]["releases"].as_array_mut().unwrap() {
        sign(&id, &key(), release);
    }
    let manifest = parse(&example).unwrap();
    assert_eq!(manifest.apps.len(), 1);
    assert_eq!(manifest.apps[0].releases.len(), 2);
}

#[test]
fn refuses_files_over_the_limit_before_parsing() {
    let huge = vec![b' '; MAX_MANIFEST_BYTES + 1];
    assert_eq!(Manifest::parse(&huge), Err(ManifestError::TooLarge));
}

#[test]
fn refuses_what_is_not_a_json_object() {
    let inputs: Vec<Vec<u8>> = vec![
        b"".to_vec(),
        b"<html>404</html>".to_vec(),
        b"[]".to_vec(),
        b"null".to_vec(),
        vec![0xff, 0xfe, 0x00],
        b"\xef\xbb\xbf{\"schema\":1}".to_vec(),
        format!("{} trailing", one_release()).into_bytes(),
    ];
    for input in inputs {
        assert!(
            matches!(
                Manifest::parse(&input),
                Err(ManifestError::Malformed { .. })
            ),
            "{input:?}"
        );
    }
}

#[test]
fn deep_nesting_is_an_error_not_a_stack_overflow() {
    let mut deep = String::from("{\"schema\":1,\"x\":");
    deep.push_str(&"[".repeat(200_000));
    assert!(Manifest::parse(deep.as_bytes()).is_err());
    let open = "{\"a\":".repeat(90_000);
    assert!(Manifest::parse(open.as_bytes()).is_err());
}

#[test]
fn refuses_unknown_fields_at_every_level() {
    let mut top = one_release();
    top["extra"] = json!(1);
    let mut app = one_release();
    app["apps"][0]["extra"] = json!(1);
    let mut release = one_release();
    release["apps"][0]["releases"][0]["install_script"] = json!("rm -rf /");
    for value in [top, app, release] {
        assert!(matches!(
            parse(&value),
            Err(ManifestError::Malformed { .. })
        ));
    }
}

#[test]
fn refuses_duplicate_keys() {
    // Two parsers reading one file differently is the classic signature bypass.
    let good = one_release().to_string();
    let schema = good.replacen("\"schema\":1", "\"schema\":1,\"schema\":1", 1);
    assert_ne!(schema, good);
    assert!(matches!(
        Manifest::parse(schema.as_bytes()),
        Err(ManifestError::Malformed { .. })
    ));
    let name = good.replacen(
        "\"name\":\"Example apps\"",
        "\"name\":\"A\",\"name\":\"B\"",
        1,
    );
    assert_ne!(name, good);
    assert!(matches!(
        Manifest::parse(name.as_bytes()),
        Err(ManifestError::Malformed { .. })
    ));
}

#[test]
fn schema_rules() {
    let mut newer = one_release();
    newer["schema"] = json!(2);
    newer["something_new"] = json!(true);
    assert_eq!(parse(&newer), Err(ManifestError::UnsupportedSchema(2)));
    for bad in [json!(1.0), json!("1"), json!(-1), json!(null)] {
        let mut value = one_release();
        value["schema"] = bad;
        assert!(matches!(
            parse(&value),
            Err(ManifestError::Malformed { .. })
        ));
    }
}

#[test]
fn errors_never_repeat_text_from_the_file() {
    let mut value = one_release();
    let nasty = format!("\u{1b}[2J{}", "x".repeat(10_000));
    value[nasty.as_str()] = json!(1);
    value["name"] = json!(nasty);
    let shown = parse(&value).unwrap_err().to_string();
    assert!(shown.len() < 200, "{shown}");
    assert!(!shown.contains('\u{1b}'));
    assert!(!shown.contains("xxx"));
}

#[test]
fn enforces_counts() {
    assert_eq!(
        parse(&manifest_json(vec![])),
        Err(ManifestError::BadAppCount)
    );
    let too_many: Vec<Value> = (0..33)
        .map(|i| {
            app_json(
                &format!("com.example.a{i}"),
                &key(),
                vec![release_json("1.0.0", b"x")],
            )
        })
        .collect();
    assert_eq!(
        parse(&manifest_json(too_many)),
        Err(ManifestError::BadAppCount)
    );
    assert_eq!(
        parse(&manifest_json(vec![app_json(APP, &key(), vec![])])),
        Err(ManifestError::BadReleaseCount(0))
    );
    let releases: Vec<Value> = (0..=MAX_RELEASES_PER_APP)
        .map(|i| release_json(&format!("1.0.{i}"), b"x"))
        .collect();
    assert_eq!(
        parse(&manifest_json(vec![app_json(APP, &key(), releases)])),
        Err(ManifestError::BadReleaseCount(0))
    );
}

#[test]
fn refuses_duplicates_and_reserved_ids() {
    let app = app_json(APP, &key(), vec![release_json("1.0.0", b"x")]);
    assert_eq!(
        parse(&manifest_json(vec![app.clone(), app])),
        Err(ManifestError::DuplicateApp(1))
    );
    // The same version with two hashes, both validly signed.
    let twice = app_json(
        APP,
        &key(),
        vec![release_json("1.0.0", b"a"), release_json("1.0.0", b"b")],
    );
    assert_eq!(
        parse(&manifest_json(vec![twice])),
        Err(ManifestError::DuplicateVersion { app: 0, release: 1 })
    );
    let ours = app_json(
        "org.openmyglass.settings",
        &key(),
        vec![release_json("1.0.0", b"x")],
    );
    assert_eq!(
        parse(&manifest_json(vec![ours])),
        Err(ManifestError::ReservedId(0))
    );
}

/// Changes a field of the first release and returns the manifest, with the old signature.
fn with_release_field(field: &str, value: Value) -> Value {
    let mut manifest = one_release();
    manifest["apps"][0]["releases"][0][field] = value;
    manifest
}

#[test]
fn refuses_bad_release_fields() {
    let cases = [
        ("version", json!("1.0"), "version"),
        ("version", json!("1.0.0+build5"), "version"),
        ("version", json!("v1.0.0"), "version"),
        ("target", json!("../../bin"), "target"),
        ("target", json!(""), "target"),
        ("size", json!(0), "size"),
        ("size", json!(MAX_PACKAGE_BYTES + 1), "size"),
        ("sha256", json!("abc"), "sha256"),
        ("sha256", json!("A".repeat(64)), "sha256"),
        ("signature", json!("00"), "signature"),
        ("permissions", json!(["Location"]), "permissions"),
        (
            "permissions",
            json!(["location", "location"]),
            "permissions",
        ),
        ("permissions", json!(["a b"]), "permissions"),
        ("permissions", json!([""]), "permissions"),
        (
            "permissions",
            json!((0..17).map(|i| format!("p{i}")).collect::<Vec<_>>()),
            "permissions",
        ),
    ];
    for (name, value, field) in cases {
        let got = parse(&with_release_field(name, value.clone()));
        assert_eq!(
            got,
            Err(ManifestError::BadReleaseField {
                app: 0,
                release: 0,
                field
            }),
            "{name} = {value}"
        );
    }
    for wrong_type in [
        ("size", json!(-1)),
        ("size", json!(1.5)),
        ("size", json!("7")),
    ] {
        assert!(matches!(
            parse(&with_release_field(wrong_type.0, wrong_type.1)),
            Err(ManifestError::Malformed { .. })
        ));
    }
    assert_eq!(
        parse(&with_release_field("url", json!("http://example.org/a"))),
        Err(ManifestError::BadReleaseUrl {
            app: 0,
            release: 0,
            error: UrlError::NotHttps
        })
    );
    assert_eq!(
        parse(&with_release_field("url", json!("https://nas.local/a"))),
        Err(ManifestError::BadReleaseUrl {
            app: 0,
            release: 0,
            error: UrlError::PrivateHost
        })
    );
}

#[test]
fn refuses_bad_app_fields() {
    let cases = [
        ("id", json!("not an id"), "id"),
        ("id", json!("../../etc/passwd"), "id"),
        ("name", json!(""), "name"),
        ("name", json!("Set\u{202e}tings"), "name"),
        ("name", json!("x".repeat(65)), "name"),
        ("summary", json!("two\nlines"), "summary"),
        ("summary", json!("x".repeat(281)), "summary"),
        ("license", json!(""), "license"),
        ("publisher_key", json!("zz"), "publisher_key"),
        // All zeros is a small-order point: it would verify forged signatures.
        ("publisher_key", json!("00".repeat(32)), "publisher_key"),
    ];
    for (name, value, field) in cases {
        let mut manifest = one_release();
        manifest["apps"][0][name] = value.clone();
        assert_eq!(
            parse(&manifest),
            Err(ManifestError::BadAppField { app: 0, field }),
            "{name} = {value}"
        );
    }
    let mut homepage = one_release();
    homepage["apps"][0]["homepage"] = json!("javascript:alert(1)");
    assert_eq!(
        parse(&homepage),
        Err(ManifestError::BadAppUrl {
            app: 0,
            field: "homepage",
            error: UrlError::NotHttps
        })
    );
    let mut no_homepage = one_release();
    no_homepage["apps"][0]
        .as_object_mut()
        .unwrap()
        .remove("homepage");
    assert!(parse(&no_homepage).unwrap().apps[0].homepage.is_none());
}

#[test]
fn the_signature_covers_everything_that_decides_what_runs() {
    let bad = Err(ManifestError::BadSignature { app: 0, release: 0 });
    let cases = [
        ("version", json!("1.0.1")),
        ("target", json!("wasm32-wasip1")),
        ("min_api", json!(2)),
        ("size", json!(8)),
        ("sha256", json!(to_hex(&sha(b"evil package")))),
        ("permissions", json!(["location"])),
    ];
    for (field, value) in cases {
        assert_eq!(parse(&with_release_field(field, value)), bad, "{field}");
    }
    // The id: a signed release cannot be moved to another app.
    let mut moved = one_release();
    moved["apps"][0]["id"] = json!("com.example.other");
    assert_eq!(parse(&moved), bad);
    // Another key: the file is self-consistent only if it was signed by the key it names.
    let mut replaced = one_release();
    replaced["apps"][0]["publisher_key"] = json!(to_hex(other_key().verifying_key().as_bytes()));
    assert_eq!(parse(&replaced), bad);
    // A signature from elsewhere, well formed.
    assert_eq!(
        parse(&with_release_field("signature", json!("ab".repeat(64)))),
        bad
    );
}

#[test]
fn hosting_facts_are_not_signed() {
    // A package can move to a mirror, and an author can withdraw a release, without the key. Both
    // are safe: the hash still has to match, and yanking only hides.
    let moved = with_release_field("url", json!("https://mirror.example.net/t.omgapp"));
    assert_eq!(
        parse(&moved).unwrap().apps[0].releases[0].url.host(),
        "mirror.example.net"
    );
    let yanked = with_release_field("yanked", json!(true));
    assert!(parse(&yanked).unwrap().apps[0].releases[0].yanked);
}

#[test]
fn permission_order_does_not_matter() {
    let mut release = release_json("1.0.0", b"x");
    release["permissions"] = json!(["location", "camera"]);
    let manifest = manifest_json(vec![app_json(APP, &key(), vec![release])]);
    assert_eq!(
        parse(&manifest).unwrap().apps[0].releases[0].permissions,
        ["camera", "location"]
    );
}

#[test]
fn no_mutation_of_a_good_file_panics() {
    let good = manifest_json(vec![app_json(
        APP,
        &key(),
        vec![
            release_json("1.0.0", b"a"),
            release_json("1.1.0-beta.1", b"b"),
        ],
    )])
    .to_string()
    .into_bytes();
    // A fixed generator, so a failure repeats.
    let mut state = 0x9e37_79b9_7f4a_7c15u64;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state as usize
    };
    let original = Manifest::parse(&good).unwrap();
    let mut passed = 0;
    for _ in 0..20_000 {
        let mut bytes = good.clone();
        for _ in 0..(next() % 4 + 1) {
            let at = next() % bytes.len();
            match next() % 3 {
                0 => bytes[at] = next() as u8,
                1 => {
                    bytes.remove(at);
                }
                _ => bytes.insert(at, b"{}[]\",:0"[next() % 8]),
            }
        }
        // Whatever survives may differ only in what is not signed: nothing that decides what runs.
        if let Ok(parsed) = Manifest::parse(&bytes) {
            passed += 1;
            assert_eq!(parsed.apps.len(), 1);
            assert_eq!(parsed.apps[0].id, APP);
            assert_eq!(parsed.apps[0].publisher_key, original.apps[0].publisher_key);
            for (got, want) in parsed.apps[0]
                .releases
                .iter()
                .zip(&original.apps[0].releases)
            {
                assert_eq!(
                    (
                        &got.version,
                        &got.target,
                        got.min_api,
                        got.size,
                        got.sha256,
                        &got.permissions
                    ),
                    (
                        &want.version,
                        &want.target,
                        want.min_api,
                        want.size,
                        want.sha256,
                        &want.permissions
                    ),
                );
            }
            assert_eq!(parsed.apps[0].releases.len(), 2);
        }
    }
    assert!(passed < 20_000, "the mutations must actually break files");
}

// ------------------------------- selection -------------------------------

fn app_with(versions: &[&str]) -> AppEntry {
    let releases = versions.iter().map(|v| release_json(v, b"x")).collect();
    parse(&manifest_json(vec![app_json(APP, &key(), releases)]))
        .unwrap()
        .apps
        .remove(0)
}

fn best(app: &AppEntry, options: SelectOptions) -> Option<String> {
    select_release(app, &profile(), options)
        .best
        .map(|r| r.version.to_string())
}

const STABLE: SelectOptions = SelectOptions {
    allow_prerelease: false,
};
const WITH_PRE: SelectOptions = SelectOptions {
    allow_prerelease: true,
};

#[test]
fn highest_version_wins_whatever_the_file_order() {
    // 1.10.0 beats 1.9.0: these are versions and not strings.
    let app = app_with(&["1.9.0", "1.10.0", "1.2.0"]);
    assert_eq!(best(&app, STABLE).as_deref(), Some("1.10.0"));
}

#[test]
fn prereleases_only_on_request_and_in_semver_order() {
    let app = app_with(&["1.0.0", "1.1.0-beta.2", "1.1.0-beta.10"]);
    let selection = select_release(&app, &profile(), STABLE);
    assert_eq!(selection.best.unwrap().version, Version::new(1, 0, 0));
    assert_eq!(selection.skipped.len(), 2);
    assert_eq!(selection.skipped[0].1, SkipReason::Prerelease);
    assert_eq!(best(&app, WITH_PRE).as_deref(), Some("1.1.0-beta.10"));
    let released = app_with(&["1.1.0-beta.2", "1.1.0"]);
    assert_eq!(best(&released, WITH_PRE).as_deref(), Some("1.1.0"));
}

/// An app with one release changed and signed again.
fn app_with_release(version: &str, change: impl Fn(&mut Value)) -> AppEntry {
    let mut release = release_json(version, b"x");
    change(&mut release);
    parse(&manifest_json(vec![app_json(APP, &key(), vec![release])]))
        .unwrap()
        .apps
        .remove(0)
}

#[test]
fn skips_what_this_glass_cannot_run_and_says_why() {
    let reason = |app: &AppEntry| {
        let selection = select_release(app, &profile(), STABLE);
        assert!(selection.best.is_none());
        selection.skipped[0].1.clone()
    };
    let target = app_with_release("1.0.0", |r| r["target"] = json!("wasm32-wasip1"));
    assert_eq!(reason(&target), SkipReason::WrongTarget);
    let api = app_with_release("1.0.0", |r| r["min_api"] = json!(9));
    assert_eq!(reason(&api), SkipReason::NeedsNewerGlass { min_api: 9 });
    let permission = app_with_release("1.0.0", |r| r["permissions"] = json!(["root.everything"]));
    assert_eq!(reason(&permission), SkipReason::UnknownPermission);
}

fn yank_first(manifest: &mut Value) {
    manifest["apps"][0]["releases"][0]["yanked"] = json!(true);
}

#[test]
fn yanked_releases_are_not_offered() {
    let mut manifest = manifest_json(vec![app_json(
        APP,
        &key(),
        vec![release_json("2.0.0", b"x"), release_json("1.0.0", b"y")],
    )]);
    yank_first(&mut manifest);
    let app = parse(&manifest).unwrap().apps.remove(0);
    assert_eq!(best(&app, STABLE).as_deref(), Some("1.0.0"));
}

fn repository() -> HttpsUrl {
    HttpsUrl::parse(MANIFEST_URL).unwrap()
}

fn installed(version: &str, package: &[u8], permissions: &[&str]) -> InstalledApp {
    InstalledApp {
        id: APP.to_owned(),
        version: Version::parse(version).unwrap(),
        repository: repository(),
        publisher_key: *key().verifying_key().as_bytes(),
        sha256: sha(package),
        permissions: permissions.iter().map(|p| p.to_string()).collect(),
    }
}

fn app_of(releases: Vec<Value>) -> AppEntry {
    parse(&manifest_json(vec![app_json(APP, &key(), releases)]))
        .unwrap()
        .apps
        .remove(0)
}

#[test]
fn an_update_is_offered_and_new_permissions_are_called_out() {
    let mut next = release_json("1.1.0", b"new");
    next["permissions"] = json!(["camera", "location"]);
    let app = app_of(vec![release_json("1.0.0", b"old"), next]);
    let status = check_update(
        &installed("1.0.0", b"old", &["camera"]),
        &repository(),
        &app,
        &profile(),
        STABLE,
    );
    let UpdateStatus::UpdateAvailable {
        release,
        added_permissions,
        installed_yanked,
    } = status
    else {
        panic!("{status:?}");
    };
    assert_eq!(release.version, Version::new(1, 1, 0));
    assert_eq!(added_permissions, ["location"]);
    assert!(!installed_yanked);
}

#[test]
fn never_a_downgrade() {
    // A host serving an old copy of the manifest, and an owner ahead of the stable line.
    for (running, available) in [("2.0.0", "1.0.0"), ("1.1.0-beta.1", "1.0.0")] {
        let app = app_of(vec![release_json(available, b"old")]);
        let status = check_update(
            &installed(running, b"new", &[]),
            &repository(),
            &app,
            &profile(),
            STABLE,
        );
        assert!(
            matches!(status, UpdateStatus::UpToDate { .. }),
            "{running}: {status:?}"
        );
    }
    let app = app_of(vec![release_json("1.1.0", b"stable")]);
    assert!(matches!(
        check_update(
            &installed("1.1.0-beta.1", b"beta", &[]),
            &repository(),
            &app,
            &profile(),
            STABLE
        ),
        UpdateStatus::UpdateAvailable { .. }
    ));
}

#[test]
fn a_new_key_blocks_updates() {
    let manifest = manifest_json(vec![app_json(
        APP,
        &other_key(),
        vec![release_json("9.0.0", b"evil")],
    )]);
    let app = parse(&manifest).unwrap().apps.remove(0);
    assert_eq!(
        check_update(
            &installed("1.0.0", b"old", &[]),
            &repository(),
            &app,
            &profile(),
            STABLE
        ),
        UpdateStatus::PublisherChanged
    );
}

#[test]
fn the_same_id_from_another_repository_is_not_an_update() {
    let app = app_of(vec![release_json("9.0.0", b"other")]);
    let elsewhere = HttpsUrl::parse("https://elsewhere.example.net/omg-apps.json").unwrap();
    assert_eq!(
        check_update(
            &installed("1.0.0", b"old", &[]),
            &elsewhere,
            &app,
            &profile(),
            STABLE
        ),
        UpdateStatus::OtherSource
    );
}

#[test]
fn a_republished_version_is_flagged() {
    let app = app_of(vec![
        release_json("1.0.0", b"swapped"),
        release_json("1.1.0", b"new"),
    ]);
    assert_eq!(
        check_update(
            &installed("1.0.0", b"original", &[]),
            &repository(),
            &app,
            &profile(),
            STABLE
        ),
        UpdateStatus::ReleaseChanged
    );
}

#[test]
fn up_to_date_says_about_yanks_and_blocked_releases() {
    let mut newer = release_json("2.0.0", b"new");
    newer["min_api"] = json!(5);
    let mut manifest = manifest_json(vec![app_json(
        APP,
        &key(),
        vec![release_json("1.0.0", b"old"), newer],
    )]);
    yank_first(&mut manifest);
    let app = parse(&manifest).unwrap().apps.remove(0);
    let status = check_update(
        &installed("1.0.0", b"old", &[]),
        &repository(),
        &app,
        &profile(),
        STABLE,
    );
    let two = Version::new(2, 0, 0);
    assert_eq!(
        status,
        UpdateStatus::UpToDate {
            installed_yanked: true,
            newer_skipped: vec![(&two, SkipReason::NeedsNewerGlass { min_api: 5 })],
        }
    );
}

// ------------------------------- downloads -------------------------------

#[derive(Clone)]
enum Canned {
    Body(Vec<u8>),
    Redirect(&'static str),
    Status(u16),
    Fails,
    /// Sends these bytes, then the connection breaks.
    Breaks(Vec<u8>),
    /// Never ends: counts what was read from it.
    Endless(Arc<AtomicU64>),
}

#[derive(Default)]
struct TableFetcher {
    table: HashMap<String, Canned>,
    asked: Mutex<Vec<String>>,
}

impl TableFetcher {
    fn with(mut self, url: &str, canned: Canned) -> Self {
        self.table.insert(url.to_owned(), canned);
        self
    }

    fn asked(&self) -> Vec<String> {
        self.asked.lock().unwrap().clone()
    }
}

struct Broken(Cursor<Vec<u8>>);

impl Read for Broken {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        match self.0.read(buffer)? {
            0 => Err(io::Error::new(io::ErrorKind::ConnectionReset, "reset")),
            n => Ok(n),
        }
    }
}

struct Endless(Arc<AtomicU64>);

impl Read for Endless {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.0.fetch_add(buffer.len() as u64, Ordering::Relaxed);
        buffer.fill(b' ');
        Ok(buffer.len())
    }
}

impl Fetcher for TableFetcher {
    fn get(&self, url: &HttpsUrl) -> io::Result<FetchResponse> {
        self.asked.lock().unwrap().push(url.to_string());
        Ok(match self.table.get(url.as_str()).cloned() {
            None => FetchResponse::Status(404),
            Some(Canned::Body(bytes)) => FetchResponse::Body(Box::new(Cursor::new(bytes))),
            Some(Canned::Redirect(to)) => FetchResponse::Redirect(to.to_owned()),
            Some(Canned::Status(status)) => FetchResponse::Status(status),
            Some(Canned::Fails) => return Err(io::Error::other("no route")),
            Some(Canned::Breaks(bytes)) => {
                FetchResponse::Body(Box::new(Broken(Cursor::new(bytes))))
            }
            Some(Canned::Endless(count)) => FetchResponse::Body(Box::new(Endless(count))),
        })
    }
}

const PACKAGE: &[u8] = b"a package, say";
const PACKAGE_URL: &str = "https://example.org/glass/tally-1.0.0.omgapp";

fn package_app() -> AppEntry {
    app_of(vec![release_json("1.0.0", PACKAGE)])
}

fn files(dir: &std::path::Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    names
}

/// Runs a download into a fresh store, returning the result and what is on disk.
fn download(fetcher: &TableFetcher) -> (Result<VerifiedPackage, DownloadError>, Vec<String>) {
    let dir = tempfile::tempdir().unwrap();
    let store = PackageStore::open(dir.path()).unwrap();
    let app = package_app();
    let release = &app.releases[0];
    let mut sink = store.begin(&release.sha256).unwrap();
    let result = fetch_package(fetcher, &app, release, &mut sink);
    drop(sink);
    (result, files(dir.path()))
}

#[test]
fn a_good_download_lands_under_its_hash() {
    let dir = tempfile::tempdir().unwrap();
    let store = PackageStore::open(dir.path()).unwrap();
    let app = package_app();
    let release = &app.releases[0];
    let fetcher = TableFetcher::default().with(PACKAGE_URL, Canned::Body(PACKAGE.to_vec()));
    let mut sink = store.begin(&release.sha256).unwrap();
    let package = fetch_package(&fetcher, &app, release, &mut sink).unwrap();
    drop(sink);
    assert_eq!(package.app_id(), APP);
    assert_eq!(*package.version(), Version::new(1, 0, 0));
    assert_eq!(package.size(), PACKAGE.len() as u64);
    assert_eq!(*package.sha256(), sha(PACKAGE));
    assert_eq!(
        files(dir.path()),
        [format!("{}.omgapp", to_hex(&sha(PACKAGE)))]
    );
    assert_eq!(
        std::fs::read(store.path_of(&sha(PACKAGE))).unwrap(),
        PACKAGE
    );
    store.remove(&sha(PACKAGE)).unwrap();
    store.remove(&sha(PACKAGE)).unwrap();
    assert!(files(dir.path()).is_empty());
}

#[test]
fn wrong_bytes_leave_nothing_behind() {
    let same_length: Vec<u8> = PACKAGE.iter().map(|b| b ^ 1).collect();
    let (result, left) =
        download(&TableFetcher::default().with(PACKAGE_URL, Canned::Body(same_length)));
    assert!(matches!(result, Err(DownloadError::HashMismatch)));
    assert!(left.is_empty(), "{left:?}");

    let (result, left) =
        download(&TableFetcher::default().with(PACKAGE_URL, Canned::Body(PACKAGE[1..].to_vec())));
    assert!(matches!(result, Err(DownloadError::TooShort)));
    assert!(left.is_empty(), "{left:?}");

    let (result, left) =
        download(&TableFetcher::default().with(PACKAGE_URL, Canned::Body(Vec::new())));
    assert!(matches!(result, Err(DownloadError::TooShort)));
    assert!(left.is_empty(), "{left:?}");
}

#[test]
fn an_endless_body_is_cut_at_the_declared_size() {
    let count = Arc::new(AtomicU64::new(0));
    let (result, left) =
        download(&TableFetcher::default().with(PACKAGE_URL, Canned::Endless(count.clone())));
    assert!(matches!(result, Err(DownloadError::TooLarge)));
    assert!(left.is_empty(), "{left:?}");
    // The first over-long read ends it.
    assert_eq!(count.load(Ordering::Relaxed), 64 * 1024);
}

#[test]
fn a_correct_prefix_with_extra_bytes_is_refused() {
    let mut longer = PACKAGE.to_vec();
    longer.push(0);
    let (result, left) = download(&TableFetcher::default().with(PACKAGE_URL, Canned::Body(longer)));
    assert!(matches!(result, Err(DownloadError::TooLarge)));
    assert!(left.is_empty());
}

#[test]
fn a_broken_connection_leaves_nothing_behind() {
    let (result, left) =
        download(&TableFetcher::default().with(PACKAGE_URL, Canned::Breaks(PACKAGE[..3].to_vec())));
    assert!(matches!(result, Err(DownloadError::Io(_))));
    assert!(left.is_empty(), "{left:?}");

    let (result, _) = download(&TableFetcher::default().with(PACKAGE_URL, Canned::Fails));
    assert!(matches!(result, Err(DownloadError::Io(_))));
    let (result, _) = download(&TableFetcher::default().with(PACKAGE_URL, Canned::Status(503)));
    assert!(matches!(result, Err(DownloadError::Status(503))));
}

#[test]
fn a_part_file_left_by_a_crash_is_overwritten() {
    let dir = tempfile::tempdir().unwrap();
    let store = PackageStore::open(dir.path()).unwrap();
    let app = package_app();
    let release = &app.releases[0];
    let part = dir.path().join(format!("{}.part", to_hex(&release.sha256)));
    std::fs::write(&part, vec![b'!'; 1000]).unwrap();
    let fetcher = TableFetcher::default().with(PACKAGE_URL, Canned::Body(PACKAGE.to_vec()));
    let mut sink = store.begin(&release.sha256).unwrap();
    fetch_package(&fetcher, &app, release, &mut sink).unwrap();
    assert!(!part.exists());
    assert_eq!(
        std::fs::read(store.path_of(&release.sha256)).unwrap(),
        PACKAGE
    );
}

#[test]
fn the_verifier_alone_gives_the_same_answers() {
    let app = package_app();
    let release = &app.releases[0];
    let mut verifier = DownloadVerifier::new(&app, release);
    for chunk in PACKAGE.chunks(3) {
        verifier.push(chunk).unwrap();
    }
    assert_eq!(*verifier.finish().unwrap().sha256(), sha(PACKAGE));

    let mut verifier = DownloadVerifier::new(&app, release);
    assert!(matches!(
        verifier.push(&vec![0; PACKAGE.len() + 1]),
        Err(DownloadError::TooLarge)
    ));
    // The refused chunk did not count.
    verifier.push(PACKAGE).unwrap();
    assert!(verifier.finish().is_ok());
}

#[test]
fn redirects_are_checked_before_any_request() {
    let cdn = "https://cdn.example.net/tally";
    let fetcher = TableFetcher::default()
        .with(PACKAGE_URL, Canned::Redirect(cdn))
        .with(cdn, Canned::Body(PACKAGE.to_vec()));
    let (result, _) = download(&fetcher);
    assert!(result.is_ok());
    assert_eq!(fetcher.asked(), [PACKAGE_URL, cdn]);

    for bad in [
        "http://cdn.example.net/tally",
        "https://nas.local/tally",
        "https://192.168.1.1/tally",
        "file:///etc/passwd",
        "//evil.example.net/x",
    ] {
        let fetcher = TableFetcher::default().with(PACKAGE_URL, Canned::Redirect(bad));
        let (result, _) = download(&fetcher);
        assert!(
            matches!(result, Err(DownloadError::BadRedirect(_))),
            "{bad}"
        );
        assert_eq!(fetcher.asked(), [PACKAGE_URL], "{bad} reached the fetcher");
    }
}

#[test]
fn a_redirect_loop_ends() {
    let fetcher = TableFetcher::default().with(PACKAGE_URL, Canned::Redirect(PACKAGE_URL));
    let (result, _) = download(&fetcher);
    assert!(matches!(result, Err(DownloadError::TooManyRedirects)));
    assert_eq!(fetcher.asked().len(), 6);
}

#[test]
fn a_manifest_stream_is_cut_at_the_limit() {
    let source = RepositorySource::parse(MANIFEST_URL).unwrap();
    let count = Arc::new(AtomicU64::new(0));
    let fetcher = TableFetcher::default().with(MANIFEST_URL, Canned::Endless(count.clone()));
    assert!(matches!(
        fetch_manifest(&fetcher, &source),
        Err(DownloadError::TooLarge)
    ));
    assert!(count.load(Ordering::Relaxed) <= MAX_MANIFEST_BYTES as u64 + 8 * 1024);

    let fetcher = TableFetcher::default().with(MANIFEST_URL, Canned::Body(b"<html>".to_vec()));
    assert!(matches!(
        fetch_manifest(&fetcher, &source),
        Err(DownloadError::Manifest(ManifestError::Malformed { .. }))
    ));
    assert!(matches!(
        fetch_manifest(&TableFetcher::default(), &source),
        Err(DownloadError::Status(404))
    ));
}

// ------------------------------- the whole path -------------------------------

#[test]
fn add_a_github_repository_install_then_update_then_survive_a_takeover() {
    let raw = "https://raw.githubusercontent.com/someone/tally/HEAD/omg-apps.json";
    let v1 = b"version one".as_slice();
    let v2 = b"version two".as_slice();
    let url_of =
        |v: &str| format!("https://github.com/someone/tally/releases/download/v{v}/t.omgapp");
    let asset = "https://release-assets.example.net/signed/t.omgapp";

    let mut r1 = release_json("1.0.0", v1);
    r1["url"] = json!(url_of("1.0.0"));
    let first = manifest_json(vec![app_json(APP, &key(), vec![r1.clone()])]).to_string();

    // Add the repository, the way the owner pastes it.
    let source = RepositorySource::parse("https://github.com/someone/tally").unwrap();
    assert_eq!(source.manifest_url().as_str(), raw);
    let fetcher = TableFetcher::default()
        .with(raw, Canned::Body(first.into_bytes()))
        .with(&url_of("1.0.0"), Canned::Body(v1.to_vec()));
    let manifest = fetch_manifest(&fetcher, &source).unwrap();

    // Install: the highest offered release, through the store.
    let app = manifest.app(APP).unwrap();
    let release = select_release(app, &profile(), STABLE).best.unwrap();
    let dir = tempfile::tempdir().unwrap();
    let store = PackageStore::open(dir.path()).unwrap();
    let mut sink = store.begin(&release.sha256).unwrap();
    let package = fetch_package(&fetcher, app, release, &mut sink).unwrap();
    let installed = InstalledApp {
        id: package.app_id().to_owned(),
        version: package.version().clone(),
        repository: source.manifest_url().clone(),
        publisher_key: app.publisher_key,
        sha256: *package.sha256(),
        permissions: release.permissions.clone(),
    };

    // A new version appears, behind a redirect to the asset host.
    let mut r2 = release_json("1.1.0", v2);
    r2["url"] = json!(url_of("1.1.0"));
    let second =
        manifest_json(vec![app_json(APP, &key(), vec![r1.clone(), r2.clone()])]).to_string();
    let redirect: &'static str = asset;
    let fetcher = TableFetcher::default()
        .with(raw, Canned::Body(second.into_bytes()))
        .with(&url_of("1.1.0"), Canned::Redirect(redirect))
        .with(asset, Canned::Body(v2.to_vec()));
    let manifest = fetch_manifest(&fetcher, &source).unwrap();
    let app = manifest.app(APP).unwrap();
    let UpdateStatus::UpdateAvailable { release, .. } =
        check_update(&installed, source.manifest_url(), app, &profile(), STABLE)
    else {
        panic!("no update offered");
    };
    let mut sink = store.begin(&release.sha256).unwrap();
    let updated = fetch_package(&fetcher, app, release, &mut sink).unwrap();
    assert_eq!(*updated.version(), Version::new(1, 1, 0));
    assert_eq!(std::fs::read(store.path_of(updated.sha256())).unwrap(), v2);

    // The account is stolen: a valid manifest under the thief's key, malicious "update" included.
    let mut evil = release_json("9.9.9", b"malware");
    evil["url"] = json!(url_of("9.9.9"));
    let stolen = manifest_json(vec![app_json(APP, &other_key(), vec![evil])]).to_string();
    let fetcher = TableFetcher::default().with(raw, Canned::Body(stolen.into_bytes()));
    let manifest = fetch_manifest(&fetcher, &source).unwrap();
    assert_eq!(
        check_update(
            &installed,
            source.manifest_url(),
            manifest.app(APP).unwrap(),
            &profile(),
            STABLE
        ),
        UpdateStatus::PublisherChanged
    );
}
