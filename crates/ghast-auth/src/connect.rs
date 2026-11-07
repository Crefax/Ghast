//! Opening the game connection: target addresses, proxy routing policy and the [`Connector`].
//!
//! A [`Connector`] turns a [`TargetAddr`] into a connected [`TcpStream`] (with `TCP_NODELAY` set),
//! either directly or through a [`Socks5Proxy`]. Which of the two is allowed for a given target is
//! decided by a [`ProxyPolicy`], so an application can enforce rules such as "never connect to a
//! public server without a proxy" in one place.
//!
//! # DNS
//!
//! When a target is reached through the proxy, its domain name is sent to the proxy and resolved
//! there (SOCKS5 address type `DOMAINNAME`). The connector never resolves a proxied target
//! locally, so no DNS query for it leaves the machine. Only two things are ever resolved locally,
//! through the configured [`Resolve`] implementation: the proxy's own host name (if it is not an
//! IP literal) and targets of *direct* connections.
//!
//! Minecraft `_minecraft._tcp` SRV records are not looked up here: SRV lookups cannot be done
//! through SOCKS5, and doing them locally would leak the target name.

use std::fmt;
use std::future::Future;
use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use thiserror::Error;
use tokio::net::TcpStream;

use crate::socks5::{self, Socks5Error, Socks5Proxy};

/// Maximum length of a domain name in bytes (the SOCKS5 length prefix is one byte).
pub const MAX_DOMAIN_LEN: usize = 255;

/// The default Minecraft server port.
pub const DEFAULT_MINECRAFT_PORT: u16 = 25565;

/// Where to connect: an IP socket address or a domain name plus port.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TargetAddr {
    /// An IP address and port.
    Ip(SocketAddr),
    /// A domain name and port. Through a proxy the name is resolved by the proxy.
    ///
    /// Values built with [`TargetAddr::new`] or [`str::parse`] are validated (non-empty, at most
    /// [`MAX_DOMAIN_LEN`] bytes, only `A-Z a-z 0-9 - . _`). A hand-built variant is validated
    /// again before it is sent to a proxy.
    Domain(String, u16),
}

/// Why a target address could not be parsed.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum AddrError {
    /// The host part is empty.
    #[error("host is empty")]
    EmptyHost,
    /// No port was given and no default applies.
    #[error("port is missing")]
    MissingPort,
    /// The port is not a number in `0..=65535`.
    #[error("invalid port {0:?}")]
    InvalidPort(String),
    /// A bracketed IPv6 literal is malformed.
    #[error("invalid IPv6 address {0:?}")]
    InvalidIpv6(String),
    /// The domain name is longer than [`MAX_DOMAIN_LEN`] bytes.
    #[error("domain name is {0} bytes long, the maximum is {MAX_DOMAIN_LEN}")]
    DomainTooLong(usize),
    /// The domain name contains a character outside `A-Z a-z 0-9 - . _`.
    ///
    /// Internationalized names must be given in their ASCII (punycode) form.
    #[error("domain name contains the invalid character {0:?}")]
    InvalidDomainChar(char),
}

impl TargetAddr {
    /// Builds a target from a host (IP literal or domain name) and a port.
    ///
    /// IP literals (including IPv6 without brackets) become [`TargetAddr::Ip`]; everything else is
    /// validated as a domain name.
    pub fn new(host: &str, port: u16) -> Result<Self, AddrError> {
        let unbracketed = host
            .strip_prefix('[')
            .and_then(|h| h.strip_suffix(']'))
            .unwrap_or(host);
        if let Ok(ip) = unbracketed.parse::<IpAddr>() {
            return Ok(Self::Ip(SocketAddr::new(ip, port)));
        }
        if unbracketed.len() != host.len() {
            return Err(AddrError::InvalidIpv6(host.to_owned()));
        }
        validate_domain(host)?;
        Ok(Self::Domain(host.to_owned(), port))
    }

    /// Parses `host`, `host:port`, `[v6]`, `[v6]:port` or a bare IPv6 literal, using
    /// `default_port` when the string has no port.
    ///
    /// ```
    /// use ghast_auth::connect::{DEFAULT_MINECRAFT_PORT, TargetAddr};
    ///
    /// let t = TargetAddr::parse_with_default_port("mc.example.net", DEFAULT_MINECRAFT_PORT).unwrap();
    /// assert_eq!(t, TargetAddr::Domain("mc.example.net".into(), 25565));
    /// ```
    pub fn parse_with_default_port(s: &str, default_port: u16) -> Result<Self, AddrError> {
        parse_target(s, Some(default_port))
    }

