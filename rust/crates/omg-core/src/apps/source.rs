//! Turning what the owner pasted into the address of a repository manifest.

use thiserror::Error;

use super::url::{HttpsUrl, UrlError};

/// The manifest's file name, at the root of a GitHub repository or of a site directory.
pub const MANIFEST_FILE: &str = "omg-apps.json";

const GITHUB_RAW_HOST: &str = "raw.githubusercontent.com";

/// Why an address cannot be added as a repository.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum SourceError {
    /// The address is not an acceptable URL.
    #[error("invalid address: {0}")]
    Url(#[from] UrlError),
    /// A GitHub address that is not `https://github.com/OWNER/REPO`.
    #[error("use the repository's main address, https://github.com/OWNER/REPO")]
    NotAGithubRepository,
    /// A site address with a query: the manifest location must be stable and plain.
    #[error("a repository address cannot have a query")]
    HasQuery,
}

/// A repository the companion can read. Its [`manifest_url`](Self::manifest_url) is its identity,
/// in one spelling per repository: GitHub names are lowercased because GitHub ignores their case,
/// `:443` is dropped, and repeated slashes collapse.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositorySource {
    manifest_url: HttpsUrl,
    github: Option<(String, String)>,
}

impl RepositorySource {
    /// Reads a pasted address: `https://github.com/OWNER/REPO`, a site directory, or the full URL
    /// of a `.json` manifest.
    pub fn parse(input: &str) -> Result<Self, SourceError> {
        let url = HttpsUrl::parse(input.trim())?;
        if url.target().contains('?') {
            return Err(SourceError::HasQuery);
        }
        let target = collapse_slashes(url.target());
        let github = match url.host() {
            "github.com" | "www.github.com" => Some(github_from_page(&url, &target)?),
            GITHUB_RAW_HOST => Some(github_from_raw(&url, &target)?),
            _ => None,
        };
        let manifest = if let Some((owner, repo)) = &github {
            format!("https://{GITHUB_RAW_HOST}/{owner}/{repo}/HEAD/{MANIFEST_FILE}")
        } else if target.ends_with(".json") {
            format!("https://{}{target}", url.origin())
        } else if target.ends_with('/') {
            format!("https://{}{target}{MANIFEST_FILE}", url.origin())
        } else {
            format!("https://{}{target}/{MANIFEST_FILE}", url.origin())
        };
        Ok(Self {
            manifest_url: HttpsUrl::parse(&manifest)?,
            github,
        })
    }

    /// Where the manifest lives.
    pub fn manifest_url(&self) -> &HttpsUrl {
        &self.manifest_url
    }

    /// Owner and repository, lowercase, for a GitHub repository.
    pub fn github(&self) -> Option<(&str, &str)> {
        self.github.as_ref().map(|(o, r)| (o.as_str(), r.as_str()))
    }

    /// What the UI shows as the source of an app, always beside the app's own name:
    /// `github.com/owner/repo`, or the origin and the path of the manifest. A host alone would
    /// not do, since every GitHub repository is on one host and so are the users of a shared site.
    pub fn label(&self) -> String {
        match &self.github {
            Some((owner, repo)) => format!("github.com/{owner}/{repo}"),
            None => format!(
                "{}{}",
                self.manifest_url.origin(),
                self.manifest_url.target()
            ),
        }
    }

    /// Whether `url` is still this repository, for a manifest redirect. For a site: the same
    /// origin, so a site shared between users is one trust domain. For GitHub, where every
    /// repository shares one host: the same owner and repository.
    pub fn covers(&self, url: &HttpsUrl) -> bool {
        match &self.github {
            Some((owner, repo)) => {
                let prefix = format!("/{owner}/{repo}/");
                url.origin() == GITHUB_RAW_HOST
                    && url.target().len() > prefix.len()
                    && url.target()[..prefix.len()].eq_ignore_ascii_case(&prefix)
            }
            None => url.origin() == self.manifest_url.origin(),
        }
    }

    /// Rebuilds a source from a manifest URL kept by the companion.
    pub fn from_manifest_url(manifest_url: &HttpsUrl) -> Result<Self, SourceError> {
        Self::parse(manifest_url.as_str())
    }
}

fn collapse_slashes(target: &str) -> String {
    let mut out = String::with_capacity(target.len());
    for c in target.chars() {
        if c != '/' || !out.ends_with('/') {
            out.push(c);
        }
    }
    out
}

/// `/OWNER/REPO`, on GitHub's real port. GitHub's API is rate limited per address without a token,
/// so the manifest is read as a plain file from the default branch, as from any other site.
fn github_from_page(url: &HttpsUrl, target: &str) -> Result<(String, String), SourceError> {
    if url.origin() != url.host() {
        return Err(SourceError::NotAGithubRepository);
    }
    let path = target.trim_matches('/');
    let mut parts = path.split('/');
    let (Some(owner), Some(repo), None) = (parts.next(), parts.next(), parts.next()) else {
        return Err(SourceError::NotAGithubRepository);
    };
    github_names(owner, repo)
}

