//! The repository manifest: untrusted JSON from anywhere on the internet.
//!
//! [`Manifest::parse`] is all-or-nothing. It returns a value only when every field of every app
//! and release is well formed and every release's signature verifies; anything else refuses the
//! whole file. Errors name the position and the field, and never repeat text from the file, since
//! they can end up on a screen. The format is in `docs/plans/2026-11-07-companion-applications.md`.

use std::collections::HashSet;

use ed25519_dalek::{Signature, VerifyingKey};
use semver::Version;
use serde::Deserialize;
use thiserror::Error;

use super::url::{HttpsUrl, UrlError};

/// Largest manifest file, in bytes. Checked before the JSON parser sees anything.
pub const MAX_MANIFEST_BYTES: usize = 512 * 1024;
/// Most apps in one repository.
pub const MAX_APPS: usize = 32;
/// Most releases of one app. Bounds the signature checks of one manifest to 512.
pub const MAX_RELEASES_PER_APP: usize = 16;
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

/// App ids under this prefix belong to the project and are never accepted from a repository.
const RESERVED_ID_PREFIX: &str = "org.openmyglass.";

const SCHEMA_VERSION: u32 = 1;

/// Domain separation of the signed statement of a release.
const STATEMENT_DOMAIN: &[u8] = b"openmyglass-app-release-v1\0";

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
}

/// A checked repository manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    /// The repository's display name. Untrusted text, checked but not vouched for.
    pub name: String,
    /// Its apps, in file order.
    pub apps: Vec<AppEntry>,
}

/// One app of a repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppEntry {
    /// Reverse-DNS id, also a safe path component on the Glass.
    pub id: String,
    /// Display name. Untrusted text: always show the repository's host beside it.
    pub name: String,
    /// One line of description.
    pub summary: String,
    /// Licence, as the author states it.
    pub license: String,
    /// The author's page, if any.
    pub homepage: Option<HttpsUrl>,
    /// The ed25519 key that signs this app's releases.
    pub publisher_key: [u8; 32],
    /// Releases, in file order. All signatures have been verified.
    pub releases: Vec<Release>,
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
    /// Where to download the package. Not signed, so a package can move host.
    pub url: HttpsUrl,
    /// Exact size of the package.
    pub size: u64,
    /// SHA-256 of the package.
    pub sha256: Sha256Digest,
    /// What the app asks for, sorted. Syntax is checked here; see [`super::select`] for meaning.
    pub permissions: Vec<String>,
    /// The author withdrew this release. Not signed: it can only hide a release.
    pub yanked: bool,
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
    #[serde(default)]
    yanked: bool,
    signature: String,
}

/// Only `schema` is read first, so that a newer file gets "update the companion" and not a
/// confusing complaint about the first field this version does not know.
#[derive(Deserialize)]
struct SchemaOnly {
    schema: u32,
}