    /// The port.
    pub fn port(&self) -> u16 {
        match self {
            Self::Ip(addr) => addr.port(),
            Self::Domain(_, port) => *port,
        }
    }

    /// The host part as a string (IPv6 without brackets).
    pub fn host(&self) -> String {
        match self {
            Self::Ip(addr) => addr.ip().to_string(),
            Self::Domain(host, _) => host.clone(),
        }
    }

    /// Returns `true` if the target is on this machine or on the local network: loopback,
    /// private IPv4 ranges (RFC 1918), link-local, IPv6 unique-local, or the `localhost` name
    /// (RFC 6761).
    ///
    /// Other domain names are **not** considered local, because deciding that would require a
    /// local DNS lookup. CGNAT (`100.64.0.0/10`) is not considered local either.
    pub fn is_local(&self) -> bool {
        match self {
            Self::Ip(addr) => is_local_ip(addr.ip()),
            Self::Domain(host, _) => {
                let host = host.strip_suffix('.').unwrap_or(host).to_ascii_lowercase();
                host == "localhost" || host.ends_with(".localhost")
            }
        }
    }
}

impl From<SocketAddr> for TargetAddr {
    fn from(addr: SocketAddr) -> Self {
        Self::Ip(addr)
    }
}

impl FromStr for TargetAddr {
    type Err = AddrError;

    /// Parses `host:port` or `[v6]:port`. The port is required; see
    /// [`TargetAddr::parse_with_default_port`] for a default.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        parse_target(s, None)
    }
}

impl fmt::Display for TargetAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ip(addr) => write!(f, "{addr}"),
            Self::Domain(host, port) => write!(f, "{host}:{port}"),
        }
    }
}

fn parse_target(s: &str, default_port: Option<u16>) -> Result<TargetAddr, AddrError> {
    let s = s.trim();
    let parse_port = |p: &str| {
        p.parse::<u16>()
            .map_err(|_| AddrError::InvalidPort(p.to_owned()))
    };
    let need_default = || default_port.ok_or(AddrError::MissingPort);

    if let Some(rest) = s.strip_prefix('[') {
        let (inside, after) = rest
            .split_once(']')
            .ok_or_else(|| AddrError::InvalidIpv6(s.to_owned()))?;
        let ip: Ipv6Addr = inside
            .parse()
            .map_err(|_| AddrError::InvalidIpv6(inside.to_owned()))?;
        let port = match after {
            "" => need_default()?,
            _ => match after.strip_prefix(':') {
                Some(p) => parse_port(p)?,
                None => return Err(AddrError::InvalidIpv6(s.to_owned())),
            },
        };
        return Ok(TargetAddr::Ip(SocketAddr::new(IpAddr::V6(ip), port)));
    }
    // A bare IPv6 literal has several colons and no port.
    if let Ok(ip) = s.parse::<Ipv6Addr>() {
        return Ok(TargetAddr::Ip(SocketAddr::new(
            IpAddr::V6(ip),
            need_default()?,
        )));
    }
    let (host, port) = match s.rsplit_once(':') {
        Some((host, port)) => (host, parse_port(port)?),
        None => (s, need_default()?),
    };
    if host.is_empty() {
        return Err(AddrError::EmptyHost);
    }
    if let Ok(ip) = host.parse::<Ipv4Addr>() {
        return Ok(TargetAddr::Ip(SocketAddr::new(IpAddr::V4(ip), port)));
    }
    validate_domain(host)?;
    Ok(TargetAddr::Domain(host.to_owned(), port))
}

pub(crate) fn validate_domain(host: &str) -> Result<(), AddrError> {
    if host.is_empty() {
        return Err(AddrError::EmptyHost);
    }
    if host.len() > MAX_DOMAIN_LEN {
        return Err(AddrError::DomainTooLong(host.len()));
    }
    match host
        .chars()
        .find(|c| !(c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '_')))
    {
        Some(c) => Err(AddrError::InvalidDomainChar(c)),
        None => Ok(()),
    }
}

fn is_local_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_local_v4(v4),
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_local_v4(v4);
            }
            v6.is_loopback() || v6.is_unique_local() || v6.is_unicast_link_local()
        }
    }
}

