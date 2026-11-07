//! A SOCKS5 client: RFC 1928 (CONNECT) and RFC 1929 (username/password authentication).
//!
//! Only what a game client needs is implemented: the `CONNECT` command, the "no
//! authentication" and "username/password" methods, and all three address types. Domain targets
//! are always sent to the proxy as `DOMAINNAME` (remote DNS, like curl's `socks5h://`); this module
//! never resolves anything.
//!
//! Most applications use [`crate::connect::Connector`], which adds TCP connect, timeouts and the
//! proxy policy. [`handshake`] is the bare negotiation over any byte stream.
//!
//! Every reply from the proxy is validated; a malformed, truncated or unexpected reply yields a
//! [`Socks5Error`], never a panic. Exactly the bytes of the replies are read from the stream, so
//! data the proxy forwards right after its CONNECT reply stays in the stream.

use std::fmt;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::str::FromStr;

use thiserror::Error;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::SecretString;
use crate::connect::{AddrError, TargetAddr, validate_domain};

const VERSION: u8 = 0x05;
const AUTH_VERSION: u8 = 0x01;
const METHOD_NO_AUTH: u8 = 0x00;
const METHOD_USER_PASS: u8 = 0x02;
const METHOD_NONE_ACCEPTABLE: u8 = 0xff;
const CMD_CONNECT: u8 = 0x01;
const ATYP_IPV4: u8 = 0x01;
const ATYP_DOMAIN: u8 = 0x03;
const ATYP_IPV6: u8 = 0x04;

/// Default SOCKS port, used when a proxy URL has none.
pub const DEFAULT_SOCKS_PORT: u16 = 1080;

/// Username/password credentials for RFC 1929 authentication.
///
/// Both parts must be 1 to 255 bytes long (the protocol's length prefix is one byte). The
/// password is never printed by `Debug`.
#[derive(Clone, PartialEq, Eq)]
pub struct Socks5Auth {
    username: String,
    password: SecretString,
}

impl Socks5Auth {
    /// Creates credentials, checking the RFC 1929 length limits.
    pub fn new(
        username: impl Into<String>,
        password: impl Into<SecretString>,
    ) -> Result<Self, CredentialsError> {
        let username = username.into();
        let password = password.into();
        if username.is_empty() || username.len() > 255 {
            return Err(CredentialsError::Username(username.len()));
        }
        let plen = password.expose().len();
        if plen == 0 || plen > 255 {
            return Err(CredentialsError::Password(plen));
        }
        Ok(Self { username, password })
    }

    /// The username.
    pub fn username(&self) -> &str {
        &self.username
    }

    /// The password.
    pub fn password(&self) -> &SecretString {
        &self.password
    }
}

impl fmt::Debug for Socks5Auth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Socks5Auth")
            .field("username", &self.username)
            .field("password", &self.password)
            .finish()
    }
}

/// Credentials that violate the RFC 1929 limits.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CredentialsError {
    /// The username is empty or longer than 255 bytes.
    #[error("SOCKS5 username must be 1 to 255 bytes long, got {0}")]
    Username(usize),
    /// The password is empty or longer than 255 bytes.
    #[error("SOCKS5 password must be 1 to 255 bytes long, got {0}")]
    Password(usize),
}

/// A SOCKS5 proxy: its address and optional credentials.
///
/// Can be parsed from a URL: `socks5h://user:pass@host:port`. The schemes `socks5h://` and
/// `socks5://` are both accepted and behave the same: ghast always resolves targets on the proxy.
/// Percent-encoding (`%40` for `@`, ...) is decoded in the username and password. The port
/// defaults to [`DEFAULT_SOCKS_PORT`]. A URL without a scheme (`host:port`) is accepted too.
///
/// `Debug` and `Display` never show the password.
#[derive(Clone, PartialEq, Eq)]
pub struct Socks5Proxy {
    addr: TargetAddr,
    auth: Option<Socks5Auth>,
}