impl Manifest {
    /// Parses and checks a manifest. Never panics, whatever the input.
    pub fn parse(bytes: &[u8]) -> Result<Self, ManifestError> {
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

        if !good_text(&raw.name, MAX_NAME_LEN) {
            return Err(ManifestError::BadField("name"));
        }
        if raw.apps.is_empty() || raw.apps.len() > MAX_APPS {
            return Err(ManifestError::BadAppCount);
        }
        let mut seen = HashSet::new();
        let mut apps = Vec::with_capacity(raw.apps.len());
        for (index, app) in raw.apps.into_iter().enumerate() {
            let app = check_app(index, app)?;
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

fn check_app(index: usize, raw: RawApp) -> Result<AppEntry, ManifestError> {
    let bad = |field| ManifestError::BadAppField { app: index, field };
    if !good_app_id(&raw.id) {
        return Err(bad("id"));
    }
    if raw.id.starts_with(RESERVED_ID_PREFIX) {
        return Err(ManifestError::ReservedId(index));
    }
    if !good_text(&raw.name, MAX_NAME_LEN) {
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

    let mut versions = HashSet::new();
    let mut releases = Vec::with_capacity(raw.releases.len());
    for (position, raw_release) in raw.releases.into_iter().enumerate() {
        let (release, signature) = check_release(index, position, raw_release)?;
        if !versions.insert(release.version.clone()) {
            return Err(ManifestError::DuplicateVersion {
                app: index,
                release: position,
            });
        }
        // verify_strict: no malleable signatures, no small-order values.
        key.verify_strict(&release_statement(&raw.id, &release), &signature)
            .map_err(|_| ManifestError::BadSignature {
                app: index,
                release: position,
            })?;
        releases.push(release);
    }
    Ok(AppEntry {
        id: raw.id,
        name: raw.name,
        summary: raw.summary,
        license: raw.license,
        homepage,
        publisher_key,
        releases,
    })
}

fn check_release(
    app: usize,
    release: usize,
    raw: RawRelease,
) -> Result<(Release, Signature), ManifestError> {
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
    Ok((
        Release {
            version,
            target: raw.target,
            min_api: raw.min_api,
            url,
            size: raw.size,
            sha256,
            permissions,
            yanked: raw.yanked,
        },
        Signature::from_bytes(&signature),
    ))
}

/// The exact bytes a publisher signs for a release, and that the Glass can verify again without
/// the manifest. Length-prefixed, so no two different releases give the same bytes. Covers what
/// the app is and what it may do; the URL and `yanked` are left out on purpose.
pub fn release_statement(app_id: &str, release: &Release) -> Vec<u8> {
    fn field(out: &mut Vec<u8>, bytes: &[u8]) {
        // Every field is limited far below u32::MAX by the checks above.
        out.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
        out.extend_from_slice(bytes);
    }
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

/// Strict semantic version: no build metadata, and one spelling per version (the parser refuses
/// leading zeros), so a version string identifies one thing.
fn parse_version(text: &str) -> Option<Version> {
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

/// One line of displayable text: not empty, no outer spaces, nothing that moves the cursor, hides
/// characters or reorders what the owner reads.
fn good_text(text: &str, max_bytes: usize) -> bool {
    !text.is_empty()
        && text.len() <= max_bytes
        && text.trim() == text
        && !text
            .chars()
            .any(|c| c.is_control() || is_invisible_format(c))
}

/// The Unicode format characters that are used to spoof text: zero-width, bidirectional controls,
/// word joiners, the byte order mark, interlinear annotation and tags.
fn is_invisible_format(c: char) -> bool {
    matches!(c,
        '\u{00AD}'
        | '\u{034F}'
        | '\u{061C}'
        | '\u{115F}'..='\u{1160}'
        | '\u{17B4}'..='\u{17B5}'
        | '\u{180B}'..='\u{180F}'
        | '\u{200B}'..='\u{200F}'
        | '\u{2028}'..='\u{202F}'
        | '\u{205F}'..='\u{206F}'
        | '\u{3000}'
        | '\u{3164}'
        | '\u{FE00}'..='\u{FE0F}'
        | '\u{FEFF}'
        | '\u{FFA0}'
        | '\u{FFF0}'..='\u{FFFB}'
        | '\u{E0000}'..='\u{E0FFF}')
}

/// Lowercase hex of exactly `N` bytes: one spelling per value.
fn decode_hex<const N: usize>(text: &str) -> Option<[u8; N]> {
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
    fn text_refuses_spoofing_characters() {
        assert!(good_text("Città del Sole: 東京", MAX_NAME_LEN));
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
            "nbsp\u{2060}join",
            "line\u{2028}sep",
        ] {
            assert!(!good_text(bad, MAX_NAME_LEN), "{bad:?}");
        }
        assert!(!good_text(&"a".repeat(MAX_NAME_LEN + 1), MAX_NAME_LEN));
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

    #[test]
    fn statement_is_injective_across_field_boundaries() {
        let release = |target: &str, permissions: &[&str]| Release {
            version: Version::new(1, 0, 0),
            target: target.into(),
            min_api: 1,
            url: HttpsUrl::parse("https://example.org/a").unwrap(),
            size: 1,
            sha256: [0; 32],
            permissions: permissions.iter().map(|s| s.to_string()).collect(),
            yanked: false,
        };
        let a = release_statement("a.b", &release("t", &["x", "y"]));
        assert_ne!(a, release_statement("a.b", &release("t", &["xy"])));
        assert_ne!(a, release_statement("a.bt", &release("", &["x", "y"])));
        // The URL and yanked can change without a new signature.
        let mut moved = release("t", &["x", "y"]);
        moved.url = HttpsUrl::parse("https://mirror.example.net/a").unwrap();
        moved.yanked = true;
        assert_eq!(a, release_statement("a.b", &moved));
    }
}
