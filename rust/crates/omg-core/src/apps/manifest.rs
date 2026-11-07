//! The repository manifest: untrusted JSON from anywhere on the internet.
//!
//! [`Manifest::parse`] is all-or-nothing. It returns a value only when every field of every app
//! and release is well formed and every signature verifies; anything else refuses the whole file.
//! Errors name the position and the field, and never repeat text from the file, since they can end
//! up on a screen. The format is in `docs/plans/2026-11-07-companion-applications.md`.
//!
//! Two kinds of signature, both by the app's publisher key:
//!
//! * A release signature covers what one package is. The Glass can check it alone.
//! * An app state signature covers everything else the author says about the app at one moment:
//!   this repository, a sequence number, name, summary, licence, homepage, withdrawn versions, and
//!   the exact set of releases. Without the key a host cannot edit any of that. It can still serve
//!   an older signed state, which only a client's memory can catch: see [`super::trust`].

use std::collections::HashSet;

use ed25519_dalek::{Signature, VerifyingKey};
use semver::Version;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use thiserror::Error;

use super::source::RepositorySource;
use super::url::{HttpsUrl, UrlError};

/// Largest manifest file, in bytes. Checked before the JSON parser sees anything.
pub const MAX_MANIFEST_BYTES: usize = 512 * 1024;
/// Most apps in one repository.
pub const MAX_APPS: usize = 32;
/// Most releases of one app. Bounds the signature checks of one manifest to 32 * 17 = 544.
pub const MAX_RELEASES_PER_APP: usize = 16;
/// Most withdrawn versions of one app. They may name releases no longer listed.
pub const MAX_WITHDRAWN: usize = 64;
/// Most permissions of one release.
pub const MAX_PERMISSIONS: usize = 16;
/// Largest package. At the measured phone-to-Glass Bluetooth rate of about 6 kB/s this is about
/// 3 hours, so a big package needs the Wi-Fi path: see the design doc.
pub const MAX_PACKAGE_BYTES: u64 = 64 * 1024 * 1024;

const MAX_ID_LEN: usize = 64;
const MAX_NAME_LEN: usize = 64;
const MAX_SUMMARY_LEN: usize = 280;
const MAX_LICENSE_LEN: usize = 64;
const MAX_VERSION_LEN: usize = 48;
const MAX_TARGET_LEN: usize = 32;
const MAX_PERMISSION_LEN: usize = 48;

/// This id and those under it belong to the project and are never accepted from a repository.
const RESERVED_ID: &str = "org.openmyglass";

const SCHEMA_VERSION: u32 = 1;

/// Domain separation of the signed statement of a release.
const STATEMENT_DOMAIN: &[u8] = b"openmyglass-app-release-v1\0";
/// Domain separation of the signed state of an app.
const STATE_DOMAIN: &[u8] = b"openmyglass-app-state-v1\0";

/// A SHA-256 digest.
pub type Sha256Digest = [u8; 32];

/// Why a manifest was refused.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ManifestError {
    /// Bigger than [`MAX_MANIFEST_BYTES`].
    #[error("manifest is larger than the limit")]
    TooLarge,
    /// Not valid JSON, an unknown or duplicate field, or a wrong type.
    #[error("manifest is malformed near line {line}, column {column}")]
    Malformed {
        /// 1-based line.
        line: usize,
        /// 1-based column.
        column: usize,
    },
    /// Written for a format this companion does not know.
    #[error("manifest format {0} is not supported, the companion may need an update")]
    UnsupportedSchema(u32),
    /// More apps than [`MAX_APPS`], or none.
    #[error("manifest has a bad number of apps")]
    BadAppCount,
    /// A top-level field is invalid.
    #[error("manifest field `{0}` is invalid")]
    BadField(&'static str),
    /// A field of an app is invalid.
    #[error("app {app}: field `{field}` is invalid")]
    BadAppField {
        /// Position of the app in the file.
        app: usize,
        /// The field.
        field: &'static str,
    },
    /// A URL of an app is invalid.
    #[error("app {app}: field `{field}`: {error}")]
    BadAppUrl {
        /// Position of the app in the file.
        app: usize,
        /// The field.
        field: &'static str,
        /// Why.
        error: UrlError,
    },
    /// An app id under the project's reserved prefix.
    #[error("app {0}: the id uses a reserved prefix")]
    ReservedId(usize),
    /// The same app id twice in the file.
    #[error("app {0}: the id appears twice")]
    DuplicateApp(usize),
    /// More releases than [`MAX_RELEASES_PER_APP`], or none.
    #[error("app {0}: bad number of releases")]
    BadReleaseCount(usize),
    /// A field of a release is invalid.
    #[error("app {app}, release {release}: field `{field}` is invalid")]
    BadReleaseField {
        /// Position of the app in the file.
        app: usize,
        /// Position of the release in the app.
        release: usize,
        /// The field.
        field: &'static str,
    },
    /// A release's download URL is invalid.
    #[error("app {app}, release {release}: url: {error}")]
    BadReleaseUrl {
        /// Position of the app in the file.
        app: usize,
        /// Position of the release in the app.
        release: usize,
        /// Why.
        error: UrlError,
    },
    /// The same version twice in an app: the file would make two claims about one version.
    #[error("app {app}, release {release}: the version appears twice")]
    DuplicateVersion {
        /// Position of the app in the file.
        app: usize,
        /// Position of the release in the app.
        release: usize,
    },
    /// The signature does not verify against the app's publisher key.
    #[error("app {app}, release {release}: bad signature")]
    BadSignature {
        /// Position of the app in the file.
        app: usize,
        /// Position of the release in the app.
        release: usize,
    },
    /// The app state signature does not verify: its text, withdrawn versions or set of releases
    /// were edited, or it was signed for another repository.
    #[error("app {0}: bad state signature")]
    BadStateSignature(usize),
}

