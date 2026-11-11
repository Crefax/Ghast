//! Microsoft login chain against a local mock HTTP server (no real services are contacted).

#![cfg(feature = "microsoft")]

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, SystemTime};

use ghast_auth::SecretString;
use ghast_auth::microsoft::{
    CachedAccount, DevicePoll, Endpoints, JoinRejection, MicrosoftAuth, MicrosoftConfig,
    MicrosoftError, MinecraftProfile, MinecraftSession, Stage, TokenCache, XboxError,
};
use serde_json::json;
use uuid::Uuid;
use wiremock::matchers::{body_partial_json, body_string_contains, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const CLIENT_ID: &str = "test-client";
const PROFILE_ID: &str = "069a79f444e94726a5befca90e38aaf5";

fn auth_for(server: &MockServer, live: bool) -> MicrosoftAuth {
    let config = if live {
        MicrosoftConfig::live(CLIENT_ID)
    } else {
        MicrosoftConfig::azure_ad(CLIENT_ID)
    };
    MicrosoftAuth::new(config.with_endpoints(Endpoints::with_base_url(&server.uri()))).unwrap()
}

fn temp_cache(name: &str) -> TokenCache {
    static N: AtomicUsize = AtomicUsize::new(0);
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!(
        "ghast-auth-{}-{}-{name}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    TokenCache::new(dir.join("nested").join("account.json"))
}

fn json_response(status: u16, body: serde_json::Value) -> ResponseTemplate {
    ResponseTemplate::new(status).set_body_json(body)
}

/// Mounts Xbox, XSTS, Minecraft login and profile for the Microsoft access token `msa`.
async fn mount_chain(server: &MockServer, rps_ticket: &str) {
    Mock::given(method("POST"))
        .and(path("/user/authenticate"))
        .and(header("x-xbl-contract-version", "1"))
        .and(body_partial_json(json!({
            "Properties": {
                "AuthMethod": "RPS",
                "SiteName": "user.auth.xboxlive.com",
                "RpsTicket": rps_ticket,
            },
            "RelyingParty": "http://auth.xboxlive.com",
            "TokenType": "JWT",
        })))
        .respond_with(json_response(
            200,
            json!({
                "IssueInstant": "2026-11-03T00:00:00Z",
                "NotAfter": "2026-11-17T00:00:00Z",
                "Token": "xbl-token",
                "DisplayClaims": { "xui": [{ "uhs": "uhs123" }] },
            }),
        ))
        .expect(1)
        .mount(server)
        .await;
    Mock::given(method("POST"))
        .and(path("/xsts/authorize"))
        .and(body_partial_json(json!({
            "Properties": { "SandboxId": "RETAIL", "UserTokens": ["xbl-token"] },
            "RelyingParty": "rp://api.minecraftservices.com/",
        })))
        .respond_with(json_response(
            200,
            json!({
                "Token": "xsts-token",
                "DisplayClaims": { "xui": [{ "uhs": "uhs123" }] },
            }),
        ))
        .expect(1)
        .mount(server)
        .await;
    Mock::given(method("POST"))
        .and(path("/authentication/login_with_xbox"))
        .and(body_partial_json(
            json!({ "identityToken": "XBL3.0 x=uhs123;xsts-token" }),
        ))
        .respond_with(json_response(
            200,
            json!({
                "username": "some-uuid",
                "roles": [],
                "access_token": "mc-token",
                "token_type": "Bearer",
                "expires_in": 86400,
            }),
        ))
        .expect(1)
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/minecraft/profile"))
        .and(header("authorization", "Bearer mc-token"))
        .respond_with(json_response(
            200,
            json!({ "id": PROFILE_ID, "name": "Notch", "skins": [], "capes": [] }),
        ))
        .expect(1)
        .mount(server)
        .await;
}

async fn mount_device_code(server: &MockServer, url_path: &str) {
    Mock::given(method("POST"))
        .and(path(url_path))
        .and(body_string_contains("client_id=test-client"))
        .respond_with(json_response(
            200,
            json!({
                "user_code": "ABCD-1234",
                "device_code": "dev-code",
                "verification_uri": "https://www.microsoft.com/link",
                "expires_in": 900,
                "interval": 0,
                "message": "To sign in, use a web browser...",
            }),
        ))
        .expect(1)
        .mount(server)
        .await;
}

async fn mount_token_success(server: &MockServer, url_path: &str, grant: &str) {
    Mock::given(method("POST"))
        .and(path(url_path))
        .and(body_string_contains(grant))
        .respond_with(json_response(
            200,
            json!({
                "token_type": "bearer",
                "expires_in": 86400,
                "scope": "XboxLive.signin",
                "access_token": "msa-access",
                "refresh_token": "msa-refresh-2",
            }),
        ))
        .mount(server)
        .await;
}

fn profile_uuid() -> Uuid {
    Uuid::parse_str(PROFILE_ID).unwrap()
}

#[tokio::test]
async fn live_device_code_flow_end_to_end() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/live/oauth20_connect.srf"))
        .and(body_string_contains("response_type=device_code"))
        .and(body_string_contains(
            "scope=service%3A%3Auser.auth.xboxlive.com%3A%3AMBI_SSL",
        ))
        .respond_with(json_response(
            200,
            json!({
                "user_code": "ABCD-1234",
                "device_code": "dev-code",
                "verification_uri": "https://www.microsoft.com/link",
                "expires_in": 900,
                "interval": 0,
            }),
        ))
        .expect(1)
        .mount(&server)
        .await;
    // First poll: still pending. Second poll: success.
    Mock::given(method("POST"))
        .and(path("/live/oauth20_token.srf"))
        .and(body_string_contains("device_code=dev-code"))
        .respond_with(json_response(
            400,
            json!({ "error": "authorization_pending", "error_description": "wait" }),
        ))
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&server)
        .await;
    mount_token_success(
        &server,
        "/live/oauth20_token.srf",
        "grant_type=urn%3Aietf%3Aparams%3Aoauth%3Agrant-type%3Adevice_code",
    )
    .await;
    mount_chain(&server, "msa-access").await;

    let auth = auth_for(&server, true);
    let cache = temp_cache("live");
    let mut shown = None;
    let session = auth
        .authenticate(Some(&cache), |code| shown = Some(code.user_code.clone()))
        .await
        .unwrap();
    assert_eq!(shown.as_deref(), Some("ABCD-1234"));
    assert_eq!(session.profile.id, profile_uuid());
    assert_eq!(session.profile.name, "Notch");
    assert_eq!(session.access_token.expose(), "mc-token");
    assert!(session.is_valid_for(Duration::from_secs(86000)));

    let saved = cache.load().await.unwrap().unwrap();
    assert_eq!(saved.client_id, CLIENT_ID);
    assert_eq!(
        saved
            .microsoft_refresh_token
            .as_ref()
            .map(SecretString::expose),
        Some("msa-refresh-2")
    );
    assert_eq!(saved.minecraft.as_ref(), Some(&session));

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(cache.path())
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600, "cache file mode {mode:o}");
        let dir = cache.path().parent().unwrap();
        let mode = std::fs::metadata(dir).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o700, "cache dir mode {mode:o}");
    }
    // No temporary files are left behind.
    let entries: Vec<_> = std::fs::read_dir(cache.path().parent().unwrap())
        .unwrap()
        .collect();
    assert_eq!(entries.len(), 1);
}