impl Socks5Proxy {
    /// A proxy without authentication.
    pub fn new(addr: TargetAddr) -> Self {
        Self { addr, auth: None }
    }

    /// Sets the credentials.
    pub fn with_auth(mut self, auth: Socks5Auth) -> Self {
        self.auth = Some(auth);
        self
    }

    /// The proxy's address. A domain name here is resolved locally (it is the proxy itself, not
    /// the game server).
    pub fn addr(&self) -> &TargetAddr {
        &self.addr
    }

    /// The credentials, if any.
    pub fn auth(&self) -> Option<&Socks5Auth> {
        self.auth.as_ref()
    }
}

impl fmt::Debug for Socks5Proxy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Socks5Proxy")
            .field("addr", &self.addr)
            .field("auth", &self.auth)
            .finish()
    }
}

impl fmt::Display for Socks5Proxy {
    /// `socks5h://user@host:port` (no password).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("socks5h://")?;
        if let Some(auth) = &self.auth {
            write!(f, "{}@", auth.username)?;
        }
        write!(f, "{}", self.addr)
    }
}

/// Why a proxy URL could not be parsed.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum ProxyUrlError {
    /// The scheme is not `socks5://` or `socks5h://`.
    #[error("unsupported proxy scheme {0:?} (expected socks5:// or socks5h://)")]
    Scheme(String),
    /// The URL has a path, query or fragment.
    #[error("proxy URL must not have a path, query or fragment")]
    TrailingData,
    /// A percent escape is malformed or does not decode to UTF-8.
    #[error("invalid percent-encoding in proxy credentials")]
    PercentEncoding,
    /// The user info has a username but no password (or the reverse).
    #[error("proxy credentials need both a username and a password")]
    IncompleteCredentials,
    /// The credentials violate the RFC 1929 limits.
    #[error(transparent)]
    Credentials(#[from] CredentialsError),
    /// The host or port is invalid.
    #[error(transparent)]
    Addr(#[from] AddrError),
}

impl FromStr for Socks5Proxy {
    type Err = ProxyUrlError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();
        let rest = match s.split_once("://") {
            Some((scheme, rest)) => {
                if !(scheme.eq_ignore_ascii_case("socks5")
                    || scheme.eq_ignore_ascii_case("socks5h"))
                {
                    return Err(ProxyUrlError::Scheme(scheme.to_owned()));
                }
                rest
            }
            None => s,
        };
        let rest = rest.strip_suffix('/').unwrap_or(rest);
        if rest.contains(['/', '?', '#']) {
            return Err(ProxyUrlError::TrailingData);
        }
        let (userinfo, hostport) = match rest.rsplit_once('@') {
            Some((u, h)) => (Some(u), h),
            None => (None, rest),
        };
        let addr = TargetAddr::parse_with_default_port(hostport, DEFAULT_SOCKS_PORT)?;
        let mut proxy = Socks5Proxy::new(addr);
        if let Some(userinfo) = userinfo {
            let (user, pass) = userinfo
                .split_once(':')
                .ok_or(ProxyUrlError::IncompleteCredentials)?;
            let user = percent_decode(user)?;
            let pass = percent_decode(pass)?;
            if user.is_empty() || pass.is_empty() {
                return Err(ProxyUrlError::IncompleteCredentials);
            }
            proxy = proxy.with_auth(Socks5Auth::new(user, pass)?);
        }
        Ok(proxy)
    }
}

fn percent_decode(s: &str) -> Result<String, ProxyUrlError> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = bytes
                .get(i + 1..i + 3)
                .ok_or(ProxyUrlError::PercentEncoding)?;
            let hex = std::str::from_utf8(hex).map_err(|_| ProxyUrlError::PercentEncoding)?;
            let byte = u8::from_str_radix(hex, 16).map_err(|_| ProxyUrlError::PercentEncoding)?;
            out.push(byte);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).map_err(|_| ProxyUrlError::PercentEncoding)
}

