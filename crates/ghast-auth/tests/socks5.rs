//! SOCKS5 client tests against in-process mock proxies.
//!
//! Protocol-level tests run [`handshake`] over an in-memory duplex pipe with a scripted proxy;
//! connector tests use real loopback TCP sockets.

use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use ghast_auth::connect::{
    ConnectError, Connector, ProxyPolicy, Resolve, ResolveFuture, Route, TargetAddr, TimeoutStage,
    Timeouts,
};
use ghast_auth::socks5::{BoundAddr, ReplyCode, Socks5Auth, Socks5Error, Socks5Proxy, handshake};
use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream, duplex};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;

// ---------------------------------------------------------------------------------------------
// A configurable, well-behaved mock proxy.

/// What the client sent to the mock proxy.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct Seen {
    methods: Vec<u8>,
    credentials: Option<(String, String)>,
    atyp: u8,
    addr: Vec<u8>,
    port: u16,
}

impl Seen {
    fn domain(&self) -> Option<String> {
        (self.atyp == 3).then(|| String::from_utf8(self.addr.clone()).unwrap())
    }
}

#[derive(Clone)]
struct MockConfig {
    /// Credentials the proxy demands; `None` = no authentication.
    credentials: Option<(&'static str, &'static str)>,
    /// REP byte of the CONNECT reply.
    reply: u8,
    /// BND.ADDR/BND.PORT part of the reply (ATYP included).
    bound: Vec<u8>,
    /// Bytes sent right after a successful reply, in the same write.
    trailing: Vec<u8>,
}

impl Default for MockConfig {
    fn default() -> Self {
        Self {
            credentials: None,
            reply: 0,
            bound: vec![1, 0, 0, 0, 0, 0, 0],
            trailing: Vec::new(),
        }
    }
}

/// Runs the proxy side of one negotiation. On success it echoes everything afterwards.
async fn serve<S>(mut s: S, cfg: MockConfig) -> io::Result<Seen>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let mut seen = Seen::default();
    assert_eq!(s.read_u8().await?, 5, "greeting version");
    let n = s.read_u8().await? as usize;
    seen.methods = vec![0; n];
    s.read_exact(&mut seen.methods).await?;

    match cfg.credentials {
        None => {
            if !seen.methods.contains(&0) {
                s.write_all(&[5, 0xff]).await?;
                return Ok(seen);
            }
            s.write_all(&[5, 0]).await?;
        }
        Some((user, pass)) => {
            if !seen.methods.contains(&2) {
                s.write_all(&[5, 0xff]).await?;
                return Ok(seen);
            }
            s.write_all(&[5, 2]).await?;
            assert_eq!(s.read_u8().await?, 1, "auth version");
            let ulen = s.read_u8().await? as usize;
            let mut u = vec![0; ulen];
            s.read_exact(&mut u).await?;
            let plen = s.read_u8().await? as usize;
            let mut p = vec![0; plen];
            s.read_exact(&mut p).await?;
            let (u, p) = (String::from_utf8(u).unwrap(), String::from_utf8(p).unwrap());
            let ok = u == user && p == pass;
            seen.credentials = Some((u, p));
            s.write_all(&[1, if ok { 0 } else { 1 }]).await?;
            if !ok {
                return Ok(seen);
            }
        }
    }

    let mut head = [0u8; 4];
    s.read_exact(&mut head).await?;
    assert_eq!(&head[..3], &[5, 1, 0], "CONNECT header");
    seen.atyp = head[3];
    seen.addr = match head[3] {
        1 => vec![0; 4],
        4 => vec![0; 16],
        3 => vec![0; s.read_u8().await? as usize],
        other => panic!("bad atyp {other}"),
    };
    s.read_exact(&mut seen.addr).await?;
    seen.port = s.read_u16().await?;

    let mut reply = vec![5, cfg.reply, 0];
    reply.extend_from_slice(&cfg.bound);
    if cfg.reply == 0 {
        reply.extend_from_slice(&cfg.trailing);
    }
    s.write_all(&reply).await?;
    if cfg.reply == 0 {
        let mut buf = [0u8; 256];
        loop {
            let n = s.read(&mut buf).await?;
            if n == 0 {
                break;
            }
            s.write_all(&buf[..n]).await?;
        }
    }
    Ok(seen)
}