/// A checked repository manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    /// The repository's display name. **Not signed**: the host can write anything here, so the UI
    /// shows it as unverified, below the repository's label.
    pub name: String,
    /// Its apps, in file order.
    pub apps: Vec<AppEntry>,
}

/// One app of a repository, with everything in it verified against `publisher_key`. That only says
/// the holder of that key wrote it: on a first install, nothing yet says who that is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppEntry {
    /// Reverse-DNS id, also a safe path component on the Glass.
    pub id: String,
    /// The manifest URL this was read from and signed for.
    pub repository: HttpsUrl,
    /// Display name, signed. Whoever holds the key chooses it, so the UI still shows the
    /// repository's label beside it.
    pub name: String,
    /// One line of description, signed.
    pub summary: String,
    /// Licence, as the author states it, signed.
    pub license: String,
    /// The author's page, if any, signed.
    pub homepage: Option<HttpsUrl>,
    /// The ed25519 key that signs this app.
    pub publisher_key: [u8; 32],
    /// Goes up with every state the author publishes.
    pub sequence: u64,
    /// Versions the author withdrew, sorted.
    pub withdrawn: Vec<Version>,
    /// The signature over [`app_state_statement`].
    pub state_signature: [u8; 64],
    /// Releases, in file order.
    pub releases: Vec<Release>,
}

impl AppEntry {
    /// Whether the author withdrew this version in this state.
    pub fn is_withdrawn(&self, version: &Version) -> bool {
        self.withdrawn.contains(version)
    }

    /// Identifies this state, to notice two different states with one sequence number.
    pub fn state_digest(&self) -> Sha256Digest {
        Sha256::digest(app_state_statement(self)).into()
    }
}