/// The `REP` field of a failed CONNECT reply (RFC 1928 §6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReplyCode {
    /// `0x01` general SOCKS server failure.
    GeneralFailure,
    /// `0x02` connection not allowed by ruleset.
    NotAllowed,
    /// `0x03` network unreachable.
    NetworkUnreachable,
    /// `0x04` host unreachable (also what most proxies answer when remote DNS fails).
    HostUnreachable,
    /// `0x05` connection refused.
    ConnectionRefused,
    /// `0x06` TTL expired.
    TtlExpired,
    /// `0x07` command not supported.
    CommandNotSupported,
    /// `0x08` address type not supported.
    AddressTypeNotSupported,
    /// Any other non-zero code.
    Other(u8),
}

impl ReplyCode {
    /// Maps a non-zero `REP` byte.
    pub fn from_byte(b: u8) -> Self {
        match b {
            0x01 => Self::GeneralFailure,
            0x02 => Self::NotAllowed,
            0x03 => Self::NetworkUnreachable,
            0x04 => Self::HostUnreachable,
            0x05 => Self::ConnectionRefused,
            0x06 => Self::TtlExpired,
            0x07 => Self::CommandNotSupported,
            0x08 => Self::AddressTypeNotSupported,
            other => Self::Other(other),
        }
    }

    /// The raw `REP` byte.
    pub fn as_byte(self) -> u8 {
        match self {
            Self::GeneralFailure => 0x01,
            Self::NotAllowed => 0x02,
            Self::NetworkUnreachable => 0x03,
            Self::HostUnreachable => 0x04,
            Self::ConnectionRefused => 0x05,
            Self::TtlExpired => 0x06,
            Self::CommandNotSupported => 0x07,
            Self::AddressTypeNotSupported => 0x08,
            Self::Other(b) => b,
        }
    }
}

impl fmt::Display for ReplyCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::GeneralFailure => f.write_str("general SOCKS server failure"),
            Self::NotAllowed => f.write_str("connection not allowed by ruleset"),
            Self::NetworkUnreachable => f.write_str("network unreachable"),
            Self::HostUnreachable => f.write_str("host unreachable"),
            Self::ConnectionRefused => f.write_str("connection refused"),
            Self::TtlExpired => f.write_str("TTL expired"),
            Self::CommandNotSupported => f.write_str("command not supported"),
            Self::AddressTypeNotSupported => f.write_str("address type not supported"),
            Self::Other(b) => write!(f, "unknown reply code {b:#04x}"),
        }
    }
}

/// The address the proxy reports in a successful CONNECT reply (`BND.ADDR`/`BND.PORT`).
///
/// Informational only; many proxies report `0.0.0.0:0`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum BoundAddr {
    /// An IP address and port.
    Ip(SocketAddr),
    /// A domain name (decoded lossily if it is not UTF-8) and port.
    Domain(String, u16),
}

