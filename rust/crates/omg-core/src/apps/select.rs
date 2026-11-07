//! Which release to offer, and whether an installed app has an update.
//!
//! Pure functions over a checked [`Manifest`](super::Manifest). They never suggest a downgrade,
//! never pick a yanked release, and report rather than hide anything that needs the owner's eye.

use semver::Version;

use super::manifest::{AppEntry, Release, Sha256Digest};
use super::url::HttpsUrl;

/// PROVISIONAL, depends on the SDK and on `docs/plans/2026-11-07-companion-applications.md` §7.
/// A release asking for anything else is not offered: nobody could say what it would be granted.
pub const KNOWN_PERMISSIONS: &[&str] = &[
    "background",
    "camera",
    "companion.fetch",
    "dictation",
    "input.button",
    "input.head",
    "location",
    "microphone",
    "network",
    "notes.write",
    "notifications.read",
];

/// What the connected Glass can run. Reported by the Glass, so unknown until the installer exists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlassProfile {
    /// Highest app API the Glass offers. PROVISIONAL: depends on the SDK.
    pub api_level: u32,
    /// Package targets it can run. PROVISIONAL: depends on the SDK.
    pub targets: Vec<String>,
}

/// The owner's choices about what is offered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SelectOptions {
    /// Offer pre-release versions too.
    pub allow_prerelease: bool,
}

/// Why a release was passed over.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkipReason {
    /// The author withdrew it.
    Yanked,
    /// A pre-release, and the owner did not opt in.
    Prerelease,
    /// It does not run on this Glass.
    WrongTarget,
    /// It needs a newer app API than this Glass has.
    NeedsNewerGlass {
        /// The API it needs.
        min_api: u32,
    },
    /// It asks for a permission this companion does not know.
    UnknownPermission,
}

/// The outcome of choosing among an app's releases.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selection<'a> {
    /// The highest release that can be offered.
    pub best: Option<&'a Release>,
    /// Those passed over, with the reason.
    pub skipped: Vec<(&'a Version, SkipReason)>,
}

/// Picks the highest release this Glass can run and the owner would accept.
pub fn select_release<'a>(
    app: &'a AppEntry,
    profile: &GlassProfile,
    options: SelectOptions,
) -> Selection<'a> {
    let mut best: Option<&Release> = None;
    let mut skipped = Vec::new();
    for release in &app.releases {
        match skip_reason(release, profile, options) {
            Some(reason) => skipped.push((&release.version, reason)),
            None => {
                if best.is_none_or(|b| release.version > b.version) {
                    best = Some(release);
                }
            }
        }
    }
    Selection { best, skipped }
}

fn skip_reason(
    release: &Release,
    profile: &GlassProfile,
    options: SelectOptions,
) -> Option<SkipReason> {
    if release.yanked {
        Some(SkipReason::Yanked)
    } else if !release.version.pre.is_empty() && !options.allow_prerelease {
        Some(SkipReason::Prerelease)
    } else if !profile.targets.contains(&release.target) {
        Some(SkipReason::WrongTarget)
    } else if release.min_api > profile.api_level {
        Some(SkipReason::NeedsNewerGlass {
            min_api: release.min_api,
        })
    } else if release
        .permissions
        .iter()
        .any(|p| !KNOWN_PERMISSIONS.contains(&p.as_str()))
    {
        Some(SkipReason::UnknownPermission)
    } else {
        None
    }
}

/// What the companion remembers of an installed app. Must be kept by the companion and, for the
/// key and the version, enforced by the Glass as well.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledApp {
    /// The app id.
    pub id: String,
    /// The installed version.
    pub version: Version,
    /// The manifest it was installed from.
    pub repository: HttpsUrl,
    /// The key pinned at first install.
    pub publisher_key: [u8; 32],
    /// The hash of the installed package.
    pub sha256: Sha256Digest,
    /// The permissions the owner granted.
    pub permissions: Vec<String>,
}

/// The state of an installed app against a repository's current manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateStatus<'a> {
    /// Nothing newer can be offered.
    UpToDate {
        /// The installed release was withdrawn by its author.
        installed_yanked: bool,
        /// Newer releases that exist but were passed over, so the UI can say why.
        newer_skipped: Vec<(&'a Version, SkipReason)>,
    },
    /// A newer release can be installed, after the owner's consent.
    UpdateAvailable {
        /// The release.
        release: &'a Release,
        /// Permissions it asks for beyond those granted. Not empty means a fresh question to the
        /// owner: an update never gains a permission silently.
        added_permissions: Vec<&'a str>,
        /// The installed release was withdrawn by its author.
        installed_yanked: bool,
    },
    /// This manifest is not the repository the app was installed from. Nothing is offered: the
    /// same id elsewhere is another author's claim, and the owner must uninstall to switch.
    OtherSource,
    /// The publisher key changed. Nothing is offered: this is what a stolen repository looks like.
    PublisherChanged,
    /// The repository now lists the installed version with a different hash. A published version
    /// never changes, so the same warning applies.
    ReleaseChanged,
}

/// Compares an installed app with `app` as read from `repository`.
pub fn check_update<'a>(
    installed: &InstalledApp,
    repository: &HttpsUrl,
    app: &'a AppEntry,
    profile: &GlassProfile,
    options: SelectOptions,
) -> UpdateStatus<'a> {
    if installed.id != app.id || installed.repository != *repository {
        return UpdateStatus::OtherSource;
    }
    if installed.publisher_key != app.publisher_key {
        return UpdateStatus::PublisherChanged;
    }
    let same_version = app.releases.iter().find(|r| r.version == installed.version);
    if same_version.is_some_and(|r| r.sha256 != installed.sha256) {
        return UpdateStatus::ReleaseChanged;
    }
    let installed_yanked = same_version.is_some_and(|r| r.yanked);

    // Only a strictly higher version is an update. So an owner on a pre-release is never moved
    // down to the last stable version, and an old manifest served by a host can hide newer
    // releases but cannot bring an old one back.
    let selection = select_release(app, profile, options);
    match selection.best {
        Some(release) if release.version > installed.version => {
            let added_permissions = release
                .permissions
                .iter()
                .map(String::as_str)
                .filter(|p| !installed.permissions.iter().any(|g| g == p))
                .collect();
            UpdateStatus::UpdateAvailable {
                release,
                added_permissions,
                installed_yanked,
            }
        }
        _ => UpdateStatus::UpToDate {
            installed_yanked,
            newer_skipped: selection
                .skipped
                .into_iter()
                .filter(|(version, reason)| {
                    **version > installed.version && *reason != SkipReason::Yanked
                })
                .collect(),
        },
    }
}