fn pipe(cfg: MockConfig) -> (DuplexStream, JoinHandle<io::Result<Seen>>) {
    let (client, server) = duplex(4096);
    (client, tokio::spawn(serve(server, cfg)))
}

/// A loopback TCP mock proxy accepting one connection.
async fn tcp_proxy(cfg: MockConfig) -> (SocketAddr, JoinHandle<io::Result<Seen>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        let (stream, _) = listener.accept().await?;
        serve(stream, cfg).await
    });
    (addr, task)
}

fn domain(host: &str, port: u16) -> TargetAddr {
    TargetAddr::new(host, port).unwrap()
}

fn auth(user: &str, pass: &str) -> Socks5Auth {
    Socks5Auth::new(user, pass).unwrap()
}

/// Counts every local resolution; resolves `fake.test` to a fixed address and fails otherwise.
#[derive(Default)]
struct CountingResolver {
    calls: AtomicUsize,
    fake: std::sync::Mutex<Option<SocketAddr>>,
}

impl Resolve for CountingResolver {
    fn resolve<'a>(&'a self, host: &'a str, _port: u16) -> ResolveFuture<'a> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let fake = *self.fake.lock().unwrap();
        Box::pin(async move {
            match (host, fake) {
                ("fake.test", Some(addr)) => Ok(vec![addr]),
                _ => Err(io::Error::other(format!("unexpected lookup of {host}"))),
            }
        })
    }
}

// ---------------------------------------------------------------------------------------------
// Successful negotiations.

#[tokio::test]
async fn domain_target_without_auth() {
    let (mut client, server) = pipe(MockConfig::default());
    let bound = handshake(&mut client, &domain("mc.example.net", 25565), None)
        .await
        .unwrap();
    assert_eq!(bound, BoundAddr::Ip("0.0.0.0:0".parse().unwrap()));
    drop(client);
    let seen = server.await.unwrap().unwrap();
    assert_eq!(seen.methods, [0]);
    assert_eq!(seen.domain().as_deref(), Some("mc.example.net"));
    assert_eq!(seen.port, 25565);
}

#[tokio::test]
async fn username_password_auth() {
    let cfg = MockConfig {
        credentials: Some(("bot", "p@ss:w0rd")),
        ..Default::default()
    };
    let (mut client, server) = pipe(cfg);
    handshake(
        &mut client,
        &domain("mc.example.net", 25565),
        Some(&auth("bot", "p@ss:w0rd")),
    )
    .await
    .unwrap();
    drop(client);
    let seen = server.await.unwrap().unwrap();
    assert_eq!(seen.methods, [0, 2]);
    assert_eq!(
        seen.credentials,
        Some(("bot".to_owned(), "p@ss:w0rd".to_owned()))
    );
}

#[tokio::test]
async fn credentials_offered_but_proxy_needs_none() {
    let (mut client, server) = pipe(MockConfig::default());
    handshake(&mut client, &domain("a.b", 1), Some(&auth("u", "p")))
        .await
        .unwrap();
    drop(client);
    let seen = server.await.unwrap().unwrap();
    assert_eq!(seen.methods, [0, 2]);
    assert_eq!(seen.credentials, None);
}

#[tokio::test]
async fn ipv4_and_ipv6_targets() {
    let (mut client, server) = pipe(MockConfig::default());
    handshake(&mut client, &"203.0.113.7:25570".parse().unwrap(), None)
        .await
        .unwrap();
    drop(client);
    let seen = server.await.unwrap().unwrap();
    assert_eq!(
        (seen.atyp, seen.addr.as_slice(), seen.port),
        (1, &[203, 0, 113, 7][..], 25570)
    );

    let (mut client, server) = pipe(MockConfig::default());
    handshake(&mut client, &"[2001:db8::42]:1".parse().unwrap(), None)
        .await
        .unwrap();
    drop(client);
    let seen = server.await.unwrap().unwrap();
    assert_eq!(seen.atyp, 4);
    let expected: std::net::Ipv6Addr = "2001:db8::42".parse().unwrap();
    assert_eq!(seen.addr, expected.octets());
}

