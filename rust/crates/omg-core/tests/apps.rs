//! The Applications section against hostile and broken repositories, with no network: the fetcher
//! is a table of canned responses, and it records what it was asked for.

use std::collections::HashMap;
use std::io::{self, Cursor, Read};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier};
use omg_core::apps::manifest::{
    to_hex, MAX_MANIFEST_BYTES, MAX_PACKAGE_BYTES, MAX_RELEASES_PER_APP,
};
use omg_core::apps::*;
use semver::Version;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

const MANIFEST_URL: &str = "https://example.org/glass/omg-apps.json";
const APP: &str = "com.example.tally";

/// The shape of a real GitHub release asset redirect (November 2026), with invented values: 900+
/// bytes, encoded slashes, colons and semicolons in the query. A real one is not kept here because
/// it carries a signature and a token.
const GITHUB_ASSET_REDIRECT: &str = "https://release-assets.githubusercontent.com/github-production-release-asset/212613049/82d5bc71-7921-4a42-bce0-29b120158ce5?sp=r&sv=2018-11-09&sr=b&spr=https&se=2026-11-07T01%3A02%3A20Z&rscd=attachment%3B+filename%3Dtally.omgapp&rsct=application%2Foctet-stream&skoid=00000000-0000-4000-8000-000000000000&sktid=00000000-0000-4000-8000-000000000001&skt=2026-11-07T00%3A01%3A30Z&ske=2026-11-07T01%3A02%3A20Z&sks=b&skv=2018-11-09&sig=AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA%3D&jwt=eyJ0eXAiOiJKV1QiLCJhbGciOiJIUzI1NiJ9.AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA.AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA&response-content-disposition=attachment%3B%20filename%3Dtally.omgapp&response-content-type=application%2Foctet-stream";

fn key() -> SigningKey {
    SigningKey::from_bytes(&[7; 32])
}

fn other_key() -> SigningKey {
    SigningKey::from_bytes(&[9; 32])
}

fn sha(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

fn source() -> RepositorySource {
    RepositorySource::parse(MANIFEST_URL).unwrap()
}

fn repository() -> HttpsUrl {
    HttpsUrl::parse(MANIFEST_URL).unwrap()
}

/// A release as JSON, unsigned.
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

/// Signs an app as JSON the way the tool for authors will: placeholders, then the statements
/// the core gives back for signing.
fn sign_app(app: &mut Value, signer: &SigningKey, repository: &HttpsUrl) {
    app["publisher_key"] = json!(to_hex(signer.verifying_key().as_bytes()));
    app["state_signature"] = json!("00".repeat(64));
    for release in app["releases"].as_array_mut().unwrap() {
        release["signature"] = json!("00".repeat(64));
    }
    let wrapped = json!({"schema": 1, "name": "x", "apps": [app.clone()]});
    let source = RepositorySource::from_manifest_url(repository).unwrap();
    // A structurally broken app cannot be signed; it keeps the placeholders, and the parse in the
    // test reports the structural error, which comes before any signature check.
    let Ok(to_sign) = Manifest::statements_to_sign(wrapped.to_string().as_bytes(), &source) else {
        return;
    };
    let to_sign = &to_sign[0];
    for (release, statement) in app["releases"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .zip(&to_sign.releases)
    {
        release["signature"] = json!(to_hex(&signer.sign(statement).to_bytes()));
    }
    app["state_signature"] = json!(to_hex(&signer.sign(&to_sign.state).to_bytes()));
}

fn unsigned_app(id: &str, sequence: u64, releases: Vec<Value>) -> Value {
    json!({
        "id": id,
        "name": "Tally",
        "summary": "Counts things on a card.",
        "license": "Apache-2.0",
        "homepage": "https://example.org/tally",
        "sequence": sequence,
        "releases": releases,
    })
}

fn app_json(id: &str, signer: &SigningKey, releases: Vec<Value>) -> Value {
    let mut app = unsigned_app(id, 1, releases);
    sign_app(&mut app, signer, &repository());
    app
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
    Manifest::parse(value.to_string().as_bytes(), &source())
}

fn parse_bytes(bytes: &[u8]) -> Result<Manifest, ManifestError> {
    Manifest::parse(bytes, &source())
}

fn empty_store() -> (tempfile::TempDir, TrustStore) {
    let dir = tempfile::tempdir().unwrap();
    let store = TrustStore::open(dir.path().join("trust.json")).unwrap();
    (dir, store)
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
    assert_eq!(manifest.name(), "Example apps");
    let app = manifest.app(APP).unwrap();
    assert_eq!(*app.releases()[0].version(), Version::new(1, 0, 0));
    assert_eq!(app.releases()[0].size(), 7);
    assert_eq!(*app.publisher_key(), *key().verifying_key().as_bytes());
    assert_eq!(*app.repository(), repository());
    assert!(manifest.app("com.example.other").is_none());
}

#[test]
fn the_format_in_the_design_doc_stays_valid_up_to_the_signatures() {
    let doc = include_str!("../../../../docs/plans/2026-11-07-companion-applications.md");
    let start = doc.find("```json\n").expect("the doc has a json example") + 8;
    let end = start + doc[start..].find("```").unwrap();
    let mut example: Value = serde_json::from_str(&doc[start..end]).unwrap();
    // The example's key and signatures are placeholders on the page: put real ones in.
    sign_app(&mut example["apps"][0], &key(), &repository());
    let manifest = parse(&example).unwrap();
    assert_eq!(manifest.apps().len(), 1);
    assert_eq!(manifest.apps()[0].releases().len(), 2);
    assert!(manifest.apps()[0].is_withdrawn(&Version::new(1, 0, 0)));
}

#[test]
fn refuses_files_over_the_limit_before_parsing() {
    let huge = vec![b' '; MAX_MANIFEST_BYTES + 1];
    assert_eq!(parse_bytes(&huge), Err(ManifestError::TooLarge));
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
            matches!(parse_bytes(&input), Err(ManifestError::Malformed { .. })),
            "{input:?}"
        );
    }
}