fn is_local_v4(v4: Ipv4Addr) -> bool {
    v4.is_loopback() || v4.is_private() || v4.is_link_local()
}

/// How a single connection should be routed. Returned by a [`ProxyPolicy`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    /// Connect through the configured proxy; fail with [`ConnectError::ProxyRequired`] if none
    /// is configured.
    Proxy,
    /// Connect through the proxy if one is configured, otherwise directly.
    PreferProxy,
    /// Connect directly, even if a proxy is configured.
    Direct,
}

/// A user-supplied routing rule, see [`ProxyPolicy::Custom`].
pub type RouteFn = dyn Fn(&TargetAddr) -> Route + Send + Sync;

/// Decides, per target, whether a connection must, may or must not use the proxy.
#[derive(Clone, Default)]
pub enum ProxyPolicy {
    /// Use the proxy when one is configured, otherwise connect directly. ([`Route::PreferProxy`]
    /// for every target.)
    #[default]
    PreferProxy,
    /// Every connection must go through the proxy. ([`Route::Proxy`] for every target.)
    RequireProxy,
    /// Targets for which [`TargetAddr::is_local`] holds are connected directly (a remote proxy
    /// usually cannot reach this machine's LAN); every other target must use the proxy.
    RequireProxyExceptLocal,
    /// A custom rule.
    Custom(Arc<RouteFn>),
}

impl ProxyPolicy {
    /// Builds a [`ProxyPolicy::Custom`] from a closure.
    pub fn custom(f: impl Fn(&TargetAddr) -> Route + Send + Sync + 'static) -> Self {
        Self::Custom(Arc::new(f))
    }

    /// The route this policy selects for `target`.
    pub fn route(&self, target: &TargetAddr) -> Route {
        match self {
            Self::PreferProxy => Route::PreferProxy,
            Self::RequireProxy => Route::Proxy,
            Self::RequireProxyExceptLocal if target.is_local() => Route::Direct,
            Self::RequireProxyExceptLocal => Route::Proxy,
            Self::Custom(f) => f(target),
        }
    }
}

impl fmt::Debug for ProxyPolicy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PreferProxy => f.write_str("PreferProxy"),
            Self::RequireProxy => f.write_str("RequireProxy"),
            Self::RequireProxyExceptLocal => f.write_str("RequireProxyExceptLocal"),
            Self::Custom(_) => f.write_str("Custom(..)"),
        }
    }
}

/// The future returned by [`Resolve::resolve`].
pub type ResolveFuture<'a> = Pin<Box<dyn Future<Output = io::Result<Vec<SocketAddr>>> + Send + 'a>>;

/// Local name resolution, used only for direct connections and for the proxy's own host name.
///
/// The default is [`SystemResolver`]. Applications can plug in their own (for example DNS over
/// HTTPS, a static table, or a counter in tests).
pub trait Resolve: Send + Sync {
    /// Resolves `host` to socket addresses with the given port.
    fn resolve<'a>(&'a self, host: &'a str, port: u16) -> ResolveFuture<'a>;
}

/// The operating system resolver (`getaddrinfo` through [`tokio::net::lookup_host`]).
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemResolver;

impl Resolve for SystemResolver {
    fn resolve<'a>(&'a self, host: &'a str, port: u16) -> ResolveFuture<'a> {
        Box::pin(async move { Ok(tokio::net::lookup_host((host, port)).await?.collect()) })
    }
}

/// Timeouts used by the [`Connector`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timeouts {
    /// Local DNS resolution (proxy host name or direct target).
    pub resolve: Duration,
    /// Each TCP connection attempt (to the proxy or the target).
    pub connect: Duration,
    /// The whole SOCKS5 negotiation, from the greeting to the CONNECT reply.
    pub handshake: Duration,
}

impl Default for Timeouts {
    fn default() -> Self {
        Self {
            resolve: Duration::from_secs(10),
            connect: Duration::from_secs(10),
            handshake: Duration::from_secs(15),
        }
    }
}

/// Which step of [`Connector::connect`] timed out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeoutStage {
    /// Local DNS resolution.
    Resolve,
    /// The TCP connection attempt.
    Connect,
    /// The SOCKS5 negotiation.
    Handshake,
}

