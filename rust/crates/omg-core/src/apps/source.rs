//! Turning what the owner pasted into the address of a repository manifest.

use thiserror::Error;

use super::url::{HttpsUrl, UrlError};

/// The manifest's file name, at the root of a GitHub repository or of a site directory.
pub const MANIFEST_FILE: &str = "omg-apps.json";

/// Why an address cannot be added as a repository.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum SourceError {
    /// The address is not an acceptable URL.
    #[error("invalid address: {0}")]
    Url(#[from] UrlError),
    /// A `github.com` address that is not `https://github.com/OWNER/REPO`.
    #[error("use the repository's main address, https://github.com/OWNER/REPO")]
    NotAGithubRepository,
    /// A site address with a query: the manifest location must be stable and plain.
    #[error("a repository address cannot have a query")]
    HasQuery,
}

/// A repository the companion can read. Its [`manifest_url`](Self::manifest_url) is its identity:
/// an installed app remembers it, and another repository offering the same app id is not treated
/// as a source of updates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositorySource {
    manifest_url: HttpsUrl,
}

impl RepositorySource {
    /// Reads a pasted address: `https://github.com/OWNER/REPO`, a site directory, or the full URL
    /// of a `.json` manifest.
    pub fn parse(input: &str) -> Result<Self, SourceError> {
        let url = HttpsUrl::parse(input.trim())?;
        if url.target().contains('?') {
            return Err(SourceError::HasQuery);
        }
        let manifest = match url.host() {
            "github.com" | "www.github.com" => github_manifest(url.target())?,
            _ if url.target().ends_with(".json") => url.as_str().to_owned(),
            _ if url.target().ends_with('/') => format!("{url}{MANIFEST_FILE}"),
            _ => format!("{url}/{MANIFEST_FILE}"),
        };
        Ok(Self {
            manifest_url: HttpsUrl::parse(&manifest)?,
        })
    }

    /// Where the manifest lives.
    pub fn manifest_url(&self) -> &HttpsUrl {
        &self.manifest_url
    }

    /// Rebuilds a source from a manifest URL kept by the companion.
    pub fn from_manifest_url(manifest_url: HttpsUrl) -> Self {
        Self { manifest_url }
    }
}

/// GitHub's API is rate limited per address without a token, so the manifest is read as a plain
/// file from the default branch, the same way as from any other site.
fn github_manifest(target: &str) -> Result<String, SourceError> {
    let path = target.trim_matches('/');
    let mut parts = path.split('/');
    let (Some(owner), Some(repo), None) = (parts.next(), parts.next(), parts.next()) else {
        return Err(SourceError::NotAGithubRepository);
    };
    let repo = repo.strip_suffix(".git").unwrap_or(repo);
    if !github_name(owner) || !github_name(repo) {
        return Err(SourceError::NotAGithubRepository);
    }
    Ok(format!(
        "https://raw.githubusercontent.com/{owner}/{repo}/HEAD/{MANIFEST_FILE}"
    ))
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
    fn github_repositories_read_the_default_branch() {
        let want = "https://raw.githubusercontent.com/o-1/my.repo/HEAD/omg-apps.json";
        assert_eq!(manifest("https://github.com/o-1/my.repo"), want);
        assert_eq!(manifest("https://github.com/o-1/my.repo/"), want);
        assert_eq!(manifest("  https://github.com/o-1/my.repo.git\n"), want);
        assert_eq!(manifest("https://www.github.com/o-1/my.repo"), want);
    }

    #[test]
    fn other_github_addresses_are_refused_not_guessed() {
        for bad in [
            "https://github.com/",
            "https://github.com/o",
            "https://github.com/o/r/tree/main",
            "https://github.com/o/r/releases",
            "https://github.com/o/.git",
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