#[test]
fn deep_nesting_is_an_error_not_a_stack_overflow() {
    let mut deep = String::from("{\"schema\":1,\"x\":");
    deep.push_str(&"[".repeat(200_000));
    assert!(parse_bytes(deep.as_bytes()).is_err());
    let open = "{\"a\":".repeat(90_000);
    assert!(parse_bytes(open.as_bytes()).is_err());
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
        parse_bytes(schema.as_bytes()),
        Err(ManifestError::Malformed { .. })
    ));
    let name = good.replacen(
        "\"name\":\"Example apps\"",
        "\"name\":\"A\",\"name\":\"B\"",
        1,
    );
    assert_ne!(name, good);
    assert!(matches!(
        parse_bytes(name.as_bytes()),
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
    let nasty = format!("\u{1b}[2J{}", "x".repeat(10_000));
    let shown = |error: ManifestError| {
        let text = error.to_string();
        assert!(text.len() < 200, "{text}");
        assert!(!text.contains('\u{1b}'), "{text}");
        assert!(!text.contains("xxx"), "{text}");
    };

    // Each case reaches a different check, and each is asserted to reach it.
    let mut unknown_field = one_release();
    unknown_field[nasty.as_str()] = json!(1);
    let error = parse(&unknown_field).unwrap_err();
    assert!(matches!(error, ManifestError::Malformed { .. }));
    shown(error);

    let mut repository_name = one_release();
    repository_name["name"] = json!(nasty);
    let error = parse(&repository_name).unwrap_err();
    assert_eq!(error, ManifestError::BadField("name"));
    shown(error);

    let mut app_name = one_release();
    app_name["apps"][0]["name"] = json!(nasty);
    let error = parse(&app_name).unwrap_err();
    assert_eq!(
        error,
        ManifestError::BadAppField {
            app: 0,
            field: "name"
        }
    );
    shown(error);

    let mut release_url = one_release();
    release_url["apps"][0]["releases"][0]["url"] = json!(format!("https://example.org/{nasty}"));
    let error = parse(&release_url).unwrap_err();
    assert!(matches!(error, ManifestError::BadReleaseUrl { .. }));
    shown(error);

    let mut version = one_release();
    version["apps"][0]["releases"][0]["version"] = json!(nasty);
    let error = parse(&version).unwrap_err();
    assert!(matches!(error, ManifestError::BadReleaseField { .. }));
    shown(error);
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
    // The same version with two hashes, both validly signed: each signed alone, then combined.
    let first = app_json(APP, &key(), vec![release_json("1.0.0", b"a")]);
    let second = app_json(APP, &key(), vec![release_json("1.0.0", b"b")]);
    let mut twice = first.clone();
    twice["releases"]
        .as_array_mut()
        .unwrap()
        .push(second["releases"][0].clone());
    assert_eq!(
        parse(&manifest_json(vec![twice])),
        Err(ManifestError::DuplicateVersion { app: 0, release: 1 })
    );
    for ours in ["org.openmyglass", "org.openmyglass.settings"] {
        let app = app_json(ours, &key(), vec![release_json("1.0.0", b"x")]);
        assert_eq!(
            parse(&manifest_json(vec![app])),
            Err(ManifestError::ReservedId(0)),
            "{ours}"
        );
    }
    // A real label that only starts the same is someone else's.
    let similar = app_json(
        "org.openmyglassfan.app",
        &key(),
        vec![release_json("1.0.0", b"x")],
    );
    assert!(parse(&manifest_json(vec![similar])).is_ok());
}

/// Changes a field of the first release and returns the manifest, with the old signatures.
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
        ("name", json!("\u{2800}"), "name"),
        ("name", json!("a\u{1D173}b"), "name"),
        ("name", json!("---"), "name"),
        ("name", json!("x".repeat(65)), "name"),
        ("summary", json!("two\nlines"), "summary"),
        ("summary", json!("x".repeat(281)), "summary"),
        ("license", json!(""), "license"),
        ("publisher_key", json!("zz"), "publisher_key"),
        // All zeros is a small-order point: it would verify forged signatures.
        ("publisher_key", json!("00".repeat(32)), "publisher_key"),
        ("withdrawn", json!(["not a version"]), "withdrawn"),
        ("withdrawn", json!(["1.0.0", "1.0.0"]), "withdrawn"),
        (
            "withdrawn",
            json!((0..65).map(|i| format!("1.0.{i}")).collect::<Vec<_>>()),
            "withdrawn",
        ),
        ("state_signature", json!("00"), "state_signature"),
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
}

#[test]
fn the_release_signature_covers_everything_that_decides_what_runs() {
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
fn the_state_signature_covers_what_the_owner_reads_and_what_is_withdrawn() {
    let bad = Err(ManifestError::BadStateSignature(0));
    let cases = [
        ("name", json!("Security fix")),
        ("summary", json!("Trust me.")),
        ("license", json!("Public domain")),
        ("homepage", json!("https://evil.example.net/")),
        ("homepage", Value::Null),
        ("sequence", json!(99)),
        ("withdrawn", json!(["1.0.0"])),
        ("state_signature", json!("ab".repeat(64))),
    ];
    for (field, value) in cases {
        let mut manifest = one_release();
        manifest["apps"][0][field] = value.clone();
        assert_eq!(parse(&manifest), bad, "{field} = {value}");
    }
}

#[test]
fn only_the_url_and_the_repository_name_are_unsigned() {
    // A package can move to a mirror without the key: the hash still has to match.
    let moved = with_release_field("url", json!("https://mirror.example.net/t.omgapp"));
    assert_eq!(
        parse(&moved).unwrap().apps()[0].releases()[0].url().host(),
        "mirror.example.net"
    );
    let mut name = one_release();
    name["name"] = json!("Official OpenMyGlass apps");
    assert_eq!(parse(&name).unwrap().name(), "Official OpenMyGlass apps");
}

fn with_bad_and_fix() -> Value {
    let mut app = unsigned_app(
        APP,
        2,
        vec![
            release_json("1.0.0", b"old"),
            release_json("1.1.0", b"bad"),
            release_json("1.1.1", b"fix"),
        ],
    );
    app["withdrawn"] = json!(["1.1.0"]);
    sign_app(&mut app, &key(), &repository());
    app
}

#[test]
fn without_the_key_a_withdrawal_cannot_be_cleared_and_a_fix_cannot_be_dropped() {
    let honest = parse(&manifest_json(vec![with_bad_and_fix()])).unwrap();
    assert!(honest.apps()[0].is_withdrawn(&Version::new(1, 1, 0)));

    let mut cleared = with_bad_and_fix();
    cleared.as_object_mut().unwrap().remove("withdrawn");
    assert_eq!(
        parse(&manifest_json(vec![cleared])),
        Err(ManifestError::BadStateSignature(0))
    );

    let mut emptied = with_bad_and_fix();
    emptied["withdrawn"] = json!([]);
    assert_eq!(
        parse(&manifest_json(vec![emptied])),
        Err(ManifestError::BadStateSignature(0))
    );

    let mut dropped = with_bad_and_fix();
    dropped["releases"].as_array_mut().unwrap().remove(2);
    assert_eq!(
        parse(&manifest_json(vec![dropped])),
        Err(ManifestError::BadStateSignature(0))
    );
}

#[test]
fn a_manifest_signed_for_one_repository_is_refused_from_another() {
    let elsewhere = RepositorySource::parse("https://elsewhere.example.net/omg-apps.json").unwrap();
    assert_eq!(
        Manifest::parse(one_release().to_string().as_bytes(), &elsewhere),
        Err(ManifestError::BadStateSignature(0))
    );
}

#[test]
fn permission_order_does_not_matter() {
    let mut release = release_json("1.0.0", b"x");
    release["permissions"] = json!(["location", "camera"]);
    let manifest = manifest_json(vec![app_json(APP, &key(), vec![release])]);
    assert_eq!(
        parse(&manifest).unwrap().apps()[0].releases()[0].permissions(),
        ["camera", "location"]
    );
}

/// Everything signed in a manifest: all of it but the repository's name and the release URLs.
fn signed_view(manifest: &Manifest) -> String {
    let apps: Vec<String> = manifest
        .apps()
        .iter()
        .map(|a| {
            let releases: Vec<String> = a
                .releases()
                .iter()
                .map(|r| {
                    format!(
                        "{:?}",
                        (
                            r.version(),
                            r.target(),
                            r.min_api(),
                            r.size(),
                            r.sha256(),
                            r.permissions(),
                            r.signature()
                        )
                    )
                })
                .collect();
            format!(
                "{:?}",
                (
                    a.id(),
                    a.repository(),
                    a.name(),
                    a.summary(),
                    a.license(),
                    a.homepage(),
                    a.publisher_key(),
                    a.sequence(),
                    a.withdrawn(),
                    a.state_signature(),
                    releases
                )
            )
        })
        .collect();
    apps.join("\n")
}

#[test]
fn no_mutation_of_a_good_file_panics_or_changes_anything_signed() {
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
    let original = parse_bytes(&good).unwrap();
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
        if let Ok(parsed) = parse_bytes(&bytes) {
            passed += 1;
            // Whatever survives may differ only in what is not signed.
            assert_eq!(signed_view(&parsed), signed_view(&original));
        }
    }
    assert!(passed < 20_000, "the mutations must actually break files");
}