#[tokio::test]
async fn azure_flow_uses_d_prefixed_ticket() {
    let server = MockServer::start().await;
    mount_device_code(&server, "/azure/devicecode").await;
    mount_token_success(&server, "/azure/token", "device_code=dev-code").await;
    mount_chain(&server, "d=msa-access").await;
    let auth = auth_for(&server, false);
    let mut message = None;
    let session = auth
        .authenticate(None, |code| message = code.message.clone())
        .await
        .unwrap();
    assert_eq!(session.profile.name, "Notch");
    assert!(message.unwrap().starts_with("To sign in"));
}

fn cached(client_id: &str, valid_for: Option<Duration>, refresh: Option<&str>) -> CachedAccount {
    CachedAccount {
        version: CachedAccount::VERSION,
        client_id: client_id.into(),
        microsoft_refresh_token: refresh.map(SecretString::from),
        minecraft: valid_for.map(|d| MinecraftSession {
            access_token: "cached-mc-token".into(),
            expires_at: SystemTime::now() + d,
            profile: MinecraftProfile {
                id: profile_uuid(),
                name: "Cached".into(),
            },
        }),
    }
}

#[tokio::test]
async fn valid_cached_session_needs_no_network() {
    let server = MockServer::start().await;
    let cache = temp_cache("valid");
    cache
        .save(&cached(
            CLIENT_ID,
            Some(Duration::from_secs(3600)),
            Some("r"),
        ))
        .await
        .unwrap();
    let auth = auth_for(&server, false);
    let session = auth
        .authenticate(Some(&cache), |_| {
            panic!("device code must not be requested")
        })
        .await
        .unwrap();
    assert_eq!(session.profile.name, "Cached");
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn nearly_expired_session_is_refreshed() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/azure/token"))
        .and(body_string_contains("grant_type=refresh_token"))
        .and(body_string_contains("refresh_token=old-refresh"))
        .respond_with(json_response(
            200,
            json!({
                "access_token": "msa-access",
                "refresh_token": "msa-refresh-2",
                "expires_in": 3600,
            }),
        ))
        .expect(1)
        .mount(&server)
        .await;
    mount_chain(&server, "d=msa-access").await;
    let cache = temp_cache("refresh");
    // Valid for one more minute only: below the five-minute margin.
    cache
        .save(&cached(
            CLIENT_ID,
            Some(Duration::from_secs(60)),
            Some("old-refresh"),
        ))
        .await
        .unwrap();
    let auth = auth_for(&server, false);
    let session = auth
        .authenticate(Some(&cache), |_| {
            panic!("device code must not be requested")
        })
        .await
        .unwrap();
    assert_eq!(session.profile.name, "Notch");
    let saved = cache.load().await.unwrap().unwrap();
    assert_eq!(
        saved.microsoft_refresh_token.unwrap().expose(),
        "msa-refresh-2"
    );
}