#[tokio::test]
async fn bound_address_types_are_parsed() {
    let mut v6 = vec![4];
    v6.extend_from_slice(
        &"2001:db8::1"
            .parse::<std::net::Ipv6Addr>()
            .unwrap()
            .octets(),
    );
    v6.extend_from_slice(&443u16.to_be_bytes());
    let cases = vec![
        (
            vec![1, 10, 0, 0, 1, 0x1f, 0x90],
            BoundAddr::Ip("10.0.0.1:8080".parse().unwrap()),
        ),
        (v6, BoundAddr::Ip("[2001:db8::1]:443".parse().unwrap())),
        (
            vec![3, 5, b'h', b'e', b'l', b'l', b'o', 0, 80],
            BoundAddr::Domain("hello".into(), 80),
        ),
        (vec![3, 0, 0, 7], BoundAddr::Domain(String::new(), 7)),
        (
            vec![3, 2, 0xff, 0xfe, 0, 1],
            BoundAddr::Domain("\u{fffd}\u{fffd}".into(), 1),
        ),
    ];
    for (bound, expected) in cases {
        let (mut client, _server) = pipe(MockConfig {
            bound,
            ..Default::default()
        });
        let got = handshake(&mut client, &domain("a.b", 1), None)
            .await
            .unwrap();
        assert_eq!(got, expected);
    }
}

#[tokio::test]
async fn data_after_reply_stays_in_stream() {
    let (mut client, _server) = pipe(MockConfig {
        trailing: b"EARLY".to_vec(),
        ..Default::default()
    });
    handshake(&mut client, &domain("a.b", 1), None)
        .await
        .unwrap();
    let mut buf = [0u8; 5];
    client.read_exact(&mut buf).await.unwrap();
    assert_eq!(&buf, b"EARLY");
    client.write_all(b"ping").await.unwrap();
    let mut buf = [0u8; 4];
    client.read_exact(&mut buf).await.unwrap();
    assert_eq!(&buf, b"ping");
}

// ---------------------------------------------------------------------------------------------
// Refusals and errors.

#[tokio::test]
async fn wrong_credentials_are_rejected() {
    let (mut client, _server) = pipe(MockConfig {
        credentials: Some(("bot", "right")),
        ..Default::default()
    });
    let err = handshake(&mut client, &domain("a.b", 1), Some(&auth("bot", "wrong")))
        .await
        .unwrap_err();
    assert!(matches!(err, Socks5Error::AuthRejected(1)), "{err:?}");
}

#[tokio::test]
async fn proxy_requires_auth_but_none_configured() {
    let (mut client, server) = pipe(MockConfig {
        credentials: Some(("bot", "pw")),
        ..Default::default()
    });
    let err = handshake(&mut client, &domain("a.b", 1), None)
        .await
        .unwrap_err();
    assert!(matches!(err, Socks5Error::NoAcceptableMethod), "{err:?}");
    assert_eq!(server.await.unwrap().unwrap().methods, [0]);
}

#[tokio::test]
async fn every_reply_code_is_mapped() {
    let expected = [
        (1, ReplyCode::GeneralFailure),
        (2, ReplyCode::NotAllowed),
        (3, ReplyCode::NetworkUnreachable),
        (4, ReplyCode::HostUnreachable),
        (5, ReplyCode::ConnectionRefused),
        (6, ReplyCode::TtlExpired),
        (7, ReplyCode::CommandNotSupported),
        (8, ReplyCode::AddressTypeNotSupported),
        (9, ReplyCode::Other(9)),
        (0xff, ReplyCode::Other(0xff)),
    ];
    for (byte, code) in expected {
        let (mut client, _server) = pipe(MockConfig {
            reply: byte,
            ..Default::default()
        });
        let err = handshake(&mut client, &domain("a.b", 1), None)
            .await
            .unwrap_err();
        match err {
            Socks5Error::Reply(got) => assert_eq!(got, code, "byte {byte:#x}"),
            other => panic!("byte {byte:#x}: {other:?}"),
        }
        // The message is human readable.
        assert!(!Socks5Error::Reply(code).to_string().is_empty());
    }
}