/// Errors from the SOCKS5 negotiation.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum Socks5Error {
    /// The proxy closed the connection in the middle of a reply.
    #[error("proxy closed the connection during the handshake")]
    UnexpectedEof,
    /// A socket error.
    #[error("I/O error: {0}")]
    Io(#[source] std::io::Error),
    /// The proxy replied with a SOCKS version other than 5 (it is probably not a SOCKS5 proxy).
    #[error("proxy replied with SOCKS version {0:#04x}, expected 0x05")]
    BadVersion(u8),
    /// The proxy accepts none of the offered methods. With credentials configured this means it
    /// wants another method; without credentials it usually means it requires authentication.
    #[error("proxy accepts none of the offered authentication methods")]
    NoAcceptableMethod,
    /// The proxy selected a method that was not offered.
    #[error("proxy selected authentication method {0:#04x}, which was not offered")]
    UnexpectedMethod(u8),
    /// The username/password sub-negotiation reply has the wrong version byte.
    #[error("proxy replied with authentication version {0:#04x}, expected 0x01")]
    BadAuthVersion(u8),
    /// The proxy rejected the username/password.
    #[error("proxy rejected the credentials (status {0:#04x})")]
    AuthRejected(u8),
    /// The proxy refused or failed the CONNECT.
    #[error("proxy refused the connection: {0}")]
    Reply(ReplyCode),
    /// The CONNECT reply has a non-zero reserved byte.
    #[error("proxy reply has non-zero reserved byte {0:#04x}")]
    BadReserved(u8),
    /// The CONNECT reply uses an unknown address type.
    #[error("proxy reply has unknown address type {0:#04x}")]
    UnknownAddressType(u8),
    /// The target domain cannot be encoded (empty, too long or invalid characters).
    #[error("invalid target: {0}")]
    InvalidTarget(#[from] AddrError),
}

impl From<std::io::Error> for Socks5Error {
    fn from(e: std::io::Error) -> Self {
        if e.kind() == std::io::ErrorKind::UnexpectedEof {
            Self::UnexpectedEof
        } else {
            Self::Io(e)
        }
    }
}

/// Runs the SOCKS5 negotiation on an already-connected stream to the proxy and asks it to
/// CONNECT to `target`.
///
/// On success the stream carries the tunnelled connection. A domain target is sent as a name and
/// resolved by the proxy. This function has no timeout of its own; wrap it in
/// [`tokio::time::timeout`] (as [`crate::connect::Connector`] does).
pub async fn handshake<S>(
    stream: &mut S,
    target: &TargetAddr,
    auth: Option<&Socks5Auth>,
) -> Result<BoundAddr, Socks5Error>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    // Encode the request first so an invalid target fails before anything is sent.
    let request = encode_connect(target)?;

    // Method negotiation.
    let greeting: &[u8] = if auth.is_some() {
        &[VERSION, 2, METHOD_NO_AUTH, METHOD_USER_PASS]
    } else {
        &[VERSION, 1, METHOD_NO_AUTH]
    };
    stream.write_all(greeting).await?;
    stream.flush().await?;
    let mut reply = [0u8; 2];
    stream.read_exact(&mut reply).await?;
    if reply[0] != VERSION {
        return Err(Socks5Error::BadVersion(reply[0]));
    }
    match (reply[1], auth) {
        (METHOD_NO_AUTH, _) => {}
        (METHOD_USER_PASS, Some(auth)) => authenticate(stream, auth).await?,
        (METHOD_NONE_ACCEPTABLE, _) => return Err(Socks5Error::NoAcceptableMethod),
        (other, _) => return Err(Socks5Error::UnexpectedMethod(other)),
    }

    // CONNECT.
    stream.write_all(&request).await?;
    stream.flush().await?;
    let mut head = [0u8; 4];
    stream.read_exact(&mut head).await?;
    let [ver, rep, rsv, atyp] = head;
    if ver != VERSION {
        return Err(Socks5Error::BadVersion(ver));
    }
    if rep != 0 {
        // The rest of a failure reply is irrelevant; the caller drops the connection.
        return Err(Socks5Error::Reply(ReplyCode::from_byte(rep)));
    }
    if rsv != 0 {
        return Err(Socks5Error::BadReserved(rsv));
    }
    read_bound_addr(stream, atyp).await
}

async fn authenticate<S>(stream: &mut S, auth: &Socks5Auth) -> Result<(), Socks5Error>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let user = auth.username.as_bytes();
    let pass = auth.password.expose().as_bytes();
    // `Socks5Auth::new` guarantees 1..=255 bytes for both (the fields are private), so the
    // length casts below are lossless.
    debug_assert!((1..=255).contains(&user.len()) && (1..=255).contains(&pass.len()));
    let mut msg = Vec::with_capacity(3 + user.len() + pass.len());
    msg.push(AUTH_VERSION);
    msg.push(user.len() as u8);
    msg.extend_from_slice(user);
    msg.push(pass.len() as u8);
    msg.extend_from_slice(pass);
    stream.write_all(&msg).await?;
    stream.flush().await?;
    let mut reply = [0u8; 2];
    stream.read_exact(&mut reply).await?;
    // RFC 1929 says the version is 0x01; a few servers echo 0x05. Accept both.
    if reply[0] != AUTH_VERSION && reply[0] != VERSION {
        return Err(Socks5Error::BadAuthVersion(reply[0]));
    }
    if reply[1] != 0 {
        return Err(Socks5Error::AuthRejected(reply[1]));
    }
    Ok(())
}

