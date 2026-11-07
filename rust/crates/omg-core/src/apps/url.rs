//! The only URLs the Applications section will ever ask a fetcher for.
//!
//! Every URL here comes from untrusted input: a pasted repository address, a manifest, or a
//! redirect. The rules are deliberately narrower than the URL standard, so that this parser and
//! the platform's HTTP stack cannot read the same string differently: HTTPS only, plain ASCII, a
//! real DNS name, no credentials, no IP addresses, no fragments, no encoded dots or slashes.

use std::fmt;

use thiserror::Error;

/// Longest URL accepted, in bytes.
pub const MAX_URL_LEN: usize = 1024;

/// Names that resolve only inside a network, refused so that a repository cannot make the phone
/// send requests to the owner's router or NAS. A public name that resolves to a private address
/// is out of reach here: see the design doc, threat T7.
const PRIVATE_SUFFIXES: &[&str] = &[
    "local",
    "localhost",
    "localdomain",
    "internal",
    "intranet",
    "lan",
    "home",
    "corp",
    "home.arpa",
    "in-addr.arpa",
    "ip6.arpa",
];

/// Why a URL was refused. Carries no part of the URL: the text is untrusted and may reach a screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum UrlError {
    /// Longer than [`MAX_URL_LEN`].
    #[error("URL too long")]
    TooLong,
    /// Anything but `https://`.
    #[error("only https:// URLs are accepted")]
    NotHttps,
    /// A character outside the accepted set.
    #[error("URL has a forbidden character")]
    BadCharacter,
    /// Not a public DNS name: an IP address, a single label, a malformed label.
    #[error("URL host is not a public DNS name")]
    BadHost,
    /// A host that exists only inside a network.
    #[error("URL host is a private network name")]
    PrivateHost,
    /// A malformed or out-of-range port.
    #[error("URL has an invalid port")]
    BadPort,
    /// No path, dot segments, or a malformed or forbidden percent escape.
    #[error("URL has an invalid path")]
    BadPath,
    /// A redirect location that is neither absolute HTTPS nor path-absolute.
    #[error("redirect location is not usable")]
    BadRedirect,
}

/// A URL that passed every rule in this module. The only type a [`super::Fetcher`] accepts.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct HttpsUrl {
    text: String,
    /// Where the path starts in `text`.
    path_at: usize,
    /// Where the host ends in `text`.
    host_end: usize,
}

const SCHEME: &str = "https://";

impl HttpsUrl {
    /// Checks `input` against every rule and returns its canonical form (lowercase host).
    pub fn parse(input: &str) -> Result<Self, UrlError> {
        if input.len() > MAX_URL_LEN {
            return Err(UrlError::TooLong);
        }
        let rest = input.strip_prefix(SCHEME).ok_or(UrlError::NotHttps)?;
        if !rest.bytes().all(allowed_byte) {
            return Err(UrlError::BadCharacter);
        }
        let slash = rest.find('/').ok_or(UrlError::BadPath)?;
        let (authority, target) = rest.split_at(slash);
        // Credentials are the classic way to make a URL show one host and reach another.
        if authority.contains('@') || authority.contains('?') {
            return Err(UrlError::BadHost);
        }
        let (host, port) = match authority.split_once(':') {
            Some((host, port)) => (host, Some(port)),
            None => (authority, None),
        };
        let host = host.to_ascii_lowercase();
        check_host(&host)?;
        if let Some(port) = port {
            check_port(port)?;
        }
        check_target(target)?;

        let host_end = SCHEME.len() + host.len();
        let mut text = String::with_capacity(input.len());
        text.push_str(SCHEME);
        text.push_str(&host);
        if let Some(port) = port {
            text.push(':');
            text.push_str(port);
        }
        let path_at = text.len();
        text.push_str(target);
        Ok(Self {
            text,
            path_at,
            host_end,
        })
    }

    /// The canonical URL.
    pub fn as_str(&self) -> &str {
        &self.text
    }

    /// The host, lowercase, without the port. This, and never an app's own name, is what the UI
    /// shows as the origin of an app.
    pub fn host(&self) -> &str {
        &self.text[SCHEME.len()..self.host_end]
    }

    /// The path and the query, starting with `/`.
    pub fn target(&self) -> &str {
        &self.text[self.path_at..]
    }

