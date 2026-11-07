//! Which release to offer, whether an installed app has an update, and the one way to start an
//! install.
//!
//! Pure functions over a checked [`Manifest`](super::Manifest) and what the companion remembers.
//! They never suggest a downgrade or a withdrawn release, and report rather than hide anything
//! that needs the owner's eye. [`plan_install`] is the only way to get the [`InstallTicket`] that
//! a download needs, so the rules here cannot be skipped on the install path.

use semver::Version;

use super::manifest::{AppEntry, Release, Sha256Digest};
use super::trust::{TrustProblem, TrustRecord};

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
    /// The author withdrew it, now or in a state this companion remembers.
    Withdrawn,
    /// Lower than a version this companion already installed for this app.
    BelowInstalledFloor,
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

/// Picks the highest release this Glass can run and the owner would accept. This only ranks. It
/// does not check the record against the app, which is [`plan_install`]'s job.
pub fn select_release<'a>(
    app: &'a AppEntry,
    trust: Option<&TrustRecord>,
    profile: &GlassProfile,
    options: SelectOptions,
) -> Selection<'a> {
    let mut best: Option<&Release> = None;
    let mut skipped = Vec::new();
    for release in &app.releases {
        match skip_reason(app, trust, release, profile, options) {
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
    app: &AppEntry,
    trust: Option<&TrustRecord>,
    release: &Release,
    profile: &GlassProfile,
    options: SelectOptions,
) -> Option<SkipReason> {
    let below_floor = trust
        .and_then(|t| t.highest.as_ref())
        .is_some_and(|(highest, _)| release.version < *highest);
    if is_withdrawn(app, trust, &release.version) {
        Some(SkipReason::Withdrawn)
    } else if below_floor {
        Some(SkipReason::BelowInstalledFloor)
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

/// In this state, or in any state this companion has seen.
fn is_withdrawn(app: &AppEntry, trust: Option<&TrustRecord>, version: &Version) -> bool {
    app.is_withdrawn(version) || trust.is_some_and(|t| t.withdrawn.contains(version))
}

/// What is on the Glass now. Who it came from is in the [`TrustRecord`], which outlives this.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledApp {
    /// The app id.
    pub id: String,
    /// The installed version.
    pub version: Version,
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
        /// The author withdrew the installed release.
        installed_withdrawn: bool,
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
        /// The author withdrew the installed release.
        installed_withdrawn: bool,
    },
    /// Nothing is offered, and the UI shows the warning for this problem.
    Blocked(TrustProblem),
}

/// Compares an installed app with `app` as read from its repository.
pub fn check_update<'a>(
    installed: &InstalledApp,
    trust: &TrustRecord,
    app: &'a AppEntry,
    profile: &GlassProfile,
    options: SelectOptions,
) -> UpdateStatus<'a> {
    if let Err(problem) = check_installed(installed, trust, app) {
        return UpdateStatus::Blocked(problem);
    }
    let installed_withdrawn = is_withdrawn(app, Some(trust), &installed.version);

    // Only a strictly higher version is an update. So an owner on a pre-release is never moved
    // down to the last stable version.
    let selection = select_release(app, Some(trust), profile, options);
    match selection.best {
        Some(release) if release.version > installed.version => UpdateStatus::UpdateAvailable {
            release,
            added_permissions: added_permissions(release, Some(installed)),
            installed_withdrawn,
        },
        _ => UpdateStatus::UpToDate {
            installed_withdrawn,
            newer_skipped: selection
                .skipped
                .into_iter()
                .filter(|(version, reason)| {
                    **version > installed.version && *reason != SkipReason::Withdrawn
                })
                .collect(),
        },
    }
}

fn check_installed(
    installed: &InstalledApp,
    trust: &TrustRecord,
    app: &AppEntry,
) -> Result<(), TrustProblem> {
    trust.check(app)?;
    if installed.id != app.id {
        return Err(TrustProblem::OtherSource);
    }
    let changed = app
        .releases
        .iter()
        .any(|r| r.version == installed.version && r.sha256 != installed.sha256);
    if changed {
        return Err(TrustProblem::ReleaseChanged);
    }
    Ok(())
}

fn added_permissions<'a>(release: &'a Release, installed: Option<&InstalledApp>) -> Vec<&'a str> {
    release
        .permissions
        .iter()
        .map(String::as_str)
        .filter(|p| installed.is_none_or(|i| !i.permissions.iter().any(|g| g == p)))
        .collect()
}

/// Why an install cannot start.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum InstallRefusal {
    /// What this companion remembers about the app disagrees with the manifest.
    #[error("{0}")]
    Untrusted(TrustProblem),
    /// No release can be offered, or none higher than the installed one. [`select_release`] says
    /// why.
    #[error("nothing to install")]
    NothingToOffer,
    /// An app is installed but its record is missing: the memory is damaged. Nothing is assumed.
    #[error("the record of the installed app is missing")]
    InstalledWithoutRecord,
}

/// Permission to download one release: proof that the rules in this module passed. Only
/// [`plan_install`] makes one, and [`super::fetch_package`] needs one.
#[derive(Debug, Clone)]
pub struct InstallTicket<'a> {
    pub(super) app: &'a AppEntry,
    pub(super) release: &'a Release,
    first_use: bool,
    permissions_to_confirm: Vec<&'a str>,
}

impl<'a> InstallTicket<'a> {
    /// The app.
    pub fn app(&self) -> &'a AppEntry {
        self.app
    }

    /// The release to download.
    pub fn release(&self) -> &'a Release {
        self.release
    }

    /// Nothing is remembered about this id, so its key becomes trusted now. The owner must see the
    /// source, the key fingerprint and the warning before going on.
    pub fn first_use(&self) -> bool {
        self.first_use
    }

    /// What the owner has to approve: every permission on a first install or a reinstall, the new
    /// ones on an update.
    pub fn permissions_to_confirm(&self) -> &[&'a str] {
        &self.permissions_to_confirm
    }
}

/// Decides whether an install or update of `app` may start. `trust` is what the
/// [`super::TrustStore`] holds for this id, and survives an uninstall, so a reinstall meets the
/// same key, the same repository, the same version floor and the same withdrawn versions.
pub fn plan_install<'a>(
    app: &'a AppEntry,
    trust: Option<&TrustRecord>,
    installed: Option<&InstalledApp>,
    profile: &GlassProfile,
    options: SelectOptions,
) -> Result<InstallTicket<'a>, InstallRefusal> {
    match (trust, installed) {
        (None, Some(_)) => return Err(InstallRefusal::InstalledWithoutRecord),
        (Some(trust), Some(installed)) => {
            check_installed(installed, trust, app).map_err(InstallRefusal::Untrusted)?;
        }
        (Some(trust), None) => trust.check(app).map_err(InstallRefusal::Untrusted)?,
        (None, None) => {}
    }
    let release = select_release(app, trust, profile, options)
        .best
        .ok_or(InstallRefusal::NothingToOffer)?;
    if installed.is_some_and(|i| release.version <= i.version) {
        return Err(InstallRefusal::NothingToOffer);
    }
    Ok(InstallTicket {
        app,
        release,
        first_use: trust.is_none(),
        permissions_to_confirm: added_permissions(release, installed),
    })
}