// ------------------------------- selection -------------------------------

fn app_with(versions: &[&str]) -> AppEntry {
    let releases = versions.iter().map(|v| release_json(v, b"x")).collect();
    app_of(releases)
}

fn app_of(releases: Vec<Value>) -> AppEntry {
    parse(&manifest_json(vec![app_json(APP, &key(), releases)]))
        .unwrap()
        .apps()[0]
        .clone()
}

/// An app in a chosen state.
fn app_state(
    signer: &SigningKey,
    sequence: u64,
    withdrawn: &[&str],
    releases: Vec<Value>,
) -> AppEntry {
    let mut app = unsigned_app(APP, sequence, releases);
    app["withdrawn"] = json!(withdrawn);
    sign_app(&mut app, signer, &repository());
    parse(&manifest_json(vec![app])).unwrap().apps()[0].clone()
}

fn best(app: &AppEntry, options: SelectOptions) -> Option<String> {
    select_release(app, None, &profile(), options)
        .best
        .map(|r| r.version().to_string())
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
    let selection = select_release(&app, None, &profile(), STABLE);
    assert_eq!(*selection.best.unwrap().version(), Version::new(1, 0, 0));
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
    app_of(vec![release])
}

#[test]
fn skips_what_this_glass_cannot_run_and_says_why() {
    let reason = |app: &AppEntry| {
        let selection = select_release(app, None, &profile(), STABLE);
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

#[test]
fn withdrawn_releases_are_not_offered() {
    let app = app_state(
        &key(),
        1,
        &["2.0.0"],
        vec![release_json("2.0.0", b"x"), release_json("1.0.0", b"y")],
    );
    assert_eq!(best(&app, STABLE).as_deref(), Some("1.0.0"));
}

// ------------------------------- trust and updates -------------------------------

/// A store that has installed `app`'s selected release, and what is on the Glass after it.
fn installed_from(app: &AppEntry) -> (tempfile::TempDir, TrustStore, InstalledApp) {
    let dir = tempfile::tempdir().unwrap();
    let mut store = TrustStore::open(dir.path().join("trust.json")).unwrap();
    let ticket = plan_install(app, &empty_store().1, None, &profile(), WITH_PRE).unwrap();
    assert!(ticket.first_use());
    let package = verified(&ticket);
    store.record_install(&ticket, &package).unwrap();
    let installed = installed_of(&ticket);
    (dir, store, installed)
}

fn installed_of(ticket: &InstallTicket<'_>) -> InstalledApp {
    InstalledApp {
        id: ticket.app().id().to_owned(),
        version: ticket.release().version().clone(),
        sha256: *ticket.release().sha256(),
        permissions: ticket.release().permissions().to_vec(),
    }
}

/// A real verification of the small test packages, which are all listed here.
fn verified(ticket: &InstallTicket<'_>) -> VerifiedPackage {
    let bytes = PACKAGES
        .iter()
        .find(|p| sha(p) == *ticket.release().sha256())
        .expect("the test package is listed");
    let mut verifier = DownloadVerifier::new(ticket);
    verifier.push(bytes).unwrap();
    verifier.finish().unwrap()
}

const PACKAGES: &[&[u8]] = &[b"old", b"new", b"bad", b"fix", b"x", b"evil", b"other"];

#[test]
fn an_update_is_offered_and_new_permissions_are_called_out() {
    let old = app_of(vec![release_json("1.0.0", b"old")]);
    let (_dir, store, mut installed) = installed_from(&old);
    installed.permissions = vec!["camera".into()];
    let mut next = release_json("1.1.0", b"new");
    next["permissions"] = json!(["camera", "location"]);
    let app = app_state(&key(), 2, &[], vec![release_json("1.0.0", b"old"), next]);
    let status = check_update(
        &installed,
        store.get(APP).unwrap(),
        &app,
        &profile(),
        STABLE,
    );
    let UpdateStatus::UpdateAvailable {
        release,
        added_permissions,
        installed_withdrawn,
    } = status
    else {
        panic!("{status:?}");
    };
    assert_eq!(*release.version(), Version::new(1, 1, 0));
    assert_eq!(added_permissions, ["location"]);
    assert!(!installed_withdrawn);

    let ticket = plan_install(&app, &store, Some(&installed), &profile(), STABLE).unwrap();
    assert!(!ticket.first_use());
    assert_eq!(ticket.permissions_to_confirm(), ["location"]);
}

#[test]
fn a_first_install_confirms_every_permission() {
    let app = app_with_release("1.0.0", |r| {
        r["permissions"] = json!(["location", "camera"])
    });
    let ticket = plan_install(&app, &empty_store().1, None, &profile(), STABLE).unwrap();
    assert!(ticket.first_use());
    assert_eq!(ticket.permissions_to_confirm(), ["camera", "location"]);
}

#[test]
fn never_a_downgrade_from_a_lower_stable_version() {
    let installed_on = app_of(vec![
        release_json("1.0.0", b"old"),
        release_json("1.1.0-beta.1", b"new"),
    ]);
    let (_dir, store, installed) = installed_from(&installed_on);
    assert_eq!(installed.version.to_string(), "1.1.0-beta.1");
    let later = app_state(
        &key(),
        2,
        &[],
        vec![
            release_json("1.0.0", b"old"),
            release_json("1.1.0-beta.1", b"new"),
        ],
    );
    let status = check_update(
        &installed,
        store.get(APP).unwrap(),
        &later,
        &profile(),
        STABLE,
    );
    assert!(
        matches!(status, UpdateStatus::UpToDate { .. }),
        "{status:?}"
    );
    assert_eq!(
        plan_install(&later, &store, Some(&installed), &profile(), STABLE).unwrap_err(),
        InstallRefusal::NothingToOffer
    );
    // A real stable release above the beta is an update.
    let released = app_state(
        &key(),
        3,
        &[],
        vec![
            release_json("1.1.0-beta.1", b"new"),
            release_json("1.1.0", b"fix"),
        ],
    );
    assert!(matches!(
        check_update(
            &installed,
            store.get(APP).unwrap(),
            &released,
            &profile(),
            STABLE
        ),
        UpdateStatus::UpdateAvailable { .. }
    ));
}

#[test]
fn a_new_key_blocks_both_the_update_check_and_the_install() {
    let (_dir, store, installed) = installed_from(&app_of(vec![release_json("1.0.0", b"old")]));
    let thief = app_state(&other_key(), 9, &[], vec![release_json("9.0.0", b"evil")]);
    assert_eq!(
        check_update(
            &installed,
            store.get(APP).unwrap(),
            &thief,
            &profile(),
            STABLE
        ),
        UpdateStatus::Blocked(TrustProblem::PublisherChanged)
    );
    assert_eq!(
        plan_install(&thief, &store, Some(&installed), &profile(), STABLE).unwrap_err(),
        InstallRefusal::Untrusted(TrustProblem::PublisherChanged)
    );
}

#[test]
fn the_same_id_from_a_rival_in_another_repository_is_blocked() {
    let (_dir, store, installed) = installed_from(&app_of(vec![release_json("1.0.0", b"old")]));
    // Another site, another key, the same id, signed correctly for that site.
    let rival_site =
        RepositorySource::parse("https://elsewhere.example.net/omg-apps.json").unwrap();
    let mut app = unsigned_app(APP, 50, vec![release_json("9.0.0", b"other")]);
    sign_app(&mut app, &other_key(), rival_site.manifest_url());
    let rival = Manifest::parse(manifest_json(vec![app]).to_string().as_bytes(), &rival_site)
        .unwrap()
        .apps()[0]
        .clone();
    assert_eq!(
        *rival.publisher_key(),
        *other_key().verifying_key().as_bytes()
    );
    assert_eq!(
        check_update(
            &installed,
            store.get(APP).unwrap(),
            &rival,
            &profile(),
            STABLE
        ),
        UpdateStatus::Blocked(TrustProblem::OtherSource)
    );
    assert_eq!(
        plan_install(&rival, &store, None, &profile(), STABLE).unwrap_err(),
        InstallRefusal::Untrusted(TrustProblem::OtherSource)
    );
}

#[test]
fn a_republished_version_is_flagged() {
    let (_dir, store, installed) = installed_from(&app_of(vec![release_json("1.0.0", b"old")]));
    let swapped = app_state(
        &key(),
        2,
        &[],
        vec![release_json("1.0.0", b"bad"), release_json("1.1.0", b"new")],
    );
    assert_eq!(
        check_update(
            &installed,
            store.get(APP).unwrap(),
            &swapped,
            &profile(),
            STABLE
        ),
        UpdateStatus::Blocked(TrustProblem::ReleaseChanged)
    );
}

fn state_one() -> AppEntry {
    // The author has published a bad 1.1.0 and does not know yet.
    app_state(
        &key(),
        2,
        &[],
        vec![release_json("1.0.0", b"old"), release_json("1.1.0", b"bad")],
    )
}

fn state_two() -> AppEntry {
    app_state(
        &key(),
        3,
        &["1.1.0"],
        vec![
            release_json("1.0.0", b"old"),
            release_json("1.1.0", b"bad"),
            release_json("1.1.1", b"fix"),
        ],
    )
}

fn installed_old() -> (tempfile::TempDir, TrustStore, InstalledApp) {
    let first = app_state(&key(), 1, &[], vec![release_json("1.0.0", b"old")]);
    installed_from(&first)
}

#[test]
fn a_remembered_state_stops_an_older_one() {
    let (_dir, mut store, installed) = installed_old();
    let older = app_state(&key(), 1, &[], vec![release_json("1.0.0", b"old")]);
    store.observe(&older).unwrap();
    // The state with a higher number is seen once.
    store.observe(&state_two()).unwrap();
    // A thief cannot edit it, so replays an old signed state, with the bad release not withdrawn.
    let replay = app_state(
        &key(),
        1,
        &[],
        vec![release_json("1.0.0", b"old"), release_json("1.1.0", b"bad")],
    );
    assert_eq!(
        check_update(
            &installed,
            store.get(APP).unwrap(),
            &replay,
            &profile(),
            STABLE
        ),
        UpdateStatus::Blocked(TrustProblem::StateRolledBack)
    );
    assert_eq!(
        plan_install(&replay, &store, Some(&installed), &profile(), STABLE).unwrap_err(),
        InstallRefusal::Untrusted(TrustProblem::StateRolledBack)
    );
    assert!(matches!(
        store.observe(&replay),
        Ok(Observed::Mismatch(TrustProblem::StateRolledBack))
    ));
}

#[test]
fn what_a_static_manifest_cannot_prevent() {
    // Not a behaviour to want, but one to state: a companion that never saw the withdrawal cannot
    // know about it. An old state is genuinely signed, so a host can serve it to a companion that
    // last read before the author withdrew the release, and the bad release is offered.
    let (_dir, store, installed) = installed_old();
    let bad_state = state_one();
    let status = check_update(
        &installed,
        store.get(APP).unwrap(),
        &bad_state,
        &profile(),
        STABLE,
    );
    let UpdateStatus::UpdateAvailable { release, .. } = status else {
        panic!("{status:?}");
    };
    assert_eq!(release.version().to_string(), "1.1.0");
    // A fresh install too: the first install trusts what it is shown.
    let ticket = plan_install(&bad_state, &empty_store().1, None, &profile(), STABLE).unwrap();
    assert_eq!(ticket.release().version().to_string(), "1.1.0");
}

#[test]
fn a_withdrawal_once_seen_stays_even_if_a_later_state_omits_it() {
    let (_dir, mut store, installed) = installed_old();
    store.observe(&state_two()).unwrap();
    // A later genuine state forgets to list it, by mistake or under a stolen key.
    let later = app_state(
        &key(),
        4,
        &[],
        vec![release_json("1.0.0", b"old"), release_json("1.1.0", b"bad")],
    );
    store.observe(&later).unwrap();
    let status = check_update(
        &installed,
        store.get(APP).unwrap(),
        &later,
        &profile(),
        STABLE,
    );
    assert!(
        matches!(status, UpdateStatus::UpToDate { .. }),
        "{status:?}"
    );
    assert_eq!(
        plan_install(&later, &store, Some(&installed), &profile(), STABLE).unwrap_err(),
        InstallRefusal::NothingToOffer
    );
}

#[test]
fn up_to_date_says_about_withdrawals_and_blocked_releases() {
    let first = app_state(&key(), 1, &[], vec![release_json("1.0.0", b"old")]);
    let (_dir, store, installed) = installed_from(&first);
    let mut newer = release_json("2.0.0", b"new");
    newer["min_api"] = json!(5);
    let app = app_state(
        &key(),
        2,
        &["1.0.0"],
        vec![release_json("1.0.0", b"old"), newer],
    );
    let status = check_update(
        &installed,
        store.get(APP).unwrap(),
        &app,
        &profile(),
        STABLE,
    );
    let two = Version::new(2, 0, 0);
    assert_eq!(
        status,
        UpdateStatus::UpToDate {
            installed_withdrawn: true,
            newer_skipped: vec![(&two, SkipReason::NeedsNewerGlass { min_api: 5 })],
        }
    );
}

#[test]
fn two_states_with_one_number_are_refused() {
    let first = app_state(&key(), 5, &[], vec![release_json("1.0.0", b"old")]);
    let (_dir, mut store, installed) = installed_from(&first);
    let fork = app_state(
        &key(),
        5,
        &[],
        vec![
            release_json("1.0.0", b"old"),
            release_json("9.0.0", b"evil"),
        ],
    );
    assert_eq!(
        check_update(
            &installed,
            store.get(APP).unwrap(),
            &fork,
            &profile(),
            STABLE
        ),
        UpdateStatus::Blocked(TrustProblem::StateForked)
    );
    assert_eq!(
        plan_install(&fork, &store, Some(&installed), &profile(), STABLE).unwrap_err(),
        InstallRefusal::Untrusted(TrustProblem::StateForked)
    );
    assert_eq!(
        store.observe(&fork).unwrap(),
        Observed::Mismatch(TrustProblem::StateForked)
    );
    // Recording the forked release with a ticket planned elsewhere is refused too.
    let (_other, elsewhere) = empty_store();
    let ticket = plan_install(&fork, &elsewhere, None, &profile(), STABLE).unwrap();
    assert!(matches!(
        store.record_install(&ticket, &verified(&ticket)),
        Err(TrustError::Problem(TrustProblem::StateForked))
    ));
}

#[test]
fn a_ticket_planned_without_the_memory_cannot_be_recorded_against_it() {
    // The attack from the review: plan against an empty memory to skip the floor and the
    // withdrawals, then record into the real one.
    let first = app_state(
        &key(),
        2,
        &["1.5.0"],
        vec![release_json("1.5.0", b"bad"), release_json("2.0.0", b"new")],
    );
    let (_dir, mut store, _installed) = installed_from(&first);
    assert_eq!(
        store.get(APP).unwrap().highest.as_ref().unwrap().0,
        Version::new(2, 0, 0)
    );

    let withdrawn = app_state(&key(), 3, &[], vec![release_json("1.5.0", b"bad")]);
    assert_eq!(
        plan_install(&withdrawn, &store, None, &profile(), STABLE).unwrap_err(),
        InstallRefusal::NothingToOffer
    );
    let (_other, empty) = empty_store();
    let ticket = plan_install(&withdrawn, &empty, None, &profile(), STABLE).unwrap();
    assert!(ticket.first_use());
    assert!(matches!(
        store.record_install(&ticket, &verified(&ticket)),
        Err(TrustError::Withdrawn)
    ));

    let older = app_state(&key(), 3, &[], vec![release_json("1.0.0", b"old")]);
    let ticket = plan_install(&older, &empty, None, &profile(), STABLE).unwrap();
    assert!(matches!(
        store.record_install(&ticket, &verified(&ticket)),
        Err(TrustError::BelowFloor)
    ));

    let thief = app_state(&other_key(), 9, &[], vec![release_json("9.0.0", b"evil")]);
    let ticket = plan_install(&thief, &empty, None, &profile(), STABLE).unwrap();
    assert!(matches!(
        store.record_install(&ticket, &verified(&ticket)),
        Err(TrustError::Problem(TrustProblem::PublisherChanged))
    ));
    // None of that changed the record.
    let record = store.get(APP).unwrap();
    assert_eq!(record.publisher_key, *key().verifying_key().as_bytes());
    assert_eq!(record.highest.as_ref().unwrap().0, Version::new(2, 0, 0));
}

#[test]
fn a_newer_state_seen_during_a_download_does_not_void_it_but_a_withdrawal_does() {
    let first = app_state(&key(), 1, &[], vec![release_json("1.0.0", b"old")]);
    let (_dir, mut store, installed) = installed_from(&first);
    let two = app_state(
        &key(),
        2,
        &[],
        vec![release_json("1.0.0", b"old"), release_json("1.1.0", b"new")],
    );
    let ticket = plan_install(&two, &store, Some(&installed), &profile(), STABLE).unwrap();
    // While it downloads, a refresh sees state 3, which only adds a release.
    let three = app_state(
        &key(),
        3,
        &[],
        vec![
            release_json("1.0.0", b"old"),
            release_json("1.1.0", b"new"),
            release_json("1.2.0-beta.1", b"x"),
        ],
    );
    assert_eq!(store.observe(&three).unwrap(), Observed::Updated);
    store.record_install(&ticket, &verified(&ticket)).unwrap();
    assert_eq!(store.get(APP).unwrap().sequence, 3);
    assert_eq!(
        store.get(APP).unwrap().highest.as_ref().unwrap().0,
        Version::new(1, 1, 0)
    );

    // The same, but a later state withdraws the release being downloaded.
    let five = app_state(
        &key(),
        5,
        &[],
        vec![release_json("1.1.0", b"new"), release_json("1.3.0", b"fix")],
    );
    let ticket = plan_install(&five, &store, None, &profile(), STABLE).unwrap();
    assert_eq!(ticket.release().version().to_string(), "1.3.0");
    let six = app_state(
        &key(),
        6,
        &["1.3.0"],
        vec![release_json("1.1.0", b"new"), release_json("1.3.0", b"fix")],
    );
    assert_eq!(store.observe(&six).unwrap(), Observed::Updated);
    assert!(matches!(
        store.record_install(&ticket, &verified(&ticket)),
        Err(TrustError::Withdrawn)
    ));
}

#[test]
fn another_repository_listing_the_same_id_does_not_stop_a_refresh() {
    let (_dir, mut store, _installed) =
        installed_from(&app_of(vec![release_json("1.0.0", b"old")]));
    let rival_site =
        RepositorySource::parse("https://elsewhere.example.net/omg-apps.json").unwrap();
    let mut rival = unsigned_app(APP, 50, vec![release_json("9.0.0", b"other")]);
    sign_app(&mut rival, &other_key(), rival_site.manifest_url());
    let mut unrelated = unsigned_app("com.example.other", 1, vec![release_json("1.0.0", b"x")]);
    sign_app(&mut unrelated, &other_key(), rival_site.manifest_url());
    let manifest = Manifest::parse(
        manifest_json(vec![rival, unrelated]).to_string().as_bytes(),
        &rival_site,
    )
    .unwrap();
    let results: Vec<Observed> = manifest
        .apps()
        .iter()
        .map(|app| store.observe(app).unwrap())
        .collect();
    assert_eq!(
        results,
        [
            Observed::Mismatch(TrustProblem::OtherSource),
            Observed::Unknown
        ]
    );
}

#[test]
fn two_store_instances_on_one_file_keep_each_others_changes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("trust.json");
    let mut a = TrustStore::open(&path).unwrap();
    let mut b = TrustStore::open(&path).unwrap();
    let mut c = TrustStore::open(&path).unwrap();

    let tally = app_of(vec![release_json("1.0.0", b"old")]);
    let ticket = plan_install(&tally, &a, None, &profile(), STABLE).unwrap();
    a.record_install(&ticket, &verified(&ticket)).unwrap();

    // b has not seen it, records another app, and must not erase the first.
    let mut other = unsigned_app("com.example.other", 1, vec![release_json("1.0.0", b"x")]);
    sign_app(&mut other, &key(), &repository());
    let other = parse(&manifest_json(vec![other])).unwrap().apps()[0].clone();
    let ticket = plan_install(&other, &b, None, &profile(), STABLE).unwrap();
    b.record_install(&ticket, &verified(&ticket)).unwrap();
    assert!(b.get(APP).is_some(), "b read the file again under the lock");

    // c's view is still empty, so it plans the thief's app as a first use. Recording it, under
    // the lock and against the file, is refused.
    let thief = app_state(&other_key(), 9, &[], vec![release_json("9.0.0", b"evil")]);
    let ticket = plan_install(&thief, &c, None, &profile(), STABLE).unwrap();
    assert!(ticket.first_use());
    assert!(matches!(
        c.record_install(&ticket, &verified(&ticket)),
        Err(TrustError::Problem(TrustProblem::PublisherChanged))
    ));

    let reopened = TrustStore::open(&path).unwrap();
    assert_eq!(
        reopened.get(APP).unwrap().publisher_key,
        *key().verifying_key().as_bytes()
    );
    assert!(reopened.get("com.example.other").is_some());
}

#[cfg(unix)]
#[test]
fn a_failed_write_changes_nothing() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("trust.json");
    let mut store = TrustStore::open(&path).unwrap();
    let tally = app_of(vec![release_json("1.0.0", b"old")]);
    let ticket = plan_install(&tally, &store, None, &profile(), STABLE).unwrap();
    store.record_install(&ticket, &verified(&ticket)).unwrap();
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );

    // The directory becomes read-only: the new file cannot be created.
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o500)).unwrap();
    let result = store.forget(APP);
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    if result.is_ok() {
        // Running as root: permissions do not stop it, so there is nothing to check here.
        return;
    }
    assert!(
        store.get(APP).is_some(),
        "memory is unchanged after a failed write"
    );
    assert!(TrustStore::open(&path).unwrap().get(APP).is_some());
}

