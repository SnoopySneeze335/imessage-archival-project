//! Sign-in: one password, session cookies, the first-run setup code, and the
//! request checks around them (decision 0008).
//!
//! The app works over plain HTTP (LAN, tailnet) and behind an HTTPS reverse
//! proxy; neither blocks the other. The cookie follows the request's scheme,
//! and `X-Forwarded-*` headers count only from a configured proxy address.

use std::{
    collections::HashMap,
    net::IpAddr,
    str::FromStr,
    time::{Duration, Instant},
};

use anyhow::Result;
use argon2::{
    Argon2,
    password_hash::{PasswordHasher, PasswordVerifier, phc::PasswordHash},
};
use axum::http::{HeaderMap, Method, header};
use sha2::{Digest, Sha256};

/// Cookie name over HTTPS. The `__Host-` prefix makes the browser insist on
/// Secure, Path=/ and no Domain.
pub const COOKIE_HTTPS: &str = "__Host-session";
/// Cookie name over plain HTTP, where Secure would make the browser drop it.
pub const COOKIE_HTTP: &str = "session";

/// Shortest password accepted.
pub const MIN_PASSWORD_CHARS: usize = 8;
/// Longest password accepted, so hashing cost stays bounded.
pub const MAX_PASSWORD_BYTES: usize = 1024;

/// Argon2id with the crate's defaults (OWASP's recommended parameters).
pub fn hash_password(password: &str) -> Result<String> {
    let hash = Argon2::default()
        .hash_password(password.as_bytes())
        .map_err(|e| anyhow::anyhow!("hashing the password: {e}"))?;
    Ok(hash.to_string())
}

/// Whether `password` matches a stored PHC string.
pub fn verify_password(password: &str, stored: &str) -> bool {
    match PasswordHash::new(stored) {
        Ok(parsed) => Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .is_ok(),
        Err(_) => false,
    }
}

/// Checks a new password against the length rules.
pub fn validate_new_password(password: &str) -> Result<(), String> {
    if password.chars().count() < MIN_PASSWORD_CHARS {
        return Err(format!("Use at least {MIN_PASSWORD_CHARS} characters."));
    }
    if password.len() > MAX_PASSWORD_BYTES {
        return Err(format!("Use at most {MAX_PASSWORD_BYTES} bytes."));
    }
    Ok(())
}

fn random_bytes<const N: usize>() -> [u8; N] {
    let mut b = [0u8; N];
    getrandom::fill(&mut b).expect("the operating system's random number generator");
    b
}

/// A new session token: 256 random bits, hex.
pub fn new_token() -> String {
    crate::blobs::hex(&random_bytes::<32>())
}

/// What user.db stores for a token.
pub fn token_hash(token: &str) -> String {
    crate::blobs::hex(&Sha256::digest(token.as_bytes()))
}

/// Letters and digits that cannot be mistaken for one another.
const CODE_ALPHABET: &[u8] = b"ABCDEFGHJKMNPQRSTUVWXYZ23456789";

/// A first-run setup code such as `K7QM-3XRP` (about 40 bits).
pub fn new_setup_code() -> String {
    let bytes = random_bytes::<8>();
    let chars: String = bytes
        .iter()
        .map(|b| CODE_ALPHABET[usize::from(*b) % CODE_ALPHABET.len()] as char)
        .collect();
    format!("{}-{}", &chars[..4], &chars[4..])
}

/// Compares a typed setup code with the real one, ignoring case, spaces and
/// dashes.
pub fn setup_code_matches(typed: &str, code: &str) -> bool {
    let norm = |s: &str| -> String {
        s.chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .map(|c| c.to_ascii_uppercase())
            .collect()
    };
    let (a, b) = (norm(typed), norm(code));
    // Constant-time over equal lengths.
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .fold(0u8, |acc, (x, y)| acc | (x ^ y))
            == 0
}

/// An IP address or CIDR range of a trusted reverse proxy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IpRange {
    addr: IpAddr,
    prefix: u8,
}

impl FromStr for IpRange {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, String> {
        let s = s.trim();
        let (addr, prefix) = match s.split_once('/') {
            Some((a, p)) => (a, Some(p)),
            None => (s, None),
        };
        let addr: IpAddr = addr
            .parse()
            .map_err(|_| format!("not an IP address or CIDR range: {s}"))?;
        let max = if addr.is_ipv4() { 32 } else { 128 };
        let prefix = match prefix {
            Some(p) => p
                .parse::<u8>()
                .ok()
                .filter(|&p| p <= max)
                .ok_or_else(|| format!("bad prefix length in {s}"))?,
            None => max,
        };
        Ok(Self { addr, prefix })
    }
}