    /// Resolves the `Location` of a redirect from this URL. The result passes the same rules.
    pub fn redirect(&self, location: &str) -> Result<Self, UrlError> {
        if location.starts_with(SCHEME) {
            Self::parse(location)
        } else if location.starts_with('/') && !location.starts_with("//") {
            let mut joined = String::with_capacity(self.path_at + location.len());
            joined.push_str(&self.text[..self.path_at]);
            joined.push_str(location);
            Self::parse(&joined)
        } else {
            Err(UrlError::BadRedirect)
        }
    }
}

impl fmt::Display for HttpsUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.text)
    }
}

impl fmt::Debug for HttpsUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "HttpsUrl({})", self.text)
    }
}

/// No fragment, backslash, quote, angle bracket, space, control or non-ASCII byte anywhere.
fn allowed_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric()
        || matches!(
            b,
            b'-' | b'.'
                | b'_'
                | b'~'
                | b'/'
                | b'?'
                | b'='
                | b'&'
                | b'%'
                | b'+'
                | b':'
                | b','
                | b'!'
                | b'$'
                | b'*'
                | b'('
                | b')'
                | b';'
                | b'@'
        )
}

fn check_host(host: &str) -> Result<(), UrlError> {
    if host.is_empty() || host.len() > 253 {
        return Err(UrlError::BadHost);
    }
    let labels: Vec<&str> = host.split('.').collect();
    // One label is an intranet name or `localhost`. An empty label is `a..b` or a trailing dot,
    // which would slip `localhost.` past the suffix check.
    if labels.len() < 2 {
        return Err(UrlError::BadHost);
    }
    for label in &labels {
        let good = !label.is_empty()
            && label.len() <= 63
            && label
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            && !label.starts_with('-')
            && !label.ends_with('-');
        if !good {
            return Err(UrlError::BadHost);
        }
    }
    // A last label that is numeric makes the whole host an IPv4 address to URL parsers and
    // to `inet_aton` (`127.1`, `0x7f.1`, `2130706433`). No TLD starts with a digit.
    let tld = labels[labels.len() - 1];
    if !tld.as_bytes()[0].is_ascii_lowercase() {
        return Err(UrlError::BadHost);
    }
    for suffix in PRIVATE_SUFFIXES {
        if host == *suffix
            || (host.len() > suffix.len()
                && host.ends_with(suffix)
                && host.as_bytes()[host.len() - suffix.len() - 1] == b'.')
        {
            return Err(UrlError::PrivateHost);
        }
    }
    Ok(())
}

fn check_port(port: &str) -> Result<(), UrlError> {
    let digits_only = !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit());
    if !digits_only || port.len() > 5 || port.starts_with('0') {
        return Err(UrlError::BadPort);
    }
    match port.parse::<u32>() {
        Ok(n) if n <= 65535 => Ok(()),
        _ => Err(UrlError::BadPort),
    }
}

/// `target` starts with `/`. Dot segments and escapes that decode to separators are refused, so
/// no server can normalize the path into something other than what was checked and shown.
fn check_target(target: &str) -> Result<(), UrlError> {
    let path = target.split('?').next().unwrap_or(target);
    if path
        .split('/')
        .any(|segment| segment == "." || segment == "..")
    {
        return Err(UrlError::BadPath);
    }
    let bytes = target.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let (Some(hi), Some(lo)) = (
                bytes.get(i + 1).copied().and_then(hex_value),
                bytes.get(i + 2).copied().and_then(hex_value),
            ) else {
                return Err(UrlError::BadPath);
            };
            let decoded = hi * 16 + lo;
            if !(0x20..0x7f).contains(&decoded) || matches!(decoded, b'.' | b'/' | b'\\' | b'%') {
                return Err(UrlError::BadPath);
            }
            i += 3;
        } else {
            i += 1;
        }
    }
    Ok(())
}