#[cfg(unix)]
#[test]
fn a_planted_link_is_not_followed() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("trust.json");
    let target = dir.path().join("elsewhere");
    std::os::unix::fs::symlink(&target, dir.path().join("trust.json.lock")).unwrap();
    let mut store = TrustStore::open(&path).unwrap();
    assert!(matches!(store.forget(APP), Err(TrustError::Io(_))));
    assert!(!target.exists());
}

#[test]
fn uninstalling_forgets_nothing_and_the_memory_survives_a_restart() {
    let path;
    let dir = tempfile::tempdir().unwrap();
    {
        path = dir.path().join("trust.json");
        let mut store = TrustStore::open(&path).unwrap();
        let first = app_state(
            &key(),
            2,
            &["1.5.0"],
            vec![release_json("1.5.0", b"bad"), release_json("2.0.0", b"new")],
        );
        let ticket = plan_install(&first, &empty_store().1, None, &profile(), STABLE).unwrap();
        store.record_install(&ticket, &verified(&ticket)).unwrap();
    }
    // The app is uninstalled: nothing about the store changes. The companion restarts.
    let mut store = TrustStore::open(&path).unwrap();
    let record = store.get(APP).unwrap();
    assert_eq!(record.highest.as_ref().unwrap().0, Version::new(2, 0, 0));
    assert_eq!(record.withdrawn, [Version::new(1, 5, 0)]);

    // A thief's key is refused on the reinstall, with no installed app to compare with.
    let thief = app_state(&other_key(), 9, &[], vec![release_json("9.0.0", b"evil")]);
    assert_eq!(
        plan_install(&thief, &store, None, &profile(), STABLE).unwrap_err(),
        InstallRefusal::Untrusted(TrustProblem::PublisherChanged)
    );
    // A real but older release is under the floor, even in a higher genuine state.
    let older = app_state(&key(), 3, &[], vec![release_json("1.0.0", b"old")]);
    assert_eq!(
        plan_install(&older, &store, None, &profile(), STABLE).unwrap_err(),
        InstallRefusal::NothingToOffer
    );
    assert_eq!(
        select_release(&older, store.get(APP), &profile(), STABLE).skipped[0].1,
        SkipReason::BelowInstalledFloor
    );
    // The same version may come back, without being a first use.
    let same = app_state(&key(), 3, &[], vec![release_json("2.0.0", b"new")]);
    let ticket = plan_install(&same, &store, None, &profile(), STABLE).unwrap();
    assert!(!ticket.first_use());
    // The same version with other bytes may not.
    let other_bytes = app_state(&key(), 3, &[], vec![release_json("2.0.0", b"evil")]);
    assert_eq!(
        plan_install(&other_bytes, &store, None, &profile(), STABLE).unwrap_err(),
        InstallRefusal::Untrusted(TrustProblem::ReleaseChanged)
    );

    // Only the owner's explicit choice clears it, and it is a first use again.
    store.forget(APP).unwrap();
    let store = TrustStore::open(&path).unwrap();
    assert!(store.get(APP).is_none());
    let ticket = plan_install(&thief, &store, None, &profile(), STABLE).unwrap();
    assert!(ticket.first_use());
}