impl fmt::Display for TimeoutStage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Resolve => "DNS resolution",
            Self::Connect => "TCP connect",
            Self::Handshake => "SOCKS5 handshake",
        })
    }
}

/// Errors from [`Connector::connect`].
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ConnectError {
    /// The policy requires a proxy for this target but none is configured. Nothing was sent.
    #[error("the proxy policy requires a proxy to connect to {0}, but none is configured")]
    ProxyRequired(TargetAddr),
    /// Local DNS resolution failed.
    #[error("could not resolve {host}: {source}")]
    Resolve {
        /// The name being resolved.
        host: String,
        /// The resolver error.
        #[source]
        source: io::Error,
    },
    /// Local DNS resolution returned no addresses.
    #[error("{0} resolved to no addresses")]
    NoAddresses(String),
    /// A step took longer than its [`Timeouts`] entry.
    #[error("{0} timed out")]
    Timeout(TimeoutStage),
    /// The TCP connection to the target (direct) or the proxy failed.
    #[error("could not connect to {addr}: {source}")]
    Connect {
        /// The last address that was tried.
        addr: SocketAddr,
        /// The socket error.
        #[source]
        source: io::Error,
    },
    /// The SOCKS5 negotiation failed (including the proxy refusing the CONNECT).
    #[error("SOCKS5 proxy {proxy}: {source}")]
    Socks5 {
        /// The proxy address (credentials are never included).
        proxy: TargetAddr,
        /// What went wrong.
        #[source]
        source: Socks5Error,
    },
    /// Another socket error (for example setting `TCP_NODELAY`).
    #[error(transparent)]
    Io(#[from] io::Error),
}

/// Opens game connections, directly or through a SOCKS5 proxy.
///
/// Cheap to clone; one connector can serve many bots.
#[derive(Clone)]
pub struct Connector {
    proxy: Option<Socks5Proxy>,
    policy: ProxyPolicy,
    timeouts: Timeouts,
    resolver: Arc<dyn Resolve>,
}

impl Default for Connector {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for Connector {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Connector")
            .field("proxy", &self.proxy)
            .field("policy", &self.policy)
            .field("timeouts", &self.timeouts)
            .finish_non_exhaustive()
    }
}

impl Connector {
    /// A connector without a proxy, with [`ProxyPolicy::PreferProxy`], default [`Timeouts`] and
    /// the [`SystemResolver`].
    pub fn new() -> Self {
        Self {
            proxy: None,
            policy: ProxyPolicy::default(),
            timeouts: Timeouts::default(),
            resolver: Arc::new(SystemResolver),
        }
    }

    /// Sets the SOCKS5 proxy.
    pub fn with_proxy(mut self, proxy: Socks5Proxy) -> Self {
        self.proxy = Some(proxy);
        self
    }

    /// Sets or clears the SOCKS5 proxy.
    pub fn with_optional_proxy(mut self, proxy: Option<Socks5Proxy>) -> Self {
        self.proxy = proxy;
        self
    }

    /// Sets the routing policy.
    pub fn with_policy(mut self, policy: ProxyPolicy) -> Self {
        self.policy = policy;
        self
    }

    /// Sets the timeouts.
    pub fn with_timeouts(mut self, timeouts: Timeouts) -> Self {
        self.timeouts = timeouts;
        self
    }

    /// Replaces the local resolver.
    pub fn with_resolver(mut self, resolver: Arc<dyn Resolve>) -> Self {
        self.resolver = resolver;
        self
    }

    /// The configured proxy, if any.
    pub fn proxy(&self) -> Option<&Socks5Proxy> {
        self.proxy.as_ref()
    }

    /// The routing policy.
    pub fn policy(&self) -> &ProxyPolicy {
        &self.policy
    }

    /// The route [`Connector::connect`] would take for `target`: `Some(true)` through the
    /// proxy, `Some(false)` directly, `None` if the policy forbids connecting at all.
    pub fn plan(&self, target: &TargetAddr) -> Option<bool> {
        match (self.policy.route(target), self.proxy.is_some()) {
            (Route::Proxy, true) | (Route::PreferProxy, true) => Some(true),
            (Route::Proxy, false) => None,
            (Route::PreferProxy, false) | (Route::Direct, _) => Some(false),
        }
    }

    /// Connects to `target` according to the policy and returns a stream with `TCP_NODELAY` set.
    ///
    /// Through a proxy the stream is positioned right after the SOCKS5 CONNECT reply, so the
    /// first bytes written are the game's handshake packet.
    pub async fn connect(&self, target: &TargetAddr) -> Result<TcpStream, ConnectError> {
        let stream = match (self.plan(target), &self.proxy) {
            (None, _) => return Err(ConnectError::ProxyRequired(target.clone())),
            (Some(true), Some(proxy)) => self.connect_via_proxy(proxy, target).await?,
            (Some(_), _) => self.connect_addr(target).await?,
        };
        stream.set_nodelay(true)?;
        Ok(stream)
    }

    async fn connect_via_proxy(
        &self,
        proxy: &Socks5Proxy,
        target: &TargetAddr,
    ) -> Result<TcpStream, ConnectError> {
        let mut stream = self.connect_addr(proxy.addr()).await?;
        // Disable Nagle before the negotiation so its small writes are not delayed.
        stream.set_nodelay(true)?;
        let negotiation = socks5::handshake(&mut stream, target, proxy.auth());
        match tokio::time::timeout(self.timeouts.handshake, negotiation).await {
            Err(_) => Err(ConnectError::Timeout(TimeoutStage::Handshake)),
            Ok(Err(source)) => Err(ConnectError::Socks5 {
                proxy: proxy.addr().clone(),
                source,
            }),
            Ok(Ok(_bound)) => Ok(stream),
        }
    }

    /// Opens a plain TCP connection, resolving `addr` locally if it is a domain name.
    async fn connect_addr(&self, addr: &TargetAddr) -> Result<TcpStream, ConnectError> {
        let candidates = match addr {
            TargetAddr::Ip(ip) => vec![*ip],
            TargetAddr::Domain(host, port) => {
                let lookup = self.resolver.resolve(host, *port);
                let addrs = match tokio::time::timeout(self.timeouts.resolve, lookup).await {
                    Err(_) => return Err(ConnectError::Timeout(TimeoutStage::Resolve)),
                    Ok(Err(source)) => {
                        return Err(ConnectError::Resolve {
                            host: host.clone(),
                            source,
                        });
                    }
                    Ok(Ok(addrs)) => addrs,
                };
                if addrs.is_empty() {
                    return Err(ConnectError::NoAddresses(host.clone()));
                }
                addrs
            }
        };
        let mut last_err = None;
        for candidate in candidates {
            match tokio::time::timeout(self.timeouts.connect, TcpStream::connect(candidate)).await {
                Ok(Ok(stream)) => return Ok(stream),
                Ok(Err(source)) => {
                    last_err = Some(ConnectError::Connect {
                        addr: candidate,
                        source,
                    })
                }
                Err(_) => last_err = Some(ConnectError::Timeout(TimeoutStage::Connect)),
            }
        }
        // `candidates` is never empty here, so an error was recorded.
        Err(last_err.unwrap_or(ConnectError::Timeout(TimeoutStage::Connect)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> TargetAddr {
        TargetAddr::Ip(s.parse().unwrap())
    }

    #[test]
    fn parse_targets() {
        assert_eq!("1.2.3.4:25565".parse(), Ok(ip("1.2.3.4:25565")));
        assert_eq!("[::1]:80".parse(), Ok(ip("[::1]:80")));
        assert_eq!(
            "mc.example.net:25570".parse(),
            Ok(TargetAddr::Domain("mc.example.net".into(), 25570))
        );
        assert_eq!(
            "mc.example.net".parse::<TargetAddr>(),
            Err(AddrError::MissingPort)
        );
        assert_eq!(
            TargetAddr::parse_with_default_port("mc.example.net", 25565),
            Ok(TargetAddr::Domain("mc.example.net".into(), 25565))
        );
        assert_eq!(
            TargetAddr::parse_with_default_port("::1", 25565),
            Ok(ip("[::1]:25565"))
        );
        assert_eq!(
            TargetAddr::parse_with_default_port("[fe80::1]", 1),
            Ok(ip("[fe80::1]:1"))
        );
        assert_eq!(
            "host:99999".parse::<TargetAddr>(),
            Err(AddrError::InvalidPort("99999".into()))
        );
        assert_eq!(":25565".parse::<TargetAddr>(), Err(AddrError::EmptyHost));
        assert!(matches!(
            "[::1".parse::<TargetAddr>(),
            Err(AddrError::InvalidIpv6(_))
        ));
        assert!(matches!(
            "[nope]:1".parse::<TargetAddr>(),
            Err(AddrError::InvalidIpv6(_))
        ));
        assert_eq!(
            "bad host:1".parse::<TargetAddr>(),
            Err(AddrError::InvalidDomainChar(' '))
        );
        assert_eq!(
            "b\u{fc}cher.de:1".parse::<TargetAddr>(),
            Err(AddrError::InvalidDomainChar('\u{fc}'))
        );
        let long = format!("{}:1", "a".repeat(256));
        assert_eq!(
            long.parse::<TargetAddr>(),
            Err(AddrError::DomainTooLong(256))
        );
    }

    #[test]
    fn new_and_display() {
        assert_eq!(TargetAddr::new("::1", 5), Ok(ip("[::1]:5")));
        assert_eq!(TargetAddr::new("[::1]", 5), Ok(ip("[::1]:5")));
        assert_eq!(TargetAddr::new("10.0.0.1", 5), Ok(ip("10.0.0.1:5")));
        assert!(TargetAddr::new("[x]", 5).is_err());
        assert_eq!(
            TargetAddr::new("a.b", 7).unwrap().to_string(),
            "a.b:7".to_owned()
        );
        assert_eq!(ip("[::1]:5").to_string(), "[::1]:5");
        assert_eq!(ip("[::1]:5").host(), "::1");
    }

    #[test]
    fn locality() {
        for local in [
            "127.0.0.1:1",
            "10.1.2.3:1",
            "172.16.0.1:1",
            "10.0.0.10:25570",
            "169.254.1.1:1",
            "[::1]:1",
            "[fd00::1]:1",
            "[fe80::1]:1",
            "[::ffff:10.0.0.1]:1",
        ] {
            assert!(ip(local).is_local(), "{local}");
        }
        for public in [
            "8.8.8.8:1",
            "100.64.0.1:1",
            "172.32.0.1:1",
            "[2001:db8::1]:1",
            "[::ffff:8.8.8.8]:1",
        ] {
            assert!(!ip(public).is_local(), "{public}");
        }
        assert!(TargetAddr::new("localhost", 1).unwrap().is_local());
        assert!(TargetAddr::new("LocalHost.", 1).unwrap().is_local());
        assert!(TargetAddr::new("a.localhost", 1).unwrap().is_local());
        assert!(!TargetAddr::new("lan-server", 1).unwrap().is_local());
        assert!(!TargetAddr::new("localhost.example", 1).unwrap().is_local());
    }

    #[test]
    fn policies() {
        let lan = ip("10.0.0.10:25570");
        let public = TargetAddr::new("mc.example.net", 25565).unwrap();
        assert_eq!(ProxyPolicy::PreferProxy.route(&public), Route::PreferProxy);
        assert_eq!(ProxyPolicy::RequireProxy.route(&lan), Route::Proxy);
        assert_eq!(
            ProxyPolicy::RequireProxyExceptLocal.route(&lan),
            Route::Direct
        );
        assert_eq!(
            ProxyPolicy::RequireProxyExceptLocal.route(&public),
            Route::Proxy
        );
        let custom = ProxyPolicy::custom(|t| {
            if t.port() == 25570 {
                Route::Direct
            } else {
                Route::Proxy
            }
        });
        assert_eq!(custom.route(&lan), Route::Direct);
        assert_eq!(custom.route(&public), Route::Proxy);
        assert_eq!(format!("{custom:?}"), "Custom(..)");
    }

    #[test]
    fn plan() {
        let lan = ip("10.0.0.10:25570");
        let public = ip("8.8.8.8:25565");
        let proxy: Socks5Proxy = "socks5h://127.0.0.1:1080".parse().unwrap();

        let none = Connector::new().with_policy(ProxyPolicy::RequireProxyExceptLocal);
        assert_eq!(none.plan(&lan), Some(false));
        assert_eq!(none.plan(&public), None);

        let some = none.clone().with_proxy(proxy.clone());
        assert_eq!(some.plan(&lan), Some(false));
        assert_eq!(some.plan(&public), Some(true));

        let prefer = Connector::new();
        assert_eq!(prefer.plan(&public), Some(false));
        assert_eq!(prefer.clone().with_proxy(proxy).plan(&public), Some(true));
    }
}
