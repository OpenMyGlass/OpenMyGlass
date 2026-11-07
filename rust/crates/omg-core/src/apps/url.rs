//! The only URLs the Applications section will ever ask a fetcher for.
//!
//! Every URL here comes from untrusted input: a pasted repository address, a manifest, or a
//! redirect. The rules are deliberately narrower than the URL standard, so that this parser and
//! the platform's HTTP stack cannot read the same string differently: HTTPS only, plain ASCII, a
//! real DNS name, no credentials, no IP addresses, no fragments, and a path with no dot segments or
//! encoded separators. A query is opaque to us beyond well-formed escapes: signed storage URLs
//! carry encoded slashes and colons there.

use std::fmt;

use thiserror::Error;

/// Longest URL accepted from a pasted address or a manifest, in bytes.
pub const MAX_URL_LEN: usize = 1024;
/// Longest URL accepted from a redirect, in bytes. Signed storage URLs are long: a GitHub release
/// asset redirect measured 924 bytes in November 2026, and nothing promises it stays that short.
pub const MAX_REDIRECT_URL_LEN: usize = 8192;

/// Common names for things inside a network or for a public name that resolves to a chosen
/// address. This is a denylist: it makes the casual cases fail and cannot be complete. Any public
/// name can resolve to a private address, which only the fetcher can see: design doc, threat T7.
const PRIVATE_SUFFIXES: &[&str] = &[
    "local",
    "localhost",
    "localdomain",
    "internal",
    "intranet",
    "private",
    "lan",
    "home",
    "corp",
    "test",
    "invalid",
    "ip",
    "home.arpa",
    "in-addr.arpa",
    "ip6.arpa",
    "fritz.box",
    "nip.io",
    "sslip.io",
    "xip.io",
    "localtest.me",
    "lvh.me",
];

const HTTPS_DEFAULT_PORT: &str = "443";

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
    #[error("URL has an invalid path or query")]
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
    /// Checks `input` against every rule and returns its canonical form: lowercase host, no
    /// `:443`.
    pub fn parse(input: &str) -> Result<Self, UrlError> {
        Self::parse_limited(input, MAX_URL_LEN)
    }

    fn parse_limited(input: &str, max_len: usize) -> Result<Self, UrlError> {
        if input.len() > max_len {
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
        let port = port.filter(|port| *port != HTTPS_DEFAULT_PORT);
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

    /// The host, lowercase, without the port. To show where something comes from use
    /// [`origin`](Self::origin) or [`super::RepositorySource::label`]: a port is another server.
    pub fn host(&self) -> &str {
        &self.text[SCHEME.len()..self.host_end]
    }

    /// The host and any port other than 443.
    pub fn origin(&self) -> &str {
        &self.text[SCHEME.len()..self.path_at]
    }

    /// The path and the query, starting with `/`.
    pub fn target(&self) -> &str {
        &self.text[self.path_at..]
    }

    /// Resolves the `Location` of a redirect from this URL. The result passes the same rules.
    pub fn redirect(&self, location: &str) -> Result<Self, UrlError> {
        if location.starts_with(SCHEME) {
            Self::parse_limited(location, MAX_REDIRECT_URL_LEN)
        } else if location.starts_with('/') && !location.starts_with("//") {
            let mut joined = String::with_capacity(self.path_at + location.len());
            joined.push_str(&self.text[..self.path_at]);
            joined.push_str(location);
            Self::parse_limited(&joined, MAX_REDIRECT_URL_LEN)
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

/// `target` starts with `/`. Path rules apply only to the path. They stop the usual ways of making
/// a server normalize a path into another one (`..`, `..;`, `..%20`, encoded separators, overlong
/// UTF-8); no list of this kind is a proof, which is fine because a path decides nothing here: the
/// size, hash and signature do. A query only has to have well-formed escapes.
fn check_target(target: &str) -> Result<(), UrlError> {
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let bad_segment = |segment: &str| {
        segment == "."
            || segment.starts_with("..")
            || segment.starts_with(".;")
            || segment.starts_with(".%")
    };
    if path.split('/').any(bad_segment) {
        return Err(UrlError::BadPath);
    }
    for decoded in escapes(path)? {
        let forbidden = decoded < 0x20
            || matches!(
                decoded,
                0x7f | 0xc0 | 0xc1 | b'.' | b'/' | b'\\' | b'%' | b'?' | b'#' | b';'
            );
        if forbidden {
            return Err(UrlError::BadPath);
        }
    }
    escapes(query)?;
    Ok(())
}

/// The bytes the percent escapes of `text` decode to, or an error if one is malformed.
fn escapes(text: &str) -> Result<Vec<u8>, UrlError> {
    let bytes = text.as_bytes();
    let mut decoded = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let (Some(hi), Some(lo)) = (
                bytes.get(i + 1).copied().and_then(hex_value),
                bytes.get(i + 2).copied().and_then(hex_value),
            ) else {
                return Err(UrlError::BadPath);
            };
            decoded.push(hi * 16 + lo);
            i += 3;
        } else {
            i += 1;
        }
    }
    Ok(decoded)
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
        assert!(HttpsUrl::parse("https://h.example/caf%C3%A9.omgapp").is_ok());
        assert!(HttpsUrl::parse("https://h.example/.well-known/x").is_ok());
        assert_eq!(u.origin(), "example.org:8443");
    }

    #[test]
    fn the_default_port_has_one_spelling() {
        let u = HttpsUrl::parse("https://example.org:443/a").unwrap();
        assert_eq!(u.as_str(), "https://example.org/a");
        assert_eq!(u.origin(), "example.org");
    }

    #[test]
    fn a_query_is_opaque_beyond_well_formed_escapes() {
        for good in [
            "https://h.example/a?rsct=application%2Foctet-stream",
            "https://h.example/a?X-Amz-Credential=AKIA%2F2026%2Fus-east-1",
            "https://h.example/a?se=2026-11-07T01%3A02%3A20Z&x=%2e%2e%00",
            "https://h.example/a?path=/b/../c",
        ] {
            assert!(HttpsUrl::parse(good).is_ok(), "{good}");
        }
        for bad in ["https://h.example/a?x=%zz", "https://h.example/a?x=%2"] {
            assert_eq!(HttpsUrl::parse(bad), Err(UrlError::BadPath), "{bad}");
        }
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
            "https://fritz.box/a",
            "https://speedport.ip/a",
            "https://a.test/a",
            "https://192.168.1.1.nip.io/a",
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
            "https://example.org/a/..;/b",
            "https://example.org/..%20/b",
            "https://example.org/.%2e/b",
            "https://example.org/%c0%ae/b",
            "https://example.org/a%3fb",
            "https://example.org/a%23b",
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
        // The same length is fine for a redirect, which has its own larger limit.
        let from = HttpsUrl::parse("https://example.org/a").unwrap();
        assert!(from.redirect(&long).is_ok());
        let longer = format!("https://example.org/{}", "a".repeat(MAX_REDIRECT_URL_LEN));
        assert_eq!(from.redirect(&longer), Err(UrlError::TooLong));
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