#[test]
fn a_damaged_memory_is_an_error_not_an_empty_one() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("trust.json");
    assert!(TrustStore::open(&path).unwrap().get(APP).is_none());
    for bad in [
        &b"not json"[..],
        br#"{"schema":2,"apps":[]}"#,
        br#"{"schema":1}"#,
    ] {
        std::fs::write(&path, bad).unwrap();
        assert!(matches!(TrustStore::open(&path), Err(TrustError::Corrupt)));
    }
}

#[test]
fn an_installed_app_with_no_record_is_refused() {
    let app = app_of(vec![release_json("1.0.0", b"old")]);
    let installed = InstalledApp {
        id: APP.to_owned(),
        version: Version::new(0, 9, 0),
        sha256: sha(b"x"),
        permissions: vec![],
    };
    assert_eq!(
        plan_install(&app, &empty_store().1, Some(&installed), &profile(), STABLE).unwrap_err(),
        InstallRefusal::InstalledWithoutRecord
    );
}

#[test]
fn observing_an_unknown_app_remembers_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = TrustStore::open(dir.path().join("trust.json")).unwrap();
    store.observe(&state_one()).unwrap();
    assert!(store.get(APP).is_none());
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
    /// Sends one byte at a time, each after a pause.
    Drip,
    /// Cancels the control on its first read, then sends these bytes.
    CancelsThen(FetchControl, Vec<u8>),
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