impl IpRange {
    pub fn contains(&self, ip: IpAddr) -> bool {
        let ip = match ip {
            IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(ip, IpAddr::V4),
            v4 => v4,
        };
        match (self.addr, ip) {
            (IpAddr::V4(a), IpAddr::V4(b)) => {
                let mask = u32::MAX
                    .checked_shl(32 - u32::from(self.prefix))
                    .unwrap_or(0);
                u32::from(a) & mask == u32::from(b) & mask
            }
            (IpAddr::V6(a), IpAddr::V6(b)) => {
                let mask = u128::MAX
                    .checked_shl(128 - u32::from(self.prefix))
                    .unwrap_or(0);
                u128::from(a) & mask == u128::from(b) & mask
            }
            _ => false,
        }
    }
}

/// Parses `IMESSAGE_AP_TRUSTED_PROXIES`: comma-separated addresses or ranges.
pub fn parse_proxies(s: &str) -> Result<Vec<IpRange>, String> {
    s.split(',')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(IpRange::from_str)
        .collect()
}

/// What the server knows about who sent a request and how.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Origin {
    /// The browser used HTTPS (directly, or through a trusted proxy that said
    /// so in `X-Forwarded-Proto`).
    pub https: bool,
    /// The client's address, for rate limiting.
    pub client: Option<IpAddr>,
    /// The host the browser asked for, for the Origin check.
    pub host: Option<String>,
}

fn header_str<'a>(h: &'a HeaderMap, name: &str) -> Option<&'a str> {
    h.get(name).and_then(|v| v.to_str().ok())
}

/// Reads scheme, client and host from a request. Forwarded headers count
/// only when the connection comes from a trusted proxy.
pub fn origin(headers: &HeaderMap, peer: Option<IpAddr>, trusted: &[IpRange]) -> Origin {
    let via_proxy = peer.is_some_and(|p| trusted.iter().any(|r| r.contains(p)));
    let host = header_str(headers, header::HOST.as_str()).map(str::to_owned);
    if !via_proxy {
        return Origin {
            https: false,
            client: peer,
            host,
        };
    }
    let first = |name: &str| {
        header_str(headers, name)
            .and_then(|v| v.split(',').next())
            .map(|v| v.trim().to_owned())
    };
    let https = first("x-forwarded-proto").is_some_and(|p| p.eq_ignore_ascii_case("https"));
    // The proxy appends the address it saw, so the last entry is the one
    // the trusted proxy vouches for.
    let client = header_str(headers, "x-forwarded-for")
        .and_then(|v| v.rsplit(',').next())
        .and_then(|v| v.trim().parse().ok())
        .or(peer);
    Origin {
        https,
        client,
        host: first("x-forwarded-host").or(host),
    }
}

/// `host[:port]` with the scheme's default port removed, lowercased.
fn normalize_authority(authority: &str, https: bool) -> String {
    let a = authority.trim().to_ascii_lowercase();
    let default = if https { ":443" } else { ":80" };
    a.strip_suffix(default).map_or(a.clone(), str::to_owned)
}

/// The cross-site request check for requests that change something: the
/// browser's `Origin` must name the host the request was sent to, whatever
/// that host is (LAN address, server name or tailnet name). Without an
/// `Origin` header, `Sec-Fetch-Site: same-origin` is accepted instead.
pub fn same_origin(method: &Method, headers: &HeaderMap, origin: &Origin) -> bool {
    if matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS) {
        return true;
    }
    match header_str(headers, header::ORIGIN.as_str()) {
        Some(o) => {
            let Some((scheme, authority)) = o.split_once("://") else {
                return false;
            };
            let o_https = scheme.eq_ignore_ascii_case("https");
            match &origin.host {
                Some(host) => {
                    // The browser's scheme decides the default port on both
                    // sides: behind a proxy the Host header has none.
                    normalize_authority(authority, o_https) == normalize_authority(host, o_https)
                }
                None => false,
            }
        }
        None => header_str(headers, "sec-fetch-site") == Some("same-origin"),
    }
}