fn hex_value(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_ordinary_urls_and_lowercases_the_host() {
        let u = HttpsUrl::parse("https://Example.ORG:8443/a/b-1.2.omgapp?x=1").unwrap();
        assert_eq!(u.as_str(), "https://example.org:8443/a/b-1.2.omgapp?x=1");
        assert_eq!(u.host(), "example.org");
        assert_eq!(u.target(), "/a/b-1.2.omgapp?x=1");
        assert!(HttpsUrl::parse("https://github.com/o/r/releases/download/app@1.0.0/a").is_ok());
        assert!(HttpsUrl::parse("https://xn--bcher-kva.example/a%20b").is_ok());
    }

    #[test]
    fn refuses_every_other_scheme() {
        for bad in [
            "http://example.org/a",
            "HTTPS://example.org/a",
            "file:///etc/passwd",
            "ftp://example.org/a",
            "content://x/y",
            "//example.org/a",
            "example.org/a",
            "",
        ] {
            assert_eq!(HttpsUrl::parse(bad), Err(UrlError::NotHttps), "{bad}");
        }
    }

    #[test]
    fn refuses_hosts_that_are_not_public_dns_names() {
        for bad in [
            "https://127.0.0.1/a",
            "https://127.1/a",
            "https://2130706433/a",
            "https://0x7f.0.0.1/a",
            "https://0x7f.1/a",
            "https://a.0x7f/a",
            "https://192.168.1.1/a",
            "https://intranet/a",
            "https://example.org./a",
            "https://a..org/a",
            "https://-a.org/a",
            "https://a-.org/a",
            "https://user:pw@example.org/a",
            "https://example.org@evil.org/a",
            "https://:443/a",
        ] {
            assert!(
                matches!(
                    HttpsUrl::parse(bad),
                    Err(UrlError::BadHost | UrlError::BadPort)
                ),
                "{bad}"
            );
        }
        // IPv6 literals die on the bracket.
        assert_eq!(
            HttpsUrl::parse("https://[::1]/a"),
            Err(UrlError::BadCharacter)
        );
    }

    #[test]
    fn refuses_private_network_names() {
        for bad in [
            "https://nas.local/a",
            "https://router.lan/a",
            "https://a.localhost/a",
            "https://NAS.LOCAL/a",
            "https://x.home.arpa/a",
            "https://1.1.168.192.in-addr.arpa/a",
        ] {
            assert_eq!(HttpsUrl::parse(bad), Err(UrlError::PrivateHost), "{bad}");
        }
        // Only whole labels match.
        assert!(HttpsUrl::parse("https://notlocal.org/a").is_ok());
        assert!(HttpsUrl::parse("https://local.org/a").is_ok());
        assert!(HttpsUrl::parse("https://mylan.example/a").is_ok());
    }

    #[test]
    fn refuses_ambiguous_paths_and_characters() {
        for bad in [
            "https://example.org",
            "https://example.org?x=1",
            "https://example.org/a/../b",
            "https://example.org/./b",
            "https://example.org/%2e%2e/b",
            "https://example.org/a%2Fb",
            "https://example.org/a%5Cb",
            "https://example.org/a%00",
            "https://example.org/a%0a",
            "https://example.org/a%25",
            "https://example.org/a%zz",
            "https://example.org/a%2",
        ] {
            assert!(
                matches!(HttpsUrl::parse(bad), Err(UrlError::BadPath)),
                "{bad}"
            );
        }
        for bad in [
            "https://example.org/a b",
            "https://example.org/a#frag",
            "https://example.org\\@evil.org/a",
            "https://example.org/a\nb",
            "https://exämple.org/a",
            "https://example.org/\"a\"",
        ] {
            assert_eq!(HttpsUrl::parse(bad), Err(UrlError::BadCharacter), "{bad}");
        }
        let long = format!("https://example.org/{}", "a".repeat(MAX_URL_LEN));
        assert_eq!(HttpsUrl::parse(&long), Err(UrlError::TooLong));
    }

    #[test]
    fn refuses_bad_ports() {
        for bad in [
            "https://example.org:/a",
            "https://example.org:0/a",
            "https://example.org:080/a",
            "https://example.org:65536/a",
            "https://example.org:999999/a",
            "https://example.org:a/a",
            "https://example.org:1:2/a",
        ] {
            assert_eq!(HttpsUrl::parse(bad), Err(UrlError::BadPort), "{bad}");
        }
    }

    #[test]
    fn redirects_pass_the_same_rules() {
        let u = HttpsUrl::parse("https://example.org:8443/a/b").unwrap();
        assert_eq!(
            u.redirect("/c?d=1").unwrap().as_str(),
            "https://example.org:8443/c?d=1"
        );
        assert_eq!(
            u.redirect("https://cdn.example.net/x").unwrap().host(),
            "cdn.example.net"
        );
        assert_eq!(
            u.redirect("http://cdn.example.net/x"),
            Err(UrlError::BadRedirect)
        );
        assert_eq!(u.redirect("//evil.org/x"), Err(UrlError::BadRedirect));
        assert_eq!(u.redirect("x/y"), Err(UrlError::BadRedirect));
        assert_eq!(u.redirect("/../x"), Err(UrlError::BadPath));
        assert_eq!(
            u.redirect("https://nas.local/x"),
            Err(UrlError::PrivateHost)
        );
    }
}