struct Drip;

impl Read for Drip {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        std::thread::sleep(Duration::from_millis(5));
        buffer[0] = b'.';
        Ok(1)
    }
}

struct CancelsThen(FetchControl, Cursor<Vec<u8>>);

impl Read for CancelsThen {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.0.cancel();
        // Less than the whole package, so the loop has to come back for more.
        let room = buffer.len().min(1);
        self.1.read(&mut buffer[..room])
    }
}

impl Fetcher for TableFetcher {
    fn get(&self, url: &HttpsUrl, _control: &FetchControl) -> io::Result<FetchResponse> {
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
            Some(Canned::Drip) => FetchResponse::Body(Box::new(Drip)),
            Some(Canned::CancelsThen(control, bytes)) => {
                FetchResponse::Body(Box::new(CancelsThen(control, Cursor::new(bytes))))
            }
        })
    }
}

const PACKAGE: &[u8] = b"a package, say";
const PACKAGE_URL: &str = "https://example.org/glass/tally-1.0.0.omgapp";

fn package_app() -> AppEntry {
    app_of(vec![release_json("1.0.0", PACKAGE)])
}

fn ticket_for(app: &AppEntry) -> InstallTicket<'_> {
    plan_install(app, &empty_store().1, None, &profile(), STABLE).unwrap()
}

fn long_enough() -> FetchControl {
    FetchControl::with_timeout(Duration::from_secs(60))
}

fn files(dir: &std::path::Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .filter(|name| name != ".lock")
        .collect();
    names.sort();
    names
}