/// A proxy that reads `read` bytes, writes `reply`, then closes (or idles if `hang`).
fn scripted(read: usize, reply: Vec<u8>, hang: bool) -> DuplexStream {
    let (client, mut server) = duplex(4096);
    tokio::spawn(async move {
        let mut buf = vec![0u8; read];
        if server.read_exact(&mut buf).await.is_err() {
            return;
        }
        let _ = server.write_all(&reply).await;
        if hang {
            tokio::time::sleep(Duration::from_secs(3600)).await;
        }
    });
    client
}

async fn scripted_err(read: usize, reply: &[u8]) -> Socks5Error {
    let mut client = scripted(read, reply.to_vec(), false);
    handshake(&mut client, &domain("ab.c", 1), None)
        .await
        .unwrap_err()
}

/// Bytes the client sends without auth for target `ab.c:1`: greeting 3 + request 11.
const GREETING: usize = 3;
const REQUEST: usize = 11;

#[tokio::test]
async fn malformed_method_replies() {
    assert!(matches!(
        scripted_err(GREETING, &[4, 0]).await,
        Socks5Error::BadVersion(4)
    ));
    assert!(matches!(
        scripted_err(GREETING, &[5, 0xff]).await,
        Socks5Error::NoAcceptableMethod
    ));
    // GSSAPI, never offered.
    assert!(matches!(
        scripted_err(GREETING, &[5, 1]).await,
        Socks5Error::UnexpectedMethod(1)
    ));
    // Username/password selected although no credentials were offered.
    assert!(matches!(
        scripted_err(GREETING, &[5, 2]).await,
        Socks5Error::UnexpectedMethod(2)
    ));
    // An HTTP proxy answering a SOCKS greeting.
    assert!(matches!(
        scripted_err(GREETING, b"HTTP/1.1 400").await,
        Socks5Error::BadVersion(b'H')
    ));
}

#[tokio::test]
async fn malformed_auth_replies() {
    // Greeting with credentials is 4 bytes; auth message for u/p is 1+1+1+1+1 = 5 bytes.
    for (reply, check) in [
        (vec![5, 2, 9, 0], "bad version"),
        (vec![5, 2, 1, 0x7f], "rejected"),
        (vec![5, 2, 1], "truncated"),
    ] {
        // Two steps: the client waits for the method reply before sending the auth message.
        let (mut client, mut server) = duplex(4096);
        let r = reply.clone();
        tokio::spawn(async move {
            let mut buf = [0u8; 4];
            server.read_exact(&mut buf).await.unwrap();
            server.write_all(&r[..2]).await.unwrap();
            let mut buf = [0u8; 5];
            server.read_exact(&mut buf).await.unwrap();
            assert_eq!(buf, [1, 1, b'u', 1, b'p']);
            server.write_all(&r[2..]).await.unwrap();
        });
        let err = handshake(&mut client, &domain("ab.c", 1), Some(&auth("u", "p")))
            .await
            .unwrap_err();
        match check {
            "bad version" => assert!(matches!(err, Socks5Error::BadAuthVersion(9)), "{err:?}"),
            "rejected" => assert!(matches!(err, Socks5Error::AuthRejected(0x7f)), "{err:?}"),
            _ => assert!(matches!(err, Socks5Error::UnexpectedEof), "{err:?}"),
        }
    }
}

#[tokio::test]
async fn auth_reply_with_socks_version_is_tolerated() {
    let (mut client, mut server) = duplex(4096);
    tokio::spawn(async move {
        let mut buf = [0u8; 4];
        server.read_exact(&mut buf).await.unwrap();
        server.write_all(&[5, 2]).await.unwrap();
        let mut buf = [0u8; 5];
        server.read_exact(&mut buf).await.unwrap();
        server.write_all(&[5, 0]).await.unwrap();
        let mut buf = [0u8; REQUEST];
        server.read_exact(&mut buf).await.unwrap();
        server
            .write_all(&[5, 0, 0, 1, 0, 0, 0, 0, 0, 0])
            .await
            .unwrap();
    });
    handshake(&mut client, &domain("ab.c", 1), Some(&auth("u", "p")))
        .await
        .unwrap();
}