/// The session token from a request's cookies, if any.
pub fn session_token(headers: &HeaderMap) -> Option<String> {
    let mut found = None;
    for value in headers.get_all(header::COOKIE) {
        let Ok(value) = value.to_str() else { continue };
        for pair in value.split(';') {
            if let Some((name, v)) = pair.trim().split_once('=') {
                let v = v.trim();
                let well_formed = v.len() == 64 && v.bytes().all(|b| b.is_ascii_hexdigit());
                if well_formed && (name == COOKIE_HTTPS || (name == COOKIE_HTTP && found.is_none()))
                {
                    found = Some(v.to_owned());
                }
            }
        }
    }
    found
}

/// The `Set-Cookie` value for a new session.
pub fn session_cookie(token: &str, https: bool) -> String {
    let max_age = crate::userdb::SESSION_LIFETIME_NS / 1_000_000_000;
    if https {
        format!(
            "{COOKIE_HTTPS}={token}; Path=/; Max-Age={max_age}; Secure; HttpOnly; SameSite=Strict"
        )
    } else {
        format!("{COOKIE_HTTP}={token}; Path=/; Max-Age={max_age}; HttpOnly; SameSite=Strict")
    }
}

/// `Set-Cookie` values that remove both cookies.
pub fn clear_cookies() -> [String; 2] {
    [
        format!("{COOKIE_HTTPS}=; Path=/; Max-Age=0; Secure; HttpOnly; SameSite=Strict"),
        format!("{COOKIE_HTTP}=; Path=/; Max-Age=0; HttpOnly; SameSite=Strict"),
    ]
}

/// Slows down password and setup-code guessing, per client address: five
/// free failures, then a lockout that doubles with each further failure,
/// up to 15 minutes. A success clears the client's record.
#[derive(Debug, Default)]
pub struct RateLimiter {
    clients: HashMap<Option<IpAddr>, (u32, Option<Instant>)>,
}

const FREE_FAILURES: u32 = 5;
const MAX_LOCKOUT: Duration = Duration::from_secs(15 * 60);

impl RateLimiter {
    /// `Err(wait)` while the client is locked out.
    pub fn check(&self, client: Option<IpAddr>, now: Instant) -> Result<(), Duration> {
        match self.clients.get(&client) {
            Some((_, Some(until))) if *until > now => Err(*until - now),
            _ => Ok(()),
        }
    }

    pub fn failed(&mut self, client: Option<IpAddr>, now: Instant) {
        if self.clients.len() > 10_000 {
            self.clients
                .retain(|_, (_, until)| until.is_some_and(|u| u > now));
        }
        let entry = self.clients.entry(client).or_insert((0, None));
        entry.0 += 1;
        if entry.0 >= FREE_FAILURES {
            let doublings = (entry.0 - FREE_FAILURES).min(10);
            let wait = Duration::from_secs(1u64 << doublings).min(MAX_LOCKOUT);
            entry.1 = Some(now + wait);
        }
    }