/// Runs a download into a fresh store, returning the result and what is on disk.
fn download_with(
    fetcher: &TableFetcher,
    control: &FetchControl,
) -> (Result<VerifiedPackage, DownloadError>, Vec<String>) {
    let dir = tempfile::tempdir().unwrap();
    let store = PackageStore::open(dir.path()).unwrap();
    let app = package_app();
    let ticket = ticket_for(&app);
    let mut sink = store.begin(ticket.release().sha256()).unwrap();
    let result = fetch_package(fetcher, &ticket, &mut sink, control);
    drop(sink);
    (result, files(dir.path()))
}

fn download(fetcher: &TableFetcher) -> (Result<VerifiedPackage, DownloadError>, Vec<String>) {
    download_with(fetcher, &long_enough())
}

#[test]
fn a_good_download_lands_under_its_hash_with_what_the_glass_needs() {
    let dir = tempfile::tempdir().unwrap();
    let store = PackageStore::open(dir.path()).unwrap();
    let app = package_app();
    let ticket = ticket_for(&app);
    let fetcher = TableFetcher::default().with(PACKAGE_URL, Canned::Body(PACKAGE.to_vec()));
    let mut sink = store.begin(ticket.release().sha256()).unwrap();
    let package = fetch_package(&fetcher, &ticket, &mut sink, &long_enough()).unwrap();
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

    // An AppOffer can be built: the Glass can check this with no manifest.
    let glass_side = ed25519_dalek::VerifyingKey::from_bytes(package.publisher_key()).unwrap();
    glass_side
        .verify(
            package.statement(),
            &Signature::from_bytes(package.signature()),
        )
        .unwrap();

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
fn a_slow_server_runs_out_of_time_and_leaves_nothing_behind() {
    let (result, left) = download_with(
        &TableFetcher::default().with(PACKAGE_URL, Canned::Drip),
        &FetchControl::with_timeout(Duration::from_millis(30)),
    );
    assert!(matches!(result, Err(DownloadError::DeadlineExceeded)));
    assert!(left.is_empty(), "{left:?}");

    let fetcher = TableFetcher::default().with(PACKAGE_URL, Canned::Body(PACKAGE.to_vec()));
    let (result, _) = download_with(&fetcher, &FetchControl::with_timeout(Duration::ZERO));
    assert!(matches!(result, Err(DownloadError::DeadlineExceeded)));
    assert!(fetcher.asked().is_empty(), "no request once out of time");
}

#[test]
fn a_download_can_be_cancelled() {
    let control = long_enough();
    let fetcher = TableFetcher::default().with(
        PACKAGE_URL,
        Canned::CancelsThen(control.clone(), PACKAGE.to_vec()),
    );
    let (result, left) = download_with(&fetcher, &control);
    assert!(matches!(result, Err(DownloadError::Cancelled)));
    assert!(left.is_empty(), "{left:?}");
}

#[test]
fn the_usual_deadlines_grow_with_the_size() {
    let small = FetchControl::for_package(1).remaining();
    let big = FetchControl::for_package(MAX_PACKAGE_BYTES).remaining();
    assert!(small <= Duration::from_secs(60));
    assert!(big > Duration::from_secs(4000) && big <= Duration::from_secs(4156));
    assert!(FetchControl::for_manifest().remaining() <= Duration::from_secs(60));
}

#[test]
fn concurrent_downloads_of_one_hash_cannot_corrupt_each_other() {
    let dir = tempfile::tempdir().unwrap();
    let store = PackageStore::open(dir.path()).unwrap();
    let app = package_app();
    let ticket = ticket_for(&app);
    let hash = *ticket.release().sha256();
    let fetcher = TableFetcher::default().with(PACKAGE_URL, Canned::Body(PACKAGE.to_vec()));
    let hostile_bytes = vec![b'X'; PACKAGE.len()];

    for _ in 0..50 {
        let barrier = std::sync::Barrier::new(2);
        std::thread::scope(|scope| {
            // A hostile stream for the same hash writes the wrong bytes and tries to commit them,
            // at the same moment as a good download commits.
            let hostile = scope.spawn(|| {
                let mut sink = store.begin(&hash).unwrap();
                sink.write(&hostile_bytes).unwrap();
                barrier.wait();
                sink.commit()
            });
            let good = scope.spawn(|| {
                let mut sink = store.begin(&hash).unwrap();
                barrier.wait();
                fetch_package(&fetcher, &ticket, &mut sink, &long_enough())
            });
            assert_eq!(
                hostile.join().unwrap().unwrap_err().kind(),
                io::ErrorKind::InvalidData
            );
            good.join().unwrap().unwrap();
        });
        assert_eq!(std::fs::read(store.path_of(&hash)).unwrap(), PACKAGE);
        assert_eq!(files(dir.path()).len(), 1, "{:?}", files(dir.path()));
    }
}

#[cfg(unix)]
#[test]
fn a_failed_rename_leaves_no_part_behind() {
    let dir = tempfile::tempdir().unwrap();
    let store = PackageStore::open(dir.path()).unwrap();
    let hash = sha(PACKAGE);
    // Something that is not a file sits where the package would go.
    std::fs::create_dir(store.path_of(&hash)).unwrap();
    std::fs::write(store.path_of(&hash).join("x"), b"x").unwrap();
    let mut sink = store.begin(&hash).unwrap();
    sink.write(PACKAGE).unwrap();
    assert!(sink.commit().is_err());
    drop(sink);
    assert!(
        !files(dir.path()).iter().any(|f| f.ends_with(".part")),
        "{:?}",
        files(dir.path())
    );
}

#[test]
fn commit_checks_the_file_itself() {
    let dir = tempfile::tempdir().unwrap();
    let store = PackageStore::open(dir.path()).unwrap();
    let hash = sha(PACKAGE);

    let mut wrong = store.begin(&hash).unwrap();
    wrong.write(b"something else").unwrap();
    assert_eq!(
        wrong.commit().unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
    assert!(!store.path_of(&hash).exists());
    wrong.abort();
    assert!(files(dir.path()).is_empty());

    let mut right = store.begin(&hash).unwrap();
    for chunk in PACKAGE.chunks(3) {
        right.write(chunk).unwrap();
    }
    right.commit().unwrap();
    assert_eq!(std::fs::read(store.path_of(&hash)).unwrap(), PACKAGE);
}

#[test]
fn parts_left_by_a_crash_are_cleared_only_when_no_download_can_run() {
    let dir = tempfile::tempdir().unwrap();
    let hash = sha(PACKAGE);
    let crashed = dir
        .path()
        .join(format!("{}.0011223344556677.part", to_hex(&hash)));
    std::fs::write(&crashed, vec![b'!'; 1000]).unwrap();
    let store = PackageStore::open(dir.path()).unwrap();
    assert!(!crashed.exists());

    // While a store is open, nobody else can open the directory, so nobody can clear its parts.
    let running = store.begin(&hash).unwrap();
    assert_eq!(
        PackageStore::open(dir.path()).err().map(|e| e.kind()),
        Some(io::ErrorKind::WouldBlock)
    );
    assert_eq!(files(dir.path()).len(), 1);
    drop(running);
    drop(store);
    assert!(PackageStore::open(dir.path()).is_ok());
}

#[test]
fn the_verifier_alone_gives_the_same_answers() {
    let app = package_app();
    let ticket = ticket_for(&app);
    let mut verifier = DownloadVerifier::new(&ticket);
    for chunk in PACKAGE.chunks(3) {
        verifier.push(chunk).unwrap();
    }
    assert_eq!(*verifier.finish().unwrap().sha256(), sha(PACKAGE));

    let mut verifier = DownloadVerifier::new(&ticket);
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
    let count = Arc::new(AtomicU64::new(0));
    let fetcher = TableFetcher::default().with(MANIFEST_URL, Canned::Endless(count.clone()));
    assert!(matches!(
        fetch_manifest(&fetcher, &source(), &long_enough()),
        Err(DownloadError::TooLarge)
    ));
    assert!(count.load(Ordering::Relaxed) <= MAX_MANIFEST_BYTES as u64 + 8 * 1024);

    let fetcher = TableFetcher::default().with(MANIFEST_URL, Canned::Body(b"<html>".to_vec()));
    assert!(matches!(
        fetch_manifest(&fetcher, &source(), &long_enough()),
        Err(DownloadError::Manifest(ManifestError::Malformed { .. }))
    ));
    assert!(matches!(
        fetch_manifest(&TableFetcher::default(), &source(), &long_enough()),
        Err(DownloadError::Status(404))
    ));
    let slow = TableFetcher::default().with(MANIFEST_URL, Canned::Drip);
    assert!(matches!(
        fetch_manifest(
            &slow,
            &source(),
            &FetchControl::with_timeout(Duration::from_millis(30))
        ),
        Err(DownloadError::DeadlineExceeded)
    ));
}

#[test]
fn a_manifest_may_not_redirect_to_another_site() {
    // An open redirect on a trusted host must not put someone else's manifest under its name.
    let evil = "https://evil.example.net/omg-apps.json";
    let fetcher = TableFetcher::default()
        .with(MANIFEST_URL, Canned::Redirect(evil))
        .with(evil, Canned::Body(one_release().to_string().into_bytes()));
    assert!(matches!(
        fetch_manifest(&fetcher, &source(), &long_enough()),
        Err(DownloadError::CrossOriginManifest)
    ));
    assert_eq!(fetcher.asked(), [MANIFEST_URL]);

    let other_port = "https://example.org:8443/omg-apps.json";
    let fetcher = TableFetcher::default().with(MANIFEST_URL, Canned::Redirect(other_port));
    assert!(matches!(
        fetch_manifest(&fetcher, &source(), &long_enough()),
        Err(DownloadError::CrossOriginManifest)
    ));

    // Inside the site it is fine.
    let moved = "/new/omg-apps.json";
    let fetcher = TableFetcher::default()
        .with(MANIFEST_URL, Canned::Redirect(moved))
        .with(
            "https://example.org/new/omg-apps.json",
            Canned::Body(one_release().to_string().into_bytes()),
        );
    assert!(fetch_manifest(&fetcher, &source(), &long_enough()).is_ok());
}

// ------------------------------- the whole path -------------------------------

#[test]
fn add_a_github_repository_install_then_update_then_survive_a_takeover() {
    assert!(GITHUB_ASSET_REDIRECT.len() > 900);
    let raw = "https://raw.githubusercontent.com/some-one/tally/HEAD/omg-apps.json";
    let v1 = b"old".as_slice();
    let v2 = b"new".as_slice();
    let url_of =
        |v: &str| format!("https://github.com/Some-One/tally/releases/download/v{v}/t.omgapp");

    // Add the repository, the way the owner pastes it.
    let source = RepositorySource::parse("https://github.com/Some-One/Tally").unwrap();
    assert_eq!(source.manifest_url().as_str(), raw);
    assert_eq!(source.label(), "github.com/some-one/tally");

    let release = |v: &str, bytes: &[u8]| {
        let mut r = release_json(v, bytes);
        r["url"] = json!(url_of(v));
        r
    };
    let published = |signer: &SigningKey, sequence: u64, releases: Vec<Value>| {
        let mut app = unsigned_app(APP, sequence, releases);
        sign_app(&mut app, signer, source.manifest_url());
        manifest_json(vec![app]).to_string().into_bytes()
    };

    let dir = tempfile::tempdir().unwrap();
    let packages = PackageStore::open(dir.path().join("packages")).unwrap();
    let mut trust = TrustStore::open(dir.path().join("trust.json")).unwrap();

    // Install, behind the kind of redirect GitHub really answers with.
    let fetcher = TableFetcher::default()
        .with(
            raw,
            Canned::Body(published(&key(), 1, vec![release("1.0.0", v1)])),
        )
        .with(&url_of("1.0.0"), Canned::Redirect(GITHUB_ASSET_REDIRECT))
        .with(GITHUB_ASSET_REDIRECT, Canned::Body(v1.to_vec()));
    let manifest = fetch_manifest(&fetcher, &source, &FetchControl::for_manifest()).unwrap();
    let app = manifest.app(APP).unwrap();
    let ticket = plan_install(app, &trust, None, &profile(), STABLE).unwrap();
    assert!(ticket.first_use());
    let mut sink = packages.begin(ticket.release().sha256()).unwrap();
    let control = FetchControl::for_package(ticket.release().size());
    let package = fetch_package(&fetcher, &ticket, &mut sink, &control).unwrap();
    assert_eq!(fetcher.asked().last().unwrap(), GITHUB_ASSET_REDIRECT);
    trust.record_install(&ticket, &package).unwrap();
    let installed = installed_of(&ticket);

    // A new version appears, the same way.
    let fetcher = TableFetcher::default()
        .with(
            raw,
            Canned::Body(published(
                &key(),
                2,
                vec![release("1.0.0", v1), release("1.1.0", v2)],
            )),
        )
        .with(&url_of("1.1.0"), Canned::Redirect(GITHUB_ASSET_REDIRECT))
        .with(GITHUB_ASSET_REDIRECT, Canned::Body(v2.to_vec()));
    let manifest = fetch_manifest(&fetcher, &source, &FetchControl::for_manifest()).unwrap();
    let app = manifest.app(APP).unwrap();
    trust.observe(app).unwrap();
    let ticket = plan_install(app, &trust, Some(&installed), &profile(), STABLE).unwrap();
    let mut sink = packages.begin(ticket.release().sha256()).unwrap();
    let updated = fetch_package(&fetcher, &ticket, &mut sink, &long_enough()).unwrap();
    assert_eq!(*updated.version(), Version::new(1, 1, 0));
    assert_eq!(
        std::fs::read(packages.path_of(updated.sha256())).unwrap(),
        v2
    );
    trust.record_install(&ticket, &updated).unwrap();
    let installed = installed_of(&ticket);

    // The account is stolen: a valid manifest under the thief's key, malicious "update" included.
    let fetcher = TableFetcher::default().with(
        raw,
        Canned::Body(published(&other_key(), 99, vec![release("9.9.9", b"evil")])),
    );
    let manifest = fetch_manifest(&fetcher, &source, &FetchControl::for_manifest()).unwrap();
    let stolen = manifest.app(APP).unwrap();
    assert!(matches!(
        trust.observe(stolen),
        Ok(Observed::Mismatch(TrustProblem::PublisherChanged))
    ));
    assert_eq!(
        check_update(
            &installed,
            trust.get(APP).unwrap(),
            stolen,
            &profile(),
            STABLE
        ),
        UpdateStatus::Blocked(TrustProblem::PublisherChanged)
    );
    assert_eq!(
        plan_install(stolen, &trust, Some(&installed), &profile(), STABLE).unwrap_err(),
        InstallRefusal::Untrusted(TrustProblem::PublisherChanged)
    );
}