/// One release of an app.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    /// The version. Never has build metadata, so equal versions have one meaning.
    pub version: Version,
    /// What the package runs on. PROVISIONAL: depends on the SDK decision.
    pub target: String,
    /// Lowest Glass app API it needs. PROVISIONAL: depends on the SDK.
    pub min_api: u32,
    /// Where to download the package. **The only unsigned field of an app**, so a package can move
    /// host. A wrong one can make a download fail, not change what is installed.
    pub url: HttpsUrl,
    /// Exact size of the package.
    pub size: u64,
    /// SHA-256 of the package.
    pub sha256: Sha256Digest,
    /// What the app asks for, sorted. Syntax is checked here; see [`super::select`] for meaning.
    pub permissions: Vec<String>,
    /// The publisher's signature over [`release_statement`], kept for the Glass to check again.
    pub signature: [u8; 64],
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawManifest {
    schema: u32,
    name: String,
    apps: Vec<RawApp>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawApp {
    id: String,
    name: String,
    summary: String,
    license: String,
    #[serde(default)]
    homepage: Option<String>,
    publisher_key: String,
    sequence: u64,
    #[serde(default)]
    withdrawn: Vec<String>,
    state_signature: String,
    releases: Vec<RawRelease>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRelease {
    version: String,
    target: String,
    min_api: u32,
    url: String,
    size: u64,
    sha256: String,
    permissions: Vec<String>,
    signature: String,
}

/// Only `schema` is read first, so that a newer file gets "update the companion" and not a
/// confusing complaint about the first field this version does not know.
#[derive(Deserialize)]
struct SchemaOnly {
    schema: u32,
}

impl Manifest {
    /// Parses and checks a manifest read from `source`. An app's state must be signed for that
    /// repository, so a copy of someone's manifest on another site is refused. Never panics,
    /// whatever the input.
    pub fn parse(bytes: &[u8], source: &RepositorySource) -> Result<Self, ManifestError> {
        if bytes.len() > MAX_MANIFEST_BYTES {
            return Err(ManifestError::TooLarge);
        }
        let malformed = |e: serde_json::Error| ManifestError::Malformed {
            line: e.line(),
            column: e.column(),
        };
        let schema = serde_json::from_slice::<SchemaOnly>(bytes)
            .map_err(malformed)?
            .schema;
        if schema != SCHEMA_VERSION {
            return Err(ManifestError::UnsupportedSchema(schema));
        }
        let raw: RawManifest = serde_json::from_slice(bytes).map_err(malformed)?;
        debug_assert_eq!(raw.schema, SCHEMA_VERSION);

        if !good_name(&raw.name) {
            return Err(ManifestError::BadField("name"));
        }
        if raw.apps.is_empty() || raw.apps.len() > MAX_APPS {
            return Err(ManifestError::BadAppCount);
        }
        let mut seen = HashSet::new();
        let mut apps = Vec::with_capacity(raw.apps.len());
        for (index, app) in raw.apps.into_iter().enumerate() {
            let app = check_app(index, app, source.manifest_url())?;
            if !seen.insert(app.id.clone()) {
                return Err(ManifestError::DuplicateApp(index));
            }
            apps.push(app);
        }
        Ok(Self {
            name: raw.name,
            apps,
        })
    }

    /// The app with this id, if the repository has it.
    pub fn app(&self, id: &str) -> Option<&AppEntry> {
        self.apps.iter().find(|a| a.id == id)
    }
}

fn check_app(index: usize, raw: RawApp, repository: &HttpsUrl) -> Result<AppEntry, ManifestError> {
    let bad = |field| ManifestError::BadAppField { app: index, field };
    if !good_app_id(&raw.id) {
        return Err(bad("id"));
    }
    let reserved = raw
        .id
        .strip_prefix(RESERVED_ID)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with('.'));
    if reserved {
        return Err(ManifestError::ReservedId(index));
    }
    if !good_name(&raw.name) {
        return Err(bad("name"));
    }
    if !good_text(&raw.summary, MAX_SUMMARY_LEN) {
        return Err(bad("summary"));
    }
    if !good_text(&raw.license, MAX_LICENSE_LEN) {
        return Err(bad("license"));
    }
    let homepage = raw
        .homepage
        .as_deref()
        .map(HttpsUrl::parse)
        .transpose()
        .map_err(|error| ManifestError::BadAppUrl {
            app: index,
            field: "homepage",
            error,
        })?;
    let publisher_key: [u8; 32] = decode_hex(&raw.publisher_key).ok_or(bad("publisher_key"))?;
    let key = VerifyingKey::from_bytes(&publisher_key).map_err(|_| bad("publisher_key"))?;
    if key.is_weak() {
        return Err(bad("publisher_key"));
    }
    if raw.releases.is_empty() || raw.releases.len() > MAX_RELEASES_PER_APP {
        return Err(ManifestError::BadReleaseCount(index));
    }
    if raw.withdrawn.len() > MAX_WITHDRAWN {
        return Err(bad("withdrawn"));
    }
    let mut withdrawn = raw
        .withdrawn
        .iter()
        .map(|text| parse_version(text))
        .collect::<Option<Vec<Version>>>()
        .ok_or(bad("withdrawn"))?;
    withdrawn.sort_unstable();
    if withdrawn.windows(2).any(|w| w[0] == w[1]) {
        return Err(bad("withdrawn"));
    }
    let state_signature: [u8; 64] =
        decode_hex(&raw.state_signature).ok_or(bad("state_signature"))?;

    let mut versions = HashSet::new();
    let mut releases = Vec::with_capacity(raw.releases.len());
    for (position, raw_release) in raw.releases.into_iter().enumerate() {
        let release = check_release(index, position, raw_release)?;
        if !versions.insert(release.version.clone()) {
            return Err(ManifestError::DuplicateVersion {
                app: index,
                release: position,
            });
        }
        // verify_strict: no malleable signatures, no small-order values.
        let signature = Signature::from_bytes(&release.signature);
        key.verify_strict(&release_statement(&raw.id, &release), &signature)
            .map_err(|_| ManifestError::BadSignature {
                app: index,
                release: position,
            })?;
        releases.push(release);
    }
    let app = AppEntry {
        id: raw.id,
        repository: repository.clone(),
        name: raw.name,
        summary: raw.summary,
        license: raw.license,
        homepage,
        publisher_key,
        sequence: raw.sequence,
        withdrawn,
        state_signature,
        releases,
    };
    key.verify_strict(
        &app_state_statement(&app),
        &Signature::from_bytes(&app.state_signature),
    )
    .map_err(|_| ManifestError::BadStateSignature(index))?;
    Ok(app)
}

fn check_release(app: usize, release: usize, raw: RawRelease) -> Result<Release, ManifestError> {
    let bad = |field| ManifestError::BadReleaseField {
        app,
        release,
        field,
    };
    let version = parse_version(&raw.version).ok_or(bad("version"))?;
    if !good_target(&raw.target) {
        return Err(bad("target"));
    }
    let url = HttpsUrl::parse(&raw.url).map_err(|error| ManifestError::BadReleaseUrl {
        app,
        release,
        error,
    })?;
    if raw.size == 0 || raw.size > MAX_PACKAGE_BYTES {
        return Err(bad("size"));
    }
    let sha256: Sha256Digest = decode_hex(&raw.sha256).ok_or(bad("sha256"))?;
    if raw.permissions.len() > MAX_PERMISSIONS
        || !raw.permissions.iter().all(|p| good_permission(p))
    {
        return Err(bad("permissions"));
    }
    let mut permissions = raw.permissions;
    permissions.sort_unstable();
    if permissions.windows(2).any(|w| w[0] == w[1]) {
        return Err(bad("permissions"));
    }
    let signature: [u8; 64] = decode_hex(&raw.signature).ok_or(bad("signature"))?;
    Ok(Release {
        version,
        target: raw.target,
        min_api: raw.min_api,
        url,
        size: raw.size,
        sha256,
        permissions,
        signature,
    })
}

fn field(out: &mut Vec<u8>, bytes: &[u8]) {
    // Every field is limited far below u32::MAX by the checks above.
    out.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
    out.extend_from_slice(bytes);
}

/// The exact bytes a publisher signs for a release, and that the Glass can verify again without
/// the manifest. Length-prefixed, so no two different releases give the same bytes. Covers what
/// the package is and what it may do; the URL is left out on purpose.
pub fn release_statement(app_id: &str, release: &Release) -> Vec<u8> {
    let mut out = Vec::with_capacity(256);
    out.extend_from_slice(STATEMENT_DOMAIN);
    field(&mut out, app_id.as_bytes());
    field(&mut out, release.version.to_string().as_bytes());
    field(&mut out, release.target.as_bytes());
    out.extend_from_slice(&release.min_api.to_be_bytes());
    out.extend_from_slice(&release.size.to_be_bytes());
    out.extend_from_slice(&release.sha256);
    out.extend_from_slice(&(release.permissions.len() as u32).to_be_bytes());
    for permission in &release.permissions {
        field(&mut out, permission.as_bytes());
    }
    out
}

/// The exact bytes a publisher signs for the state of an app: the repository, the sequence number,
/// the displayed text, what is withdrawn, and the hash of each release statement in version order,
/// so a host can neither edit the text nor drop or restore a single release.
pub fn app_state_statement(app: &AppEntry) -> Vec<u8> {
    let mut out = Vec::with_capacity(512);
    out.extend_from_slice(STATE_DOMAIN);
    field(&mut out, app.id.as_bytes());
    field(&mut out, app.repository.as_str().as_bytes());
    out.extend_from_slice(&app.sequence.to_be_bytes());
    field(&mut out, app.name.as_bytes());
    field(&mut out, app.summary.as_bytes());
    field(&mut out, app.license.as_bytes());
    match &app.homepage {
        Some(homepage) => {
            out.push(1);
            field(&mut out, homepage.as_str().as_bytes());
        }
        None => out.push(0),
    }
    out.extend_from_slice(&(app.withdrawn.len() as u32).to_be_bytes());
    for version in &app.withdrawn {
        field(&mut out, version.to_string().as_bytes());
    }
    let mut releases: Vec<&Release> = app.releases.iter().collect();
    releases.sort_unstable_by(|a, b| a.version.cmp(&b.version));
    out.extend_from_slice(&(releases.len() as u32).to_be_bytes());
    for release in releases {
        out.extend_from_slice(&Sha256::digest(release_statement(&app.id, release)));
    }
    out
}

/// Strict semantic version: no build metadata, and one spelling per version (the parser refuses
/// leading zeros), so a version string identifies one thing.
pub(super) fn parse_version(text: &str) -> Option<Version> {
    if text.len() > MAX_VERSION_LEN {
        return None;
    }
    let version = Version::parse(text).ok()?;
    version.build.is_empty().then_some(version)
}

/// `com.example.my_app`: at least two dot-separated labels of lowercase letters, digits and `_`,
/// each starting with a letter. Safe as a path component and a user name on the Glass.
fn good_app_id(id: &str) -> bool {
    id.len() <= MAX_ID_LEN && {
        let mut labels = 0;
        id.split('.').all(|label| {
            labels += 1;
            label.starts_with(|c: char| c.is_ascii_lowercase())
                && label
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        }) && labels >= 2
    }
}

/// `armv7-linux-musleabihf`, `wasm32-wasip1`.
fn good_target(target: &str) -> bool {
    !target.is_empty()
        && target.len() <= MAX_TARGET_LEN
        && target.starts_with(|c: char| c.is_ascii_lowercase())
        && target
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
}

/// `location`, `input.head`: lowercase labels, syntax only.
fn good_permission(permission: &str) -> bool {
    permission.len() <= MAX_PERMISSION_LEN
        && permission.split('.').all(|label| {
            label.starts_with(|c: char| c.is_ascii_lowercase())
                && label
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        })
}

/// One line of displayable text, by an allowlist: printable ASCII, Latin-1 and Latin Extended-A
/// letters, and typographic quotes and dashes. This is the script range of the project's four
/// languages and of the Glass's font. A denylist of invisible and reordering characters cannot be
/// complete, since Unicode keeps adding them; this one excludes them all. Lookalikes inside Latin
/// are still possible (`rn` and `m`), so text is never the proof of who something is from.
fn good_text(text: &str, max_bytes: usize) -> bool {
    !text.is_empty()
        && text.len() <= max_bytes
        && text.trim() == text
        && text.chars().all(allowed_text_char)
}

/// A name also needs something to read.
fn good_name(text: &str) -> bool {
    good_text(text, MAX_NAME_LEN) && text.chars().any(char::is_alphanumeric)
}

fn allowed_text_char(c: char) -> bool {
    matches!(c,
        ' '..='~'
        | '\u{A1}'..='\u{AC}'
        | '\u{AE}'..='\u{17F}'
        | '\u{2013}'
        | '\u{2014}'
        | '\u{2018}'
        | '\u{2019}'
        | '\u{201C}'
        | '\u{201D}'
        | '\u{2026}')
}

/// Lowercase hex of exactly `N` bytes: one spelling per value.
pub(super) fn decode_hex<const N: usize>(text: &str) -> Option<[u8; N]> {
    let bytes = text.as_bytes();
    if bytes.len() != N * 2 {
        return None;
    }
    let digit = |b: u8| match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        _ => None,
    };
    let mut out = [0u8; N];
    for (slot, pair) in out.iter_mut().zip(bytes.chunks_exact(2)) {
        *slot = digit(pair[0])? * 16 + digit(pair[1])?;
    }
    Some(out)
}