#[tokio::test]
async fn rejected_refresh_token_falls_back_to_device_code() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/azure/token"))
        .and(body_string_contains("grant_type=refresh_token"))
        .respond_with(json_response(
            400,
            json!({ "error": "invalid_grant", "error_description": "AADSTS70000: expired" }),
        ))
        .expect(2)
        .mount(&server)
        .await;
    mount_device_code(&server, "/azure/devicecode").await;
    mount_token_success(&server, "/azure/token", "device_code=dev-code").await;
    mount_chain(&server, "d=msa-access").await;
    let cache = temp_cache("rejected");
    cache
        .save(&cached(CLIENT_ID, None, Some("dead")))
        .await
        .unwrap();
    let auth = auth_for(&server, false);
    let mut asked = false;
    auth.authenticate(Some(&cache), |_| asked = true)
        .await
        .unwrap();
    assert!(asked);

    // Calling refresh directly reports the rejection.
    let err = auth.refresh(&"dead".into()).await.unwrap_err();
    assert!(
        matches!(err, MicrosoftError::RefreshTokenRejected),
        "{err:?}"
    );
}

#[tokio::test]
async fn refresh_outage_does_not_prompt_the_user() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/azure/token"))
        .respond_with(ResponseTemplate::new(503).set_body_string("down"))
        .mount(&server)
        .await;
    let cache = temp_cache("outage");
    cache
        .save(&cached(CLIENT_ID, None, Some("r")))
        .await
        .unwrap();
    let err = auth_for(&server, false)
        .authenticate(Some(&cache), |_| panic!("must not prompt"))
        .await
        .unwrap_err();
    assert!(
        matches!(
            err,
            MicrosoftError::Status {
                stage: Stage::MicrosoftToken,
                status: 503,
                ..
            }
        ),
        "{err:?}"
    );
}

#[tokio::test]
async fn cache_of_other_client_or_corrupt_cache_is_ignored() {
    for (name, contents) in [
        (
            "other-client",
            serde_json::to_string(&cached(
                "another-client",
                Some(Duration::from_secs(3600)),
                Some("r"),
            ))
            .unwrap(),
        ),
        ("corrupt", "{ not json".to_owned()),
        (
            "future-version",
            json!({"version": 99, "client_id": CLIENT_ID}).to_string(),
        ),
    ] {
        let server = MockServer::start().await;
        mount_device_code(&server, "/azure/devicecode").await;
        mount_token_success(&server, "/azure/token", "device_code=dev-code").await;
        mount_chain(&server, "d=msa-access").await;
        let cache = temp_cache(name);
        std::fs::create_dir_all(cache.path().parent().unwrap()).unwrap();
        std::fs::write(cache.path(), contents).unwrap();
        let mut asked = false;
        let session = auth_for(&server, false)
            .authenticate(Some(&cache), |_| asked = true)
            .await
            .unwrap();
        assert!(asked, "{name}");
        assert_eq!(session.profile.name, "Notch", "{name}");
        // The cache was rewritten in the current format.
        assert_eq!(
            cache.load().await.unwrap().unwrap().client_id,
            CLIENT_ID,
            "{name}"
        );
    }
}