    pub fn succeeded(&mut self, client: Option<IpAddr>) {
        self.clients.remove(&client);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn headers(pairs: &[(&'static str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.append(*k, HeaderValue::from_str(v).unwrap());
        }
        h
    }

    #[test]
    fn passwords_hash_and_verify() {
        let h = hash_password("correct horse").unwrap();
        assert!(h.starts_with("$argon2id$"));
        assert!(verify_password("correct horse", &h));
        assert!(!verify_password("wrong horse", &h));
        assert!(!verify_password("x", "not a hash"));
        assert!(validate_new_password("short").is_err());
        assert!(validate_new_password("long enough").is_ok());
    }

    #[test]
    fn setup_codes_match_loosely() {
        let code = new_setup_code();
        assert_eq!(code.len(), 9);
        assert!(setup_code_matches(
            &code.to_lowercase().replace('-', " "),
            &code
        ));
        assert!(!setup_code_matches("AAAA-AAAA", "AAAA-AAAB"));
        assert!(!setup_code_matches("", &code));
    }

    #[test]
    fn ranges_match_addresses() {
        let r: IpRange = "172.17.0.0/16".parse().unwrap();
        assert!(r.contains("172.17.0.1".parse().unwrap()));
        assert!(!r.contains("172.18.0.1".parse().unwrap()));
        assert!(r.contains("::ffff:172.17.3.4".parse().unwrap()));
        let one: IpRange = "100.64.0.7".parse().unwrap();
        assert!(one.contains("100.64.0.7".parse().unwrap()));
        assert!(!one.contains("100.64.0.8".parse().unwrap()));
        assert!(
            "fd7a:115c::/48"
                .parse::<IpRange>()
                .unwrap()
                .contains("fd7a:115c::1".parse().unwrap())
        );
        assert!("10.0.0.0/33".parse::<IpRange>().is_err());
        assert!(parse_proxies("").unwrap().is_empty());
        assert_eq!(parse_proxies(" 10.0.0.1 , 10.0.1.0/24").unwrap().len(), 2);
    }

    #[test]
    fn forwarded_headers_count_only_from_a_trusted_proxy() {
        let trusted = parse_proxies("172.17.0.1").unwrap();
        let h = headers(&[
            ("host", "192.168.1.20:8080"),
            ("x-forwarded-proto", "https"),
            ("x-forwarded-for", "1.2.3.4, 100.64.0.9"),
            ("x-forwarded-host", "snoopy.example.ts.net"),
        ]);
        let direct = origin(&h, Some("192.168.1.30".parse().unwrap()), &trusted);
        assert!(!direct.https);
        assert_eq!(direct.client, Some("192.168.1.30".parse().unwrap()));
        assert_eq!(direct.host.as_deref(), Some("192.168.1.20:8080"));
        let proxied = origin(&h, Some("172.17.0.1".parse().unwrap()), &trusted);
        assert!(proxied.https);
        assert_eq!(proxied.client, Some("100.64.0.9".parse().unwrap()));
        assert_eq!(proxied.host.as_deref(), Some("snoopy.example.ts.net"));
    }

    #[test]
    fn origin_must_name_the_requested_host() {
        let o = |host: &str| Origin {
            https: false,
            client: None,
            host: Some(host.to_owned()),
        };
        let post = Method::POST;
        for (origin_hdr, host, ok) in [
            ("http://192.168.1.20:8080", "192.168.1.20:8080", true),
            ("http://snoopyserver:8080", "snoopyserver:8080", true),
            ("http://SnoopyServer:8080", "snoopyserver:8080", true),
            (
                "https://snoopy.example.ts.net",
                "snoopy.example.ts.net",
                true,
            ),
            (
                "https://snoopy.example.ts.net",
                "snoopy.example.ts.net:443",
                true,
            ),
            ("http://evil.example.com", "192.168.1.20:8080", false),
            ("http://192.168.1.20:9090", "192.168.1.20:8080", false),
            ("null", "192.168.1.20:8080", false),
        ] {
            let h = headers(&[("origin", origin_hdr)]);
            assert_eq!(
                same_origin(&post, &h, &o(host)),
                ok,
                "{origin_hdr} vs {host}"
            );
        }
        assert!(same_origin(&Method::GET, &HeaderMap::new(), &o("x")));
        assert!(!same_origin(&post, &HeaderMap::new(), &o("x")));
        assert!(same_origin(
            &post,
            &headers(&[("sec-fetch-site", "same-origin")]),
            &o("x")
        ));
    }

    #[test]
    fn cookies_follow_the_scheme() {
        let t = new_token();
        assert_eq!(t.len(), 64);
        let s = session_cookie(&t, true);
        assert!(s.starts_with("__Host-session=") && s.contains("; Secure;"));
        let p = session_cookie(&t, false);
        assert!(p.starts_with("session=") && !p.contains("Secure"));
        assert!(p.contains("HttpOnly") && p.contains("SameSite=Strict"));
        let h = headers(&[("cookie", &format!("a=b; session={t}"))]);
        assert_eq!(session_token(&h).as_deref(), Some(t.as_str()));
        let junk = headers(&[("cookie", "session=<script>")]);
        assert_eq!(session_token(&junk), None);
    }

    #[test]
    fn guessing_is_slowed_down() {
        let mut rl = RateLimiter::default();
        let ip = Some("10.0.0.1".parse().unwrap());
        let other = Some("10.0.0.2".parse().unwrap());
        let now = Instant::now();
        for _ in 0..4 {
            rl.failed(ip, now);
            assert!(rl.check(ip, now).is_ok());
        }
        rl.failed(ip, now);
        assert!(rl.check(ip, now).is_err());
        assert!(rl.check(ip, now + Duration::from_secs(2)).is_ok());
        rl.failed(ip, now);
        assert_eq!(rl.check(ip, now), Err(Duration::from_secs(2)));
        assert!(rl.check(other, now).is_ok());
        for _ in 0..30 {
            rl.failed(ip, now);
        }
        assert!(rl.check(ip, now).unwrap_err() <= MAX_LOCKOUT);
        rl.succeeded(ip);
        assert!(rl.check(ip, now).is_ok());
    }
}
