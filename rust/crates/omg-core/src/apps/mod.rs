//! The Applications section: installing apps from repositories anyone can publish.
//!
//! A repository is a plain JSON file on GitHub or any HTTPS site, so everything read from it is
//! untrusted input. This module holds the logic every companion shares, and has no network and no
//! UI:
//!
//! * [`url`]: the narrow set of URLs the companion will ever request.
//! * [`source`]: from a pasted address to a repository's one identity.
//! * [`manifest`]: parsing with strict limits, and the publisher's signatures.
//! * [`trust`]: what is remembered about an app, across uninstalls.
//! * [`select`]: which release to offer, and the only way to start an install.
//! * [`download`]: fetching through an injected [`Fetcher`], with size, hash, deadline and cancel.
//!
//! No SDK and no installer on the Glass exist yet. The fields and lists that depend on them are
//! marked PROVISIONAL here. The design, the threat model and the interface the Glass will need are
//! in `docs/plans/2026-11-07-companion-applications.md`.

pub mod download;
pub mod manifest;
pub mod select;
pub mod source;
pub mod trust;
pub mod url;

pub use download::{
    fetch_manifest, fetch_package, DownloadError, DownloadVerifier, FetchControl, FetchResponse,
    Fetcher, FileSink, PackageSink, PackageStore, VerifiedPackage,
};
pub use manifest::{
    app_state_statement, release_statement, AppEntry, Manifest, ManifestError, Release,
};
pub use select::{
    check_update, plan_install, select_release, GlassProfile, InstallRefusal, InstallTicket,
    InstalledApp, SelectOptions, Selection, SkipReason, UpdateStatus, KNOWN_PERMISSIONS,
};
pub use source::{RepositorySource, SourceError};
pub use trust::{TrustError, TrustProblem, TrustRecord, TrustStore};
pub use url::{HttpsUrl, UrlError};