#[tokio::test]
async fn poll_results() {
    let server = MockServer::start().await;
    mount_device_code(&server, "/azure/devicecode").await;
    let auth = auth_for(&server, false);
    let code = auth.request_device_code().await.unwrap();
    assert_eq!(code.interval, Duration::ZERO);
    assert!(!format!("{code:?}").contains("dev-code"));

    for (error, check) in [
        ("slow_down", 0),
        ("authorization_pending", 1),
        ("expired_token", 2),
        ("authorization_declined", 3),
        ("bad_verification_code", 4),
    ] {
        server.reset().await;
        Mock::given(method("POST"))
            .and(path("/azure/token"))
            .respond_with(json_response(400, json!({ "error": error })))
            .mount(&server)
            .await;
        let result = auth.poll_device_code_once(&code).await;
        match check {
            0 => assert!(matches!(result, Ok(DevicePoll::SlowDown)), "{result:?}"),
            1 => assert!(matches!(result, Ok(DevicePoll::Pending)), "{result:?}"),
            2 => assert!(
                matches!(result, Err(MicrosoftError::DeviceCodeExpired)),
                "{result:?}"
            ),
            3 => assert!(
                matches!(result, Err(MicrosoftError::AuthorizationDeclined)),
                "{result:?}"
            ),
            _ => assert!(
                matches!(&result, Err(MicrosoftError::OAuth { error, .. }) if error == "bad_verification_code"),
                "{result:?}"
            ),
        }
    }

    // The polling loop surfaces expiry too.
    let err = auth.poll_device_code(&code).await.unwrap_err();
    assert!(matches!(err, MicrosoftError::OAuth { .. }), "{err:?}");
}

#[tokio::test]
async fn xsts_refusals_are_classified() {
    for (code, expected) in [
        (2148916233u64, XboxError::NoXboxAccount),
        (2148916235, XboxError::CountryUnavailable),
        (2148916236, XboxError::AdultVerificationRequired),
        (2148916238, XboxError::ChildAccount),
        (123, XboxError::Other(123)),
    ] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/xsts/authorize"))
            .respond_with(json_response(
                401,
                json!({ "Identity": "0", "XErr": code, "Message": "", "Redirect": "https://start.ui.xboxlive.com/CreateAccount" }),
            ))
            .mount(&server)
            .await;
        let err = auth_for(&server, false)
            .xsts_token(&"xbl".into())
            .await
            .unwrap_err();
        assert!(
            matches!(err, MicrosoftError::Xbox(e) if e == expected),
            "{code}: {err:?}"
        );
    }
}

#[tokio::test]
async fn no_license_and_rejected_token() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/minecraft/profile"))
        .and(header("authorization", "Bearer no-game"))
        .respond_with(json_response(
            404,
            json!({ "path": "/minecraft/profile", "errorType": "NOT_FOUND", "error": "NOT_FOUND" }),
        ))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/minecraft/profile"))
        .and(header("authorization", "Bearer expired"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&server)
        .await;
    let auth = auth_for(&server, false);
    assert!(matches!(
        auth.profile(&"no-game".into()).await,
        Err(MicrosoftError::NoMinecraftLicense)
    ));
    assert!(matches!(
        auth.profile(&"expired".into()).await,
        Err(MicrosoftError::MinecraftTokenRejected)
    ));
}

#[tokio::test]
async fn rate_limit_and_unapproved_app() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/authentication/login_with_xbox"))
        .and(body_partial_json(
            json!({ "identityToken": "XBL3.0 x=u;limited" }),
        ))
        .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "30"))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/authentication/login_with_xbox"))
        .and(body_partial_json(json!({ "identityToken": "XBL3.0 x=u;unapproved" })))
        .respond_with(json_response(
            403,
            json!({
                "path": "/authentication/login_with_xbox",
                "errorMessage": "Invalid app registration, see https://aka.ms/AppRegInfo for more information",
            }),
        ))
        .mount(&server)
        .await;
    let auth = auth_for(&server, false);
    let xsts = |token: &str| ghast_auth::microsoft::XboxToken {
        token: token.into(),
        user_hash: "u".into(),
    };
    let err = auth.minecraft_login(&xsts("limited")).await.unwrap_err();
    assert!(
        matches!(
            err,
            MicrosoftError::RateLimited {
                stage: Stage::MinecraftLogin,
                retry_after: Some(d),
            } if d == Duration::from_secs(30)
        ),
        "{err:?}"
    );
    let err = auth.minecraft_login(&xsts("unapproved")).await.unwrap_err();
    assert!(matches!(err, MicrosoftError::AppNotPermitted), "{err:?}");
}