/// The raw address of a default-branch manifest is the same repository. Any other raw path is
/// refused: a pinned commit or another file would be a second identity for one repository.
fn github_from_raw(url: &HttpsUrl, target: &str) -> Result<(String, String), SourceError> {
    if url.origin() != url.host() {
        return Err(SourceError::NotAGithubRepository);
    }
    let path = target.trim_start_matches('/');
    let mut parts = path.split('/');
    let (Some(owner), Some(repo), Some("HEAD"), Some(MANIFEST_FILE), None) = (
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
    ) else {
        return Err(SourceError::NotAGithubRepository);
    };
    github_names(owner, repo)
}

fn github_names(owner: &str, repo: &str) -> Result<(String, String), SourceError> {
    let owner = owner.to_ascii_lowercase();
    let repo = repo.to_ascii_lowercase();
    let repo = repo.strip_suffix(".git").unwrap_or(&repo).to_owned();
    if !github_name(&owner) || !github_name(&repo) {
        return Err(SourceError::NotAGithubRepository);
    }
    Ok((owner, repo))
}

fn github_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('.')
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(input: &str) -> String {
        RepositorySource::parse(input)
            .unwrap()
            .manifest_url()
            .to_string()
    }

    #[test]
    fn one_repository_has_one_identity() {
        let want = "https://raw.githubusercontent.com/o-1/my.repo/HEAD/omg-apps.json";
        for spelling in [
            "https://github.com/o-1/my.repo",
            "https://github.com/o-1/my.repo/",
            "  https://github.com/o-1/my.repo.git\n",
            "https://www.github.com/o-1/my.repo",
            "https://GitHub.com/O-1/My.Repo",
            "https://github.com:443/o-1/my.repo",
            "https://github.com//o-1//my.repo",
            "https://raw.githubusercontent.com/O-1/My.Repo/HEAD/omg-apps.json",
        ] {
            assert_eq!(manifest(spelling), want, "{spelling}");
        }
        assert_eq!(
            manifest("https://example.org:443//x//"),
            "https://example.org/x/omg-apps.json"
        );
    }

    #[test]
    fn every_github_repository_shows_its_own_name() {
        let source = RepositorySource::parse("https://github.com/Some-One/Tally").unwrap();
        assert_eq!(source.github(), Some(("some-one", "tally")));
        assert_eq!(source.label(), "github.com/some-one/tally");
        let again = RepositorySource::from_manifest_url(source.manifest_url()).unwrap();
        assert_eq!(again, source);
    }

    #[test]
    fn a_site_shows_its_origin_and_path() {
        let source = RepositorySource::parse("https://Example.org:8443/~me/glass").unwrap();
        assert_eq!(source.github(), None);
        assert_eq!(source.label(), "example.org:8443/~me/glass/omg-apps.json");
        let again = RepositorySource::from_manifest_url(source.manifest_url()).unwrap();
        assert_eq!(again, source);
    }

    #[test]
    fn a_github_repository_covers_only_itself() {
        let source = RepositorySource::parse("https://github.com/alice/tool").unwrap();
        let url = |u: &str| HttpsUrl::parse(u).unwrap();
        assert!(source.covers(&url(
            "https://raw.githubusercontent.com/Alice/Tool/main/omg-apps.json"
        )));
        assert!(!source.covers(&url(
            "https://raw.githubusercontent.com/mallory/x/HEAD/omg-apps.json"
        )));
        assert!(!source.covers(&url(
            "https://raw.githubusercontent.com/alice/toolbox/HEAD/omg-apps.json"
        )));
        assert!(!source.covers(&url("https://github.com/alice/tool/raw/HEAD/omg-apps.json")));
        let site = RepositorySource::parse("https://example.org/a/").unwrap();
        assert!(site.covers(&url("https://example.org/b/omg-apps.json")));
        assert!(!site.covers(&url("https://example.org:8443/a/omg-apps.json")));
    }

    #[test]
    fn other_github_addresses_are_refused_not_guessed() {
        for bad in [
            "https://github.com/",
            "https://github.com/o",
            "https://github.com/o/r/tree/main",
            "https://github.com/o/r/releases",
            "https://github.com/o/.git",
            "https://github.com:8443/o/r",
            "https://raw.githubusercontent.com/o/r/main/omg-apps.json",
            "https://raw.githubusercontent.com/o/r/HEAD/other.json",
            "https://raw.githubusercontent.com:8443/o/r/HEAD/omg-apps.json",
        ] {
            assert_eq!(
                RepositorySource::parse(bad),
                Err(SourceError::NotAGithubRepository),
                "{bad}"
            );
        }
    }

    #[test]
    fn sites_give_a_directory_or_a_file() {
        assert_eq!(
            manifest("https://example.org/glass/"),
            "https://example.org/glass/omg-apps.json"
        );
        assert_eq!(
            manifest("https://example.org/glass"),
            "https://example.org/glass/omg-apps.json"
        );
        assert_eq!(
            manifest("https://example.org/"),
            "https://example.org/omg-apps.json"
        );
        assert_eq!(
            manifest("https://example.org/x/apps.json"),
            "https://example.org/x/apps.json"
        );
        assert_eq!(
            RepositorySource::parse("https://example.org/a?b=1"),
            Err(SourceError::HasQuery)
        );
        assert!(matches!(
            RepositorySource::parse("http://example.org/"),
            Err(SourceError::Url(UrlError::NotHttps))
        ));
    }
}