/// Lowercase hex, the form manifests and file names use.
pub fn to_hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(DIGITS[(b >> 4) as usize] as char);
        out.push(DIGITS[(b & 15) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_ids() {
        for good in ["com.example.app", "a.b", "org.x_y.app2"] {
            assert!(good_app_id(good), "{good}");
        }
        let long = format!("com.{}", "a".repeat(MAX_ID_LEN));
        for bad in [
            "",
            "app",
            "Com.example",
            "com..example",
            ".com.example",
            "com.example.",
            "com.1example",
            "com.exa-mple",
            "com.example/../x",
            "com.example\n",
            "com.ex\u{e4}mple",
            &long,
        ] {
            assert!(!good_app_id(bad), "{bad}");
        }
    }

    #[test]
    fn text_is_an_allowlist() {
        assert!(good_name("Città del Sole: l’ora – señor!"));
        assert!(good_text("Déjà vu…", MAX_NAME_LEN));
        for bad in [
            "",
            " padded",
            "padded ",
            "two\nlines",
            "esc\u{1b}[2J",
            "right\u{202e}txt.exe",
            "zero\u{200b}width",
            "bom\u{feff}",
            "tag\u{e0041}",
            "join\u{2060}er",
            "line\u{2028}sep",
            "soft\u{ad}hyphen",
            "nb\u{a0}sp",
            "blank\u{2800}",
            "a\u{1D173}b",
            "a\u{1BCA0}b",
            "a\u{13430}b",
            "\u{3164}x",
            "\u{05D0}\u{05D1}abc",
            "Сyrillic С",
            "東京",
        ] {
            assert!(!good_text(bad, MAX_NAME_LEN), "{bad:?}");
        }
        assert!(!good_text(&"a".repeat(MAX_NAME_LEN + 1), MAX_NAME_LEN));
        assert!(good_text("...", MAX_NAME_LEN));
        assert!(!good_name("..."));
    }

    #[test]
    fn versions_have_one_spelling() {
        assert!(parse_version("1.2.3").is_some());
        assert!(parse_version("1.2.3-beta.1").is_some());
        for bad in [
            "",
            "1",
            "1.2",
            "v1.2.3",
            "01.2.3",
            "1.2.3+build",
            "1.2.3 ",
            "1.2.3-01",
        ] {
            assert!(parse_version(bad).is_none(), "{bad}");
        }
        assert!(parse_version(&format!("1.2.3-{}", "a".repeat(MAX_VERSION_LEN))).is_none());
    }

    #[test]
    fn hex_is_strict() {
        assert_eq!(decode_hex::<2>("00ff"), Some([0, 255]));
        assert_eq!(to_hex(&[0, 255]), "00ff");
        for bad in ["00FF", "00f", "00ff00", "0g00", "é0"] {
            assert_eq!(decode_hex::<2>(bad), None, "{bad}");
        }
    }

    fn sample_release(target: &str, permissions: &[&str]) -> Release {
        Release {
            version: Version::new(1, 0, 0),
            target: target.into(),
            min_api: 1,
            url: HttpsUrl::parse("https://example.org/a").unwrap(),
            size: 1,
            sha256: [0; 32],
            permissions: permissions.iter().map(|s| s.to_string()).collect(),
            signature: [0; 64],
        }
    }

    #[test]
    fn statement_is_injective_across_field_boundaries() {
        let release = sample_release;
        let a = release_statement("a.b", &release("t", &["x", "y"]));
        assert_ne!(a, release_statement("a.b", &release("t", &["xy"])));
        assert_ne!(a, release_statement("a.bt", &release("", &["x", "y"])));
        // The URL can change without a new signature.
        let mut moved = release("t", &["x", "y"]);
        moved.url = HttpsUrl::parse("https://mirror.example.net/a").unwrap();
        assert_eq!(a, release_statement("a.b", &moved));
    }

    #[test]
    fn the_state_statement_covers_each_part_and_ignores_file_order() {
        let base = AppEntry {
            id: "a.b".into(),
            repository: HttpsUrl::parse("https://example.org/omg-apps.json").unwrap(),
            name: "N".into(),
            summary: "S".into(),
            license: "L".into(),
            homepage: None,
            publisher_key: [0; 32],
            sequence: 1,
            withdrawn: vec![],
            state_signature: [0; 64],
            releases: vec![sample_release("t", &[])],
        };
        let statement = app_state_statement(&base);
        let changed = |change: &dyn Fn(&mut AppEntry)| {
            let mut app = base.clone();
            change(&mut app);
            app_state_statement(&app)
        };
        for edit in [
            changed(&|a| a.repository = HttpsUrl::parse("https://other.example/x.json").unwrap()),
            changed(&|a| a.sequence = 2),
            changed(&|a| a.name = "M".into()),
            changed(&|a| a.summary = "T".into()),
            changed(&|a| a.license = "M".into()),
            changed(&|a| a.homepage = Some(HttpsUrl::parse("https://h.example/").unwrap())),
            changed(&|a| a.withdrawn = vec![Version::new(1, 0, 0)]),
            changed(&|a| a.releases.clear()),
            changed(&|a| a.releases[0].sha256 = [1; 32]),
        ] {
            assert_ne!(edit, statement);
        }
        let mut second = sample_release("t", &[]);
        second.version = Version::new(2, 0, 0);
        let one_way = changed(&|a| a.releases.push(second.clone()));
        let other_way = changed(&|a| a.releases.insert(0, second.clone()));
        assert_eq!(one_way, other_way);
    }
}