#[tokio::test]
async fn invalid_responses() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/user/authenticate"))
        .and(body_partial_json(
            json!({ "Properties": { "RpsTicket": "d=garbage" } }),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_string("<html>oops</html>"))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/user/authenticate"))
        .and(body_partial_json(
            json!({ "Properties": { "RpsTicket": "d=no-xui" } }),
        ))
        .respond_with(json_response(
            200,
            json!({ "Token": "t", "DisplayClaims": { "xui": [] } }),
        ))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/user/authenticate"))
        .and(body_partial_json(
            json!({ "Properties": { "RpsTicket": "d=huge" } }),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![b' '; 2 << 20]))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/minecraft/profile"))
        .respond_with(json_response(
            200,
            json!({ "id": "not-a-uuid", "name": "x" }),
        ))
        .mount(&server)
        .await;
    let auth = auth_for(&server, false);
    for ticket in ["garbage", "no-xui", "huge"] {
        let err = auth.xbox_user_token(&ticket.into()).await.unwrap_err();
        assert!(
            matches!(
                err,
                MicrosoftError::InvalidResponse {
                    stage: Stage::XboxUser,
                    ..
                }
            ),
            "{ticket}: {err:?}"
        );
    }
    let err = auth.profile(&"t".into()).await.unwrap_err();
    assert!(
        matches!(
            err,
            MicrosoftError::InvalidResponse {
                stage: Stage::Profile,
                ..
            }
        ),
        "{err:?}"
    );
}

#[tokio::test]
async fn session_join() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/session/minecraft/join"))
        .and(body_partial_json(json!({
            "accessToken": "good",
            "selectedProfile": PROFILE_ID,
            "serverId": "-7c9d5b0044c130109a5d7b5fb5c317c02b4e28c1",
        })))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    for (token, error) in [
        ("bad", "ForbiddenOperationException"),
        ("muted", "InsufficientPrivilegesException"),
        ("banned", "UserBannedException"),
        ("weird", "SomethingNewException"),
    ] {
        Mock::given(method("POST"))
            .and(path("/session/minecraft/join"))
            .and(body_partial_json(json!({ "accessToken": token })))
            .respond_with(json_response(
                403,
                json!({ "error": error, "errorMessage": "nope", "path": "/session/minecraft/join" }),
            ))
            .mount(&server)
            .await;
    }
    Mock::given(method("POST"))
        .and(path("/session/minecraft/join"))
        .and(body_partial_json(json!({ "accessToken": "limited" })))
        .respond_with(ResponseTemplate::new(429))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/session/minecraft/join"))
        .and(body_partial_json(json!({ "accessToken": "broken" })))
        .respond_with(ResponseTemplate::new(500).set_body_string("Internal error"))
        .mount(&server)
        .await;

    let auth = auth_for(&server, false);
    let hash = "-7c9d5b0044c130109a5d7b5fb5c317c02b4e28c1";
    let id = profile_uuid();
    auth.join_server(&"good".into(), id, hash).await.unwrap();

    let expected = [
        ("bad", JoinRejection::InvalidToken),
        ("muted", JoinRejection::MultiplayerDisabled),
        ("banned", JoinRejection::Banned),
        (
            "weird",
            JoinRejection::Other("SomethingNewException".into()),
        ),
    ];
    for (token, kind) in expected {
        let err = auth.join_server(&token.into(), id, hash).await.unwrap_err();
        assert!(
            matches!(&err, MicrosoftError::JoinRejected { kind: k, message } if *k == kind && message == "nope"),
            "{token}: {err:?}"
        );
    }
    assert!(matches!(
        auth.join_server(&"limited".into(), id, hash).await,
        Err(MicrosoftError::RateLimited {
            stage: Stage::Join,
            retry_after: None
        })
    ));
    assert!(matches!(
        auth.join_server(&"broken".into(), id, hash).await,
        Err(MicrosoftError::Status {
            stage: Stage::Join,
            status: 500,
            ..
        })
    ));
}