fn encode_connect(target: &TargetAddr) -> Result<Vec<u8>, Socks5Error> {
    let mut req = Vec::with_capacity(7 + crate::connect::MAX_DOMAIN_LEN);
    req.extend_from_slice(&[VERSION, CMD_CONNECT, 0x00]);
    match target {
        TargetAddr::Ip(SocketAddr::V4(a)) => {
            req.push(ATYP_IPV4);
            req.extend_from_slice(&a.ip().octets());
        }
        TargetAddr::Ip(SocketAddr::V6(a)) => {
            req.push(ATYP_IPV6);
            req.extend_from_slice(&a.ip().octets());
        }
        TargetAddr::Domain(host, _) => {
            validate_domain(host)?;
            req.push(ATYP_DOMAIN);
            // validate_domain guarantees 1..=255 bytes.
            req.push(host.len() as u8);
            req.extend_from_slice(host.as_bytes());
        }
    }
    req.extend_from_slice(&target.port().to_be_bytes());
    Ok(req)
}

async fn read_bound_addr<S>(stream: &mut S, atyp: u8) -> Result<BoundAddr, Socks5Error>
where
    S: AsyncRead + Unpin,
{
    match atyp {
        ATYP_IPV4 => {
            let mut b = [0u8; 6];
            stream.read_exact(&mut b).await?;
            let ip = Ipv4Addr::new(b[0], b[1], b[2], b[3]);
            let port = u16::from_be_bytes([b[4], b[5]]);
            Ok(BoundAddr::Ip(SocketAddr::new(IpAddr::V4(ip), port)))
        }
        ATYP_IPV6 => {
            let mut b = [0u8; 18];
            stream.read_exact(&mut b).await?;
            let mut octets = [0u8; 16];
            octets.copy_from_slice(&b[..16]);
            let port = u16::from_be_bytes([b[16], b[17]]);
            Ok(BoundAddr::Ip(SocketAddr::new(
                IpAddr::V6(Ipv6Addr::from(octets)),
                port,
            )))
        }
        ATYP_DOMAIN => {
            let len = stream.read_u8().await? as usize;
            let mut b = vec![0u8; len + 2];
            stream.read_exact(&mut b).await?;
            let port = u16::from_be_bytes([b[len], b[len + 1]]);
            let host = String::from_utf8_lossy(&b[..len]).into_owned();
            Ok(BoundAddr::Domain(host, port))
        }
        other => Err(Socks5Error::UnknownAddressType(other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_proxy_urls() {
        let p: Socks5Proxy = "socks5h://user:pa%40ss%3A1@proxy.example:1081"
            .parse()
            .unwrap();
        assert_eq!(p.addr(), &TargetAddr::Domain("proxy.example".into(), 1081));
        let auth = p.auth().unwrap();
        assert_eq!(auth.username(), "user");
        assert_eq!(auth.password().expose(), "pa@ss:1");

        let p: Socks5Proxy = "SOCKS5://10.0.0.1".parse().unwrap();
        assert_eq!(p.addr(), &TargetAddr::Ip("10.0.0.1:1080".parse().unwrap()));
        assert!(p.auth().is_none());

        let p: Socks5Proxy = "[::1]:9050".parse().unwrap();
        assert_eq!(p.addr(), &TargetAddr::Ip("[::1]:9050".parse().unwrap()));

        // The last '@' separates the user info, so a raw '@' in the password still works.
        let p: Socks5Proxy = "socks5h://u:p@ss@h:1/".parse().unwrap();
        assert_eq!(p.auth().unwrap().password().expose(), "p@ss");

        assert!(matches!(
            "http://h:1".parse::<Socks5Proxy>(),
            Err(ProxyUrlError::Scheme(_))
        ));
        assert_eq!(
            "socks5://h:1/path".parse::<Socks5Proxy>(),
            Err(ProxyUrlError::TrailingData)
        );
        assert_eq!(
            "socks5://user@h:1".parse::<Socks5Proxy>(),
            Err(ProxyUrlError::IncompleteCredentials)
        );
        assert_eq!(
            "socks5://:p@h:1".parse::<Socks5Proxy>(),
            Err(ProxyUrlError::IncompleteCredentials)
        );
        assert_eq!(
            "socks5://u:%zz@h:1".parse::<Socks5Proxy>(),
            Err(ProxyUrlError::PercentEncoding)
        );
        assert_eq!(
            "socks5://u:%ff@h:1".parse::<Socks5Proxy>(),
            Err(ProxyUrlError::PercentEncoding)
        );
        assert_eq!(
            "socks5://u:%4@h:1".parse::<Socks5Proxy>(),
            Err(ProxyUrlError::PercentEncoding)
        );
        let long = format!("socks5://{}:p@h:1", "u".repeat(256));
        assert_eq!(
            long.parse::<Socks5Proxy>(),
            Err(ProxyUrlError::Credentials(CredentialsError::Username(256)))
        );
    }

    #[test]
    fn display_and_debug_hide_password() {
        let p: Socks5Proxy = "socks5h://bob:topsecret@h:1".parse().unwrap();
        assert_eq!(p.to_string(), "socks5h://bob@h:1");
        let dbg = format!("{p:?}");
        assert!(!dbg.contains("topsecret"), "{dbg}");
        assert!(dbg.contains("bob"), "{dbg}");
    }

    #[test]
    fn credentials_limits() {
        assert!(Socks5Auth::new("u", "p").is_ok());
        assert!(Socks5Auth::new("u".repeat(255), "p".repeat(255)).is_ok());
        assert_eq!(Socks5Auth::new("", "p"), Err(CredentialsError::Username(0)));
        assert_eq!(Socks5Auth::new("u", ""), Err(CredentialsError::Password(0)));
        assert_eq!(
            Socks5Auth::new("u", "p".repeat(256)),
            Err(CredentialsError::Password(256))
        );
    }

    #[test]
    fn encode_requests() {
        let v4 = encode_connect(&"1.2.3.4:25565".parse().unwrap()).unwrap();
        assert_eq!(v4, [5, 1, 0, 1, 1, 2, 3, 4, 0x63, 0xdd]);
        let v6 = encode_connect(&"[::1]:80".parse().unwrap()).unwrap();
        assert_eq!(v6.len(), 4 + 16 + 2);
        assert_eq!(v6[3], ATYP_IPV6);
        assert_eq!(v6[19], 1);
        let dom = encode_connect(&TargetAddr::Domain("ab.c".into(), 1)).unwrap();
        assert_eq!(dom, [5, 1, 0, 3, 4, b'a', b'b', b'.', b'c', 0, 1]);
        // Hand-built invalid domains are rejected before anything is sent.
        assert!(matches!(
            encode_connect(&TargetAddr::Domain(String::new(), 1)),
            Err(Socks5Error::InvalidTarget(AddrError::EmptyHost))
        ));
        assert!(matches!(
            encode_connect(&TargetAddr::Domain("a".repeat(256), 1)),
            Err(Socks5Error::InvalidTarget(AddrError::DomainTooLong(256)))
        ));
        assert!(matches!(
            encode_connect(&TargetAddr::Domain("a\0b".into(), 1)),
            Err(Socks5Error::InvalidTarget(AddrError::InvalidDomainChar(
                '\0'
            )))
        ));
    }

    #[test]
    fn reply_codes_round_trip() {
        for b in 1..=255u8 {
            assert_eq!(ReplyCode::from_byte(b).as_byte(), b);
        }
        assert_eq!(ReplyCode::from_byte(9), ReplyCode::Other(9));
    }
}