/// A proxy that answers the greeting correctly and then sends `reply` to the CONNECT.
async fn connect_reply_err(reply: &[u8]) -> Socks5Error {
    let (mut client, mut server) = duplex(4096);
    let reply = reply.to_vec();
    tokio::spawn(async move {
        let mut buf = [0u8; GREETING];
        server.read_exact(&mut buf).await.unwrap();
        server.write_all(&[5, 0]).await.unwrap();
        let mut buf = [0u8; REQUEST];
        server.read_exact(&mut buf).await.unwrap();
        assert_eq!(buf, [5, 1, 0, 3, 4, b'a', b'b', b'.', b'c', 0, 1]);
        server.write_all(&reply).await.unwrap();
    });
    handshake(&mut client, &domain("ab.c", 1), None)
        .await
        .unwrap_err()
}

#[tokio::test]
async fn malformed_connect_replies() {
    assert!(matches!(
        connect_reply_err(&[4, 0, 0, 1, 0, 0, 0, 0, 0, 0]).await,
        Socks5Error::BadVersion(4)
    ));
    assert!(matches!(
        connect_reply_err(&[5, 0, 1, 1, 0, 0, 0, 0, 0, 0]).await,
        Socks5Error::BadReserved(1)
    ));
    assert!(matches!(
        connect_reply_err(&[5, 0, 0, 2, 0, 0, 0, 0, 0, 0]).await,
        Socks5Error::UnknownAddressType(2)
    ));
    assert!(matches!(
        connect_reply_err(&[5, 0, 0, 0xff]).await,
        Socks5Error::UnknownAddressType(0xff)
    ));
}

#[tokio::test]
async fn truncated_replies_are_errors() {
    // Every strict prefix of a valid method reply.
    for cut in 0..2 {
        let err = scripted_err(GREETING, &[5, 0][..cut]).await;
        assert!(
            matches!(err, Socks5Error::UnexpectedEof),
            "cut {cut}: {err:?}"
        );
    }
    // Every strict prefix of valid CONNECT replies with each address type.
    let mut v6 = vec![5, 0, 0, 4];
    v6.extend_from_slice(&[0; 18]);
    let replies = [
        vec![5, 0, 0, 1, 1, 2, 3, 4, 0, 80],
        v6,
        vec![5, 0, 0, 3, 3, b'a', b'b', b'c', 0, 80],
    ];
    for full in replies {
        for cut in 0..full.len() {
            let err = connect_reply_err(&full[..cut]).await;
            assert!(
                matches!(err, Socks5Error::UnexpectedEof),
                "reply {full:?} cut {cut}: {err:?}"
            );
        }
    }
}

/// Deterministic pseudo-random proxy replies must never panic or hang the client.
#[tokio::test]
async fn random_replies_never_panic() {
    let mut state: u64 = 0x9e37_79b9_7f4a_7c15;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    for _ in 0..2000 {
        let len = (next() % 40) as usize;
        let mut reply: Vec<u8> = (0..len).map(|_| next() as u8).collect();
        // Bias towards getting past the first checks.
        if next() % 2 == 0 && reply.len() >= 2 {
            reply[0] = 5;
            reply[1] = [0, 2, 0xff][(next() % 3) as usize];
        }
        let (mut client, mut server) = duplex(4096);
        tokio::spawn(async move {
            let _ = server.write_all(&reply).await;
            // Signal EOF to the client (a proxy that stays silent is the timeout's job), then
            // drain whatever the client still sends.
            let _ = server.shutdown().await;
            let mut sink = [0u8; 512];
            while let Ok(n) = server.read(&mut sink).await {
                if n == 0 {
                    break;
                }
            }
        });
        let creds = auth("u", "p");
        let result = tokio::time::timeout(Duration::from_secs(5), async {
            let r = handshake(&mut client, &domain("ab.c", 1), Some(&creds)).await;
            drop(client);
            r
        })
        .await
        .expect("handshake must not hang when the proxy closes");
        let _ = result;
    }
}

// ---------------------------------------------------------------------------------------------
// Connector: TCP, timeouts, DNS and policy.