#[tokio::test]
async fn cache_clear_and_missing_file() {
    let cache = temp_cache("clear");
    assert!(cache.load().await.unwrap().is_none());
    cache.clear().await.unwrap();
    cache
        .save(&cached(CLIENT_ID, Some(Duration::from_secs(10)), None))
        .await
        .unwrap();
    assert!(cache.load().await.unwrap().is_some());
    // Overwriting keeps the restrictive mode and leaves no temporary files.
    cache.save(&cached(CLIENT_ID, None, None)).await.unwrap();
    assert!(cache.load().await.unwrap().unwrap().minecraft.is_none());
    cache.clear().await.unwrap();
    assert!(cache.load().await.unwrap().is_none());
    let left: Vec<_> = std::fs::read_dir(cache.path().parent().unwrap())
        .unwrap()
        .collect();
    assert!(left.is_empty());
}

#[test]
fn errors_do_not_leak_tokens() {
    let session = MinecraftSession {
        access_token: "super-secret-token".into(),
        expires_at: SystemTime::now(),
        profile: MinecraftProfile {
            id: Uuid::nil(),
            name: "x".into(),
        },
    };
    assert!(!format!("{session:?}").contains("super-secret-token"));
}

fn b64(data: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::new();
    for c in data.chunks(3) {
        let n = (u32::from(c[0]) << 16)
            | (u32::from(*c.get(1).unwrap_or(&0)) << 8)
            | u32::from(*c.get(2).unwrap_or(&0));
        for i in 0..4 {
            if i <= c.len() {
                s.push(A[(n >> (18 - 6 * i)) as usize & 63] as char);
            } else {
                s.push('=');
            }
        }
    }
    s
}

/// `player/certificates`: the key pair in PEM blocks, the V2 signature, ISO-8601 times; the
/// keys parse and sign (the signature verifies with the public key).
#[tokio::test]
async fn profile_keys_are_fetched_and_usable() {
    use rsa::pkcs8::{EncodePrivateKey, EncodePublicKey};
    use rsa::{Pkcs1v15Sign, RsaPrivateKey};
    use sha2::{Digest, Sha256};

    let server = MockServer::start().await;
    let mut rng = rand_core::OsRng;
    let key = RsaPrivateKey::new(&mut rng, 1024).unwrap();
    let private = key.to_pkcs8_der().unwrap();
    let public = key.to_public_key().to_public_key_der().unwrap();
    // Crypt.pemRsaPrivateKeyToString: MIME lines of 76.
    let pem = |tag: &str, der: &[u8]| {
        let body = b64(der);
        let lines: Vec<&str> = body
            .as_bytes()
            .chunks(76)
            .map(|c| std::str::from_utf8(c).unwrap())
            .collect();
        format!(
            "-----BEGIN {tag}-----\n{}\n-----END {tag}-----\n",
            lines.join("\n")
        )
    };
    Mock::given(method("POST"))
        .and(path("/player/certificates"))
        .and(header("authorization", "Bearer mc-token"))
        .respond_with(json_response(
            200,
            json!({
                "keyPair": {
                    "privateKey": pem("RSA PRIVATE KEY", private.as_bytes()),
                    "publicKey": pem("RSA PUBLIC KEY", public.as_bytes()),
                },
                "publicKeySignature": b64(&[1, 2, 3]),
                "publicKeySignatureV2": b64(&[9; 512]),
                "expiresAt": "2026-11-13T10:20:30.123456Z",
                "refreshedAfter": "2026-11-12T22:20:30.123456Z",
            }),
        ))
        .expect(1)
        .mount(&server)
        .await;
    let auth = auth_for(&server, true);
    assert_eq!(
        auth.certificates_url(),
        format!("{}/player/certificates", server.uri())
    );
    let keys = auth
        .profile_keys(&SecretString::new("mc-token".to_owned()))
        .await
        .unwrap();
    assert_eq!(keys.public_key_der(), public.as_bytes());
    assert_eq!(keys.key_signature(), &[9; 512][..]);
    assert_eq!(keys.expires_at_millis(), 1_794_565_230_123);
    let sig = keys.sign(b"hello").unwrap();
    let digest = Sha256::digest(b"hello");
    let scheme = Pkcs1v15Sign {
        hash_len: Some(32),
        prefix: Box::new([
            0x30, 0x31, 0x30, 0x0d, 0x06, 0x09, 0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02,
            0x01, 0x05, 0x00, 0x04, 0x20,
        ]),
    };
    key.to_public_key()
        .verify(scheme, &digest[..], &sig)
        .unwrap();
}