#[tokio::test]
async fn connector_through_proxy_end_to_end() {
    let cfg = MockConfig {
        credentials: Some(("bot", "secret")),
        ..Default::default()
    };
    let (proxy_addr, server) = tcp_proxy(cfg).await;
    let proxy = Socks5Proxy::new(proxy_addr.into()).with_auth(auth("bot", "secret"));
    let connector = Connector::new()
        .with_proxy(proxy)
        .with_policy(ProxyPolicy::RequireProxy);
    let mut stream = connector
        .connect(&domain("play.example.net", 25565))
        .await
        .unwrap();
    assert!(stream.nodelay().unwrap());
    stream.write_all(b"\x10handshake").await.unwrap();
    let mut buf = [0u8; 10];
    stream.read_exact(&mut buf).await.unwrap();
    assert_eq!(&buf, b"\x10handshake");
    drop(stream);
    let seen = server.await.unwrap().unwrap();
    assert_eq!(seen.domain().as_deref(), Some("play.example.net"));
    assert_eq!(seen.port, 25565);
}

#[tokio::test]
async fn proxied_domain_target_is_never_resolved_locally() {
    let (proxy_addr, server) = tcp_proxy(MockConfig::default()).await;
    let resolver = Arc::new(CountingResolver::default());
    let connector = Connector::new()
        .with_proxy(Socks5Proxy::new(proxy_addr.into()))
        .with_policy(ProxyPolicy::RequireProxy)
        .with_resolver(resolver.clone());
    // `.invalid` can never resolve (RFC 2606); the connection only works if the name goes to
    // the proxy untouched.
    let target = domain("leak-check.invalid", 25565);
    let stream = connector.connect(&target).await.unwrap();
    drop(stream);
    assert_eq!(
        resolver.calls.load(Ordering::SeqCst),
        0,
        "local DNS lookup happened"
    );
    let seen = server.await.unwrap().unwrap();
    assert_eq!(seen.atyp, 3);
    assert_eq!(seen.domain().as_deref(), Some("leak-check.invalid"));
}

#[tokio::test]
async fn resolver_is_used_for_direct_and_proxy_host_names() {
    // Direct connection to a domain: resolved through the configured resolver.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let resolver = Arc::new(CountingResolver::default());
    *resolver.fake.lock().unwrap() = Some(addr);
    let connector = Connector::new().with_resolver(resolver.clone());
    let accept = tokio::spawn(async move { listener.accept().await.map(|_| ()) });
    connector
        .connect(&domain("fake.test", addr.port()))
        .await
        .unwrap();
    accept.await.unwrap().unwrap();
    assert_eq!(resolver.calls.load(Ordering::SeqCst), 1);

    // The proxy's own host name is resolved locally; the target still is not.
    let (proxy_addr, server) = tcp_proxy(MockConfig::default()).await;
    *resolver.fake.lock().unwrap() = Some(proxy_addr);
    let connector = Connector::new()
        .with_proxy(Socks5Proxy::new(domain("fake.test", proxy_addr.port())))
        .with_resolver(resolver.clone());
    connector
        .connect(&domain("target.invalid", 1))
        .await
        .unwrap();
    assert_eq!(resolver.calls.load(Ordering::SeqCst), 2);
    assert_eq!(
        server.await.unwrap().unwrap().domain().as_deref(),
        Some("target.invalid")
    );
}

#[tokio::test]
async fn policy_refuses_direct_connection_without_side_effects() {
    let resolver = Arc::new(CountingResolver::default());
    let connector = Connector::new()
        .with_policy(ProxyPolicy::RequireProxyExceptLocal)
        .with_resolver(resolver.clone());
    let err = connector
        .connect(&domain("mc.example.net", 25565))
        .await
        .unwrap_err();
    assert!(matches!(err, ConnectError::ProxyRequired(_)), "{err:?}");
    assert_eq!(resolver.calls.load(Ordering::SeqCst), 0);

    let err = Connector::new()
        .with_policy(ProxyPolicy::RequireProxy)
        .connect(&"127.0.0.1:1".parse().unwrap())
        .await
        .unwrap_err();
    assert!(matches!(err, ConnectError::ProxyRequired(_)), "{err:?}");
}

#[tokio::test]
async fn local_targets_bypass_the_proxy_under_require_proxy_except_local() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    // A proxy address nobody listens on: using it would fail.
    let dead_proxy = Socks5Proxy::new("127.0.0.1:9".parse().unwrap());
    let connector = Connector::new()
        .with_proxy(dead_proxy)
        .with_policy(ProxyPolicy::RequireProxyExceptLocal);
    let accept = tokio::spawn(async move { listener.accept().await.map(|_| ()) });
    let stream = connector.connect(&addr.into()).await.unwrap();
    assert!(stream.nodelay().unwrap());
    accept.await.unwrap().unwrap();
}

#[tokio::test]
async fn custom_policy() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let port = addr.port();
    let connector = Connector::new().with_policy(ProxyPolicy::custom(move |t| {
        if t.port() == port {
            Route::Direct
        } else {
            Route::Proxy
        }
    }));
    let accept = tokio::spawn(async move { listener.accept().await.map(|_| ()) });
    connector.connect(&addr.into()).await.unwrap();
    accept.await.unwrap().unwrap();
    assert!(matches!(
        connector.connect(&"127.0.0.1:1".parse().unwrap()).await,
        Err(ConnectError::ProxyRequired(_))
    ));
}

#[tokio::test]
async fn handshake_timeout() {
    // A proxy that accepts and never answers.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (_stream, _) = listener.accept().await.unwrap();
        tokio::time::sleep(Duration::from_secs(3600)).await;
    });
    let connector = Connector::new()
        .with_proxy(Socks5Proxy::new(addr.into()))
        .with_timeouts(Timeouts {
            handshake: Duration::from_millis(200),
            ..Timeouts::default()
        });
    let started = std::time::Instant::now();
    let err = connector.connect(&domain("a.b", 1)).await.unwrap_err();
    assert!(
        matches!(err, ConnectError::Timeout(TimeoutStage::Handshake)),
        "{err:?}"
    );
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[tokio::test]
async fn proxy_errors_name_the_proxy_but_not_the_password() {
    let (proxy_addr, _server) = tcp_proxy(MockConfig {
        credentials: Some(("bot", "right")),
        ..Default::default()
    })
    .await;
    let connector = Connector::new()
        .with_proxy(Socks5Proxy::new(proxy_addr.into()).with_auth(auth("bot", "hunter2")));
    let err = connector.connect(&domain("a.b", 1)).await.unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains(&proxy_addr.to_string()), "{msg}");
    assert!(!format!("{msg} {err:?} {connector:?}").contains("hunter2"));
    assert!(matches!(
        err,
        ConnectError::Socks5 {
            source: Socks5Error::AuthRejected(1),
            ..
        }
    ));
}

#[tokio::test]
async fn unreachable_proxy_is_a_connect_error() {
    // Bind then drop to get a port that refuses connections.
    let addr = TcpListener::bind("127.0.0.1:0")
        .await
        .unwrap()
        .local_addr()
        .unwrap();
    let connector = Connector::new().with_proxy(Socks5Proxy::new(addr.into()));
    let err = connector.connect(&domain("a.b", 1)).await.unwrap_err();
    assert!(
        matches!(err, ConnectError::Connect { addr: a, .. } if a == addr),
        "{err:?}"
    );
}

#[tokio::test]
async fn proxy_refusal_is_reported() {
    let (proxy_addr, _server) = tcp_proxy(MockConfig {
        reply: 4,
        ..Default::default()
    })
    .await;
    let connector = Connector::new().with_proxy(Socks5Proxy::new(proxy_addr.into()));
    let err = connector.connect(&domain("a.b", 1)).await.unwrap_err();
    assert!(
        matches!(
            err,
            ConnectError::Socks5 {
                source: Socks5Error::Reply(ReplyCode::HostUnreachable),
                ..
            }
        ),
        "{err:?}"
    );
}

#[tokio::test]
async fn proxy_url_round_trip_with_connector() {
    let (proxy_addr, server) = tcp_proxy(MockConfig {
        credentials: Some(("user@corp", "p:w/d")),
        ..Default::default()
    })
    .await;
    let url = format!("socks5h://user%40corp:p%3Aw%2Fd@{proxy_addr}");
    let proxy: Socks5Proxy = url.parse().unwrap();
    let stream: TcpStream = Connector::new()
        .with_proxy(proxy)
        .connect(&domain("a.b", 1))
        .await
        .unwrap();
    drop(stream);
    assert_eq!(
        server.await.unwrap().unwrap().credentials,
        Some(("user@corp".into(), "p:w/d".into()))
    );
}
