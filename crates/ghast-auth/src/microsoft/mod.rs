//! Microsoft account login and the session-server `join` call (cargo feature `microsoft`).
//!
//! The chain, as done by the vanilla launcher:
//!
//! 1. **Microsoft OAuth** (device-code flow, RFC 8628): the user opens a URL, enters a short code
//!    and signs in; the client polls for an access token and a refresh token.
//! 2. **Xbox Live** user authentication with the Microsoft access token.
//! 3. **XSTS** authorization for the `rp://api.minecraftservices.com/` relying party.
//! 4. **Minecraft services** `login_with_xbox`, which returns the Minecraft access token.
//! 5. The Minecraft **profile** (UUID and name). An account without the game has none.
//!
//! [`MicrosoftAuth::authenticate`] runs the whole chain and keeps the result in a
//! [`TokenCache`] file, so later runs reuse the Minecraft token or refresh it silently.
//!
//! # OAuth client
//!
//! Microsoft only issues tokens to a registered OAuth client, chosen with [`MicrosoftConfig`]:
//!
//! * [`OAuthFlavor::AzureAd`]: your own Azure app registration (`login.microsoftonline.com`,
//!   scope `XboxLive.signin offline_access`). Mojang must approve new registrations for the
//!   Minecraft API, otherwise `login_with_xbox` fails with [`MicrosoftError::AppNotPermitted`].
//! * [`OAuthFlavor::Live`]: a client id registered with the older Live endpoint
//!   (`login.live.com`, scope `service::user.auth.xboxlive.com::MBI_SSL`).
//!
//! This crate ships no client id; the application decides which one it uses.
//!
//! # Proxies
//!
//! All requests use the [`reqwest::Client`] passed to [`MicrosoftAuth::with_http_client`]. To send
//! them through the bot's SOCKS5 proxy, build that client with `reqwest::Proxy::all("socks5h://…")`
//! (this needs reqwest's `socks` feature in the application).
//!
//! # Example
//!
//! ```no_run
//! use ghast_auth::microsoft::{MicrosoftAuth, MicrosoftConfig, TokenCache};
//!
//! # async fn run() -> Result<(), ghast_auth::microsoft::MicrosoftError> {
//! let auth = MicrosoftAuth::new(MicrosoftConfig::azure_ad("your-azure-client-id"))?;
//! let cache = TokenCache::new("mybot/account.json");
//! let session = auth
//!     .authenticate(Some(&cache), |code| {
//!         println!("Open {} and enter {}", code.verification_uri, code.user_code);
//!     })
//!     .await?;
//! println!("logged in as {} ({})", session.profile.name, session.profile.id);
//!
//! // Later, during login to an online-mode server, after computing the server hash:
//! # let server_hash = String::new();
//! auth.join_server(&session.access_token, session.profile.id, &server_hash).await?;
//! # Ok(())
//! # }
//! ```

mod cache;

use std::fmt;
use std::time::{Duration, Instant, SystemTime};

use reqwest::{Response, StatusCode};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::json;
use thiserror::Error;
use uuid::Uuid;

use crate::SecretString;

pub use cache::{CacheError, CachedAccount, TokenCache};

/// Largest response body accepted from any endpoint.
const MAX_BODY: usize = 1 << 20;
/// How much of an unexpected error body is kept in [`MicrosoftError::Status`].
const MAX_ERROR_BODY: usize = 512;
/// A cached Minecraft token is reused only if it is valid for at least this long.
const MIN_REMAINING: Duration = Duration::from_secs(5 * 60);
/// RFC 8628 §3.5: on `slow_down` the polling interval grows by five seconds.
const SLOW_DOWN_STEP: Duration = Duration::from_secs(5);

const LIVE_SCOPE: &str = "service::user.auth.xboxlive.com::MBI_SSL";
const AZURE_SCOPE: &str = "XboxLive.signin offline_access";
const DEVICE_CODE_GRANT: &str = "urn:ietf:params:oauth:grant-type:device_code";

/// Which Microsoft OAuth endpoint family the client id belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OAuthFlavor {
    /// `login.live.com` with the `MBI_SSL` scope; the Xbox ticket is the bare access token.
    Live,
    /// `login.microsoftonline.com/consumers` (Azure AD v2) with the `XboxLive.signin` scope; the
    /// Xbox ticket is `d=<access token>`.
    AzureAd,
}

impl OAuthFlavor {
    fn scope(self) -> &'static str {
        match self {
            Self::Live => LIVE_SCOPE,
            Self::AzureAd => AZURE_SCOPE,
        }
    }

    fn rps_ticket(self, access_token: &str) -> String {
        match self {
            Self::Live => access_token.to_owned(),
            Self::AzureAd => format!("d={access_token}"),
        }
    }
}

/// The URLs of every service in the chain. Override them to talk to a mock server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoints {
    /// Live device-code endpoint.
    pub live_device_code: String,
    /// Live token endpoint (polling and refresh).
    pub live_token: String,
    /// Azure AD device-code endpoint.
    pub azure_device_code: String,
    /// Azure AD token endpoint (polling and refresh).
    pub azure_token: String,
    /// Xbox Live user authentication.
    pub xbox_user_auth: String,
    /// XSTS authorization.
    pub xsts_authorize: String,
    /// Minecraft services `login_with_xbox`.
    pub minecraft_login: String,
    /// Minecraft services profile.
    pub minecraft_profile: String,
    /// Session server `join`.
    pub session_join: String,
}

impl Default for Endpoints {
    fn default() -> Self {
        Self {
            live_device_code: "https://login.live.com/oauth20_connect.srf".into(),
            live_token: "https://login.live.com/oauth20_token.srf".into(),
            azure_device_code: "https://login.microsoftonline.com/consumers/oauth2/v2.0/devicecode"
                .into(),
            azure_token: "https://login.microsoftonline.com/consumers/oauth2/v2.0/token".into(),
            xbox_user_auth: "https://user.auth.xboxlive.com/user/authenticate".into(),
            xsts_authorize: "https://xsts.auth.xboxlive.com/xsts/authorize".into(),
            minecraft_login: "https://api.minecraftservices.com/authentication/login_with_xbox"
                .into(),
            minecraft_profile: "https://api.minecraftservices.com/minecraft/profile".into(),
            session_join: "https://sessionserver.mojang.com/session/minecraft/join".into(),
        }
    }
}

impl Endpoints {
    /// Every endpoint under one base URL (for mock servers), using the same paths as the real
    /// services with a short prefix: `/live/oauth20_connect.srf`, `/live/oauth20_token.srf`,
    /// `/azure/devicecode`, `/azure/token`, `/user/authenticate`, `/xsts/authorize`,
    /// `/authentication/login_with_xbox`, `/minecraft/profile`, `/session/minecraft/join`.
    pub fn with_base_url(base: &str) -> Self {
        let base = base.trim_end_matches('/');
        Self {
            live_device_code: format!("{base}/live/oauth20_connect.srf"),
            live_token: format!("{base}/live/oauth20_token.srf"),
            azure_device_code: format!("{base}/azure/devicecode"),
            azure_token: format!("{base}/azure/token"),
            xbox_user_auth: format!("{base}/user/authenticate"),
            xsts_authorize: format!("{base}/xsts/authorize"),
            minecraft_login: format!("{base}/authentication/login_with_xbox"),
            minecraft_profile: format!("{base}/minecraft/profile"),
            session_join: format!("{base}/session/minecraft/join"),
        }
    }
}

/// Configuration of [`MicrosoftAuth`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MicrosoftConfig {
    /// The OAuth client id.
    pub client_id: String,
    /// The endpoint family the client id belongs to.
    pub flavor: OAuthFlavor,
    /// Service URLs.
    pub endpoints: Endpoints,
}

impl MicrosoftConfig {
    /// An Azure AD (v2, `consumers` tenant) client id.
    pub fn azure_ad(client_id: impl Into<String>) -> Self {
        Self {
            client_id: client_id.into(),
            flavor: OAuthFlavor::AzureAd,
            endpoints: Endpoints::default(),
        }
    }

    /// A Live (`login.live.com`) client id.
    pub fn live(client_id: impl Into<String>) -> Self {
        Self {
            client_id: client_id.into(),
            flavor: OAuthFlavor::Live,
            endpoints: Endpoints::default(),
        }
    }

    /// Replaces the endpoints.
    pub fn with_endpoints(mut self, endpoints: Endpoints) -> Self {
        self.endpoints = endpoints;
        self
    }

    fn device_code_url(&self) -> &str {
        match self.flavor {
            OAuthFlavor::Live => &self.endpoints.live_device_code,
            OAuthFlavor::AzureAd => &self.endpoints.azure_device_code,
        }
    }

    fn token_url(&self) -> &str {
        match self.flavor {
            OAuthFlavor::Live => &self.endpoints.live_token,
            OAuthFlavor::AzureAd => &self.endpoints.azure_token,
        }
    }
}

/// Which request of the chain failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Stage {
    /// Requesting a device code.
    DeviceCode,
    /// Polling for, or refreshing, the Microsoft token.
    MicrosoftToken,
    /// Xbox Live user authentication.
    XboxUser,
    /// XSTS authorization.
    Xsts,
    /// Minecraft `login_with_xbox`.
    MinecraftLogin,
    /// Minecraft profile.
    Profile,
    /// Session server `join`.
    Join,
    /// Minecraft services `player/certificates` (the profile key pair for signed chat).
    ProfileKeys,
}

impl fmt::Display for Stage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::DeviceCode => "device code request",
            Self::MicrosoftToken => "Microsoft token request",
            Self::XboxUser => "Xbox Live authentication",
            Self::Xsts => "XSTS authorization",
            Self::MinecraftLogin => "Minecraft login",
            Self::Profile => "Minecraft profile request",
            Self::Join => "session server join",
            Self::ProfileKeys => "profile key pair request",
        })
    }
}

/// XSTS refusals (the `XErr` codes of a 401 response).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum XboxError {
    /// `2148916233`: the Microsoft account has no Xbox profile yet (sign in on xbox.com once).
    NoXboxAccount,
    /// `2148916235`: Xbox Live is not available in the account's country.
    CountryUnavailable,
    /// `2148916236` / `2148916237`: the account needs adult verification (South Korea).
    AdultVerificationRequired,
    /// `2148916238`: a child account that must be added to a Microsoft family by an adult.
    ChildAccount,
    /// Any other `XErr` code.
    Other(u64),
}

impl XboxError {
    /// Maps an `XErr` code.
    pub fn from_code(code: u64) -> Self {
        match code {
            2_148_916_233 => Self::NoXboxAccount,
            2_148_916_235 => Self::CountryUnavailable,
            2_148_916_236 | 2_148_916_237 => Self::AdultVerificationRequired,
            2_148_916_238 => Self::ChildAccount,
            other => Self::Other(other),
        }
    }
}

impl fmt::Display for XboxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoXboxAccount => f.write_str("the Microsoft account has no Xbox profile"),
            Self::CountryUnavailable => f.write_str("Xbox Live is not available in this country"),
            Self::AdultVerificationRequired => f.write_str("the account needs adult verification"),
            Self::ChildAccount => {
                f.write_str("child account: an adult must add it to a Microsoft family")
            }
            Self::Other(code) => write!(f, "XSTS error {code}"),
        }
    }
}

/// Why the session server refused a `join`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JoinRejection {
    /// `ForbiddenOperationException`: the access token is invalid or expired (log in again).
    InvalidToken,
    /// `InsufficientPrivilegesException`: multiplayer is disabled for this account.
    MultiplayerDisabled,
    /// `UserBannedException`: the account is banned from multiplayer.
    Banned,
    /// Another error type, as reported by the server.
    Other(String),
}

/// Errors from the Microsoft login chain and the session server.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum MicrosoftError {
    /// Network or TLS error.
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    /// HTTP 429. `retry_after` comes from the `Retry-After` header when present.
    #[error("{stage}: rate limited (retry after {retry_after:?})")]
    RateLimited {
        /// The failing request.
        stage: Stage,
        /// How long the server asked to wait.
        retry_after: Option<Duration>,
    },
    /// An unexpected HTTP status.
    #[error("{stage}: HTTP {status}: {body}")]
    Status {
        /// The failing request.
        stage: Stage,
        /// The status code.
        status: u16,
        /// The start of the response body.
        body: String,
    },
    /// The response was not the expected JSON or was too large.
    #[error("{stage}: invalid response: {reason}")]
    InvalidResponse {
        /// The failing request.
        stage: Stage,
        /// What was wrong.
        reason: String,
    },
    /// An OAuth error not covered by a more specific variant.
    #[error("OAuth error {error}: {}", description.as_deref().unwrap_or(""))]
    OAuth {
        /// The `error` field.
        error: String,
        /// The `error_description` field.
        description: Option<String>,
    },
    /// The device code expired before the user signed in.
    #[error("the device code expired before the user signed in")]
    DeviceCodeExpired,
    /// The user declined the sign-in.
    #[error("the user declined the sign-in")]
    AuthorizationDeclined,
    /// The refresh token is invalid, expired or revoked; a new device-code login is needed.
    #[error("the refresh token was rejected; sign in again")]
    RefreshTokenRejected,
    /// XSTS refused the account.
    #[error("XSTS refused the account: {0}")]
    Xbox(XboxError),
    /// Minecraft services refused the OAuth client id (`Invalid app registration`): the Azure app
    /// is not approved for the Minecraft API.
    #[error("Minecraft services do not accept this OAuth client id (app not approved)")]
    AppNotPermitted,
    /// The account does not own Minecraft Java Edition (no profile).
    #[error("this account does not own Minecraft Java Edition")]
    NoMinecraftLicense,
    /// Minecraft services rejected the Minecraft access token (expired or revoked).
    #[error("the Minecraft access token was rejected")]
    MinecraftTokenRejected,
    /// The session server refused the `join`.
    #[error("session server refused the join: {kind:?}: {message}")]
    JoinRejected {
        /// The reason.
        kind: JoinRejection,
        /// The server's `errorMessage`.
        message: String,
    },
    /// The token cache could not be read or written.
    #[error("token cache: {0}")]
    Cache(#[from] CacheError),
}

/// A pending device-code login: show [`DeviceCode::user_code`] and
/// [`DeviceCode::verification_uri`] to the user.
#[derive(Clone)]
pub struct DeviceCode {
    /// The short code the user types in.
    pub user_code: String,
    /// The page where the user enters the code.
    pub verification_uri: String,
    /// A ready-made instruction from Microsoft, when provided (Azure AD only).
    pub message: Option<String>,
    /// How long the code is valid.
    pub expires_in: Duration,
    /// The polling interval requested by the server.
    pub interval: Duration,
    device_code: SecretString,
    deadline: Instant,
}

impl DeviceCode {
    /// When the code stops being valid.
    pub fn deadline(&self) -> Instant {
        self.deadline
    }
}

impl fmt::Debug for DeviceCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DeviceCode")
            .field("user_code", &self.user_code)
            .field("verification_uri", &self.verification_uri)
            .field("expires_in", &self.expires_in)
            .field("interval", &self.interval)
            .finish_non_exhaustive()
    }
}

/// The result of one poll of the token endpoint.
#[derive(Debug, Clone)]
pub enum DevicePoll {
    /// The user has not finished signing in yet.
    Pending,
    /// The server asks to poll less often (RFC 8628 `slow_down`).
    SlowDown,
    /// The user signed in.
    Complete(MicrosoftToken),
}

/// A Microsoft OAuth token pair.
#[derive(Debug, Clone)]
pub struct MicrosoftToken {
    /// The access token (used for Xbox Live authentication).
    pub access_token: SecretString,
    /// The refresh token, if the server issued one.
    pub refresh_token: Option<SecretString>,
    /// When the access token expires.
    pub expires_at: SystemTime,
}

/// An Xbox Live or XSTS token with its user hash.
#[derive(Debug, Clone)]
pub struct XboxToken {
    /// The token.
    pub token: SecretString,
    /// The user hash (`uhs` claim).
    pub user_hash: String,
}

/// A Minecraft profile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MinecraftProfile {
    /// The player UUID.
    pub id: Uuid,
    /// The player name.
    pub name: String,
}

/// A logged-in Minecraft account: what the client needs to join online-mode servers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MinecraftSession {
    /// The Minecraft access token (for [`MicrosoftAuth::join_server`]).
    pub access_token: SecretString,
    /// When the access token expires.
    #[serde(with = "unix_seconds")]
    pub expires_at: SystemTime,
    /// The player's profile.
    pub profile: MinecraftProfile,
}

impl MinecraftSession {
    /// Returns `true` if the token is still valid for at least `margin`.
    pub fn is_valid_for(&self, margin: Duration) -> bool {
        SystemTime::now()
            .checked_add(margin)
            .is_some_and(|t| t < self.expires_at)
    }
}

mod unix_seconds {
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use serde::{Deserialize, Deserializer, Serializer};

    pub(super) fn serialize<S: Serializer>(t: &SystemTime, s: S) -> Result<S::Ok, S::Error> {
        let secs = t
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        s.serialize_u64(secs)
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<SystemTime, D::Error> {
        let secs = u64::deserialize(d)?;
        Ok(UNIX_EPOCH
            .checked_add(Duration::from_secs(secs))
            .unwrap_or(UNIX_EPOCH))
    }
}

/// Microsoft account login client.
#[derive(Debug, Clone)]
pub struct MicrosoftAuth {
    http: reqwest::Client,
    config: MicrosoftConfig,
    /// `player/certificates` (see [`MicrosoftAuth::with_certificates_url`]).
    certificates: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CertificatesResponse {
    key_pair: CertificatesKeyPair,
    #[serde(rename = "publicKeySignatureV2")]
    public_key_signature_v2: String,
    expires_at: String,
    refreshed_after: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CertificatesKeyPair {
    private_key: String,
    public_key: String,
}

#[derive(Deserialize)]
struct DeviceCodeResponse {
    user_code: String,
    device_code: String,
    #[serde(alias = "verification_url")]
    verification_uri: String,
    expires_in: u64,
    #[serde(default)]
    interval: Option<u64>,
    #[serde(default)]
    message: Option<String>,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: Option<u64>,
}

#[derive(Deserialize)]
struct OAuthErrorResponse {
    error: String,
    #[serde(default)]
    error_description: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct XboxResponse {
    token: String,
    display_claims: DisplayClaims,
}

#[derive(Deserialize)]
struct DisplayClaims {
    xui: Vec<Xui>,
}

#[derive(Deserialize)]
struct Xui {
    uhs: String,
}

#[derive(Deserialize)]
struct XErrResponse {
    #[serde(rename = "XErr")]
    xerr: u64,
}

#[derive(Deserialize)]
struct MinecraftLoginResponse {
    access_token: String,
    expires_in: u64,
}

#[derive(Deserialize)]
struct ProfileResponse {
    id: String,
    name: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MojangError {
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    error_message: Option<String>,
}

impl MicrosoftAuth {
    /// A client with its own HTTPS client (rustls, 10 s connect / 30 s request timeouts).
    pub fn new(config: MicrosoftConfig) -> Result<Self, MicrosoftError> {
        let http = reqwest::Client::builder()
            .user_agent(concat!("ghast-auth/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .build()?;
        Ok(Self {
            http,
            config,
            certificates: None,
        })
    }

    /// A client using the given HTTP client (for proxies or custom TLS settings).
    pub fn with_http_client(config: MicrosoftConfig, http: reqwest::Client) -> Self {
        Self {
            http,
            config,
            certificates: None,
        }
    }

    /// The `player/certificates` URL [`MicrosoftAuth::profile_keys`] posts to. Default: the
    /// profile endpoint's host with the path `/player/certificates`
    /// (`https://api.minecraftservices.com/player/certificates`, or `{base}/player/certificates`
    /// for [`Endpoints::with_base_url`]).
    #[must_use]
    pub fn with_certificates_url(mut self, url: impl Into<String>) -> Self {
        self.certificates = Some(url.into());
        self
    }

    /// The `player/certificates` URL in use.
    pub fn certificates_url(&self) -> String {
        if let Some(u) = &self.certificates {
            return u.clone();
        }
        let p = &self.config.endpoints.minecraft_profile;
        match p.strip_suffix("/minecraft/profile") {
            Some(base) => format!("{base}/player/certificates"),
            None => "https://api.minecraftservices.com/player/certificates".to_owned(),
        }
    }

    /// The configuration.
    pub fn config(&self) -> &MicrosoftConfig {
        &self.config
    }

    /// Full login: reuse a cached Minecraft token if it is valid for at least five more minutes,
    /// else refresh the Microsoft token, else run the device-code flow (calling `on_code` with
    /// the code to show the user). The result is written back to `cache`.
    ///
    /// A missing or unreadable (corrupt) cache file counts as empty. A rejected refresh token
    /// falls back to the device-code flow; other errors (network, rate limits, ...) are returned
    /// so that a temporary outage does not prompt the user for a new sign-in.
    pub async fn authenticate(
        &self,
        cache: Option<&TokenCache>,
        on_code: impl FnOnce(&DeviceCode),
    ) -> Result<MinecraftSession, MicrosoftError> {
        let cached = match cache {
            Some(cache) => match cache.load().await {
                Ok(account) => account,
                Err(CacheError::Format(_)) => None,
                Err(e) => return Err(e.into()),
            },
            None => None,
        }
        .filter(|account| account.client_id == self.config.client_id);

        if let Some(account) = &cached {
            if let Some(session) = &account.minecraft
                && session.is_valid_for(MIN_REMAINING)
            {
                return Ok(session.clone());
            }
            if let Some(refresh) = &account.microsoft_refresh_token {
                match self.refresh(refresh).await {
                    Ok(token) => return self.finish(cache, token).await,
                    Err(MicrosoftError::RefreshTokenRejected) => {}
                    Err(e) => return Err(e),
                }
            }
        }

        let code = self.request_device_code().await?;
        on_code(&code);
        let token = self.poll_device_code(&code).await?;
        self.finish(cache, token).await
    }

    async fn finish(
        &self,
        cache: Option<&TokenCache>,
        token: MicrosoftToken,
    ) -> Result<MinecraftSession, MicrosoftError> {
        let session = self.login_with_microsoft_token(&token.access_token).await?;
        if let Some(cache) = cache {
            let account = CachedAccount {
                version: CachedAccount::VERSION,
                client_id: self.config.client_id.clone(),
                microsoft_refresh_token: token.refresh_token,
                minecraft: Some(session.clone()),
            };
            cache.save(&account).await?;
        }
        Ok(session)
    }

    /// Xbox Live, XSTS, Minecraft login and profile, starting from a Microsoft access token.
    pub async fn login_with_microsoft_token(
        &self,
        access_token: &SecretString,
    ) -> Result<MinecraftSession, MicrosoftError> {
        let user = self.xbox_user_token(access_token).await?;
        let xsts = self.xsts_token(&user.token).await?;
        let (mc_token, expires_at) = self.minecraft_login(&xsts).await?;
        let profile = self.profile(&mc_token).await?;
        Ok(MinecraftSession {
            access_token: mc_token,
            expires_at,
            profile,
        })
    }

    /// Step 1a: requests a device code.
    pub async fn request_device_code(&self) -> Result<DeviceCode, MicrosoftError> {
        let stage = Stage::DeviceCode;
        let mut form = vec![
            ("client_id", self.config.client_id.as_str()),
            ("scope", self.config.flavor.scope()),
        ];
        if self.config.flavor == OAuthFlavor::Live {
            form.push(("response_type", "device_code"));
        }
        let resp = self
            .http
            .post(self.config.device_code_url())
            .form(&form)
            .send()
            .await?;
        let resp = check_status(resp, stage).await?;
        let body: DeviceCodeResponse = read_json(resp, stage).await?;
        let expires_in = Duration::from_secs(body.expires_in);
        Ok(DeviceCode {
            user_code: body.user_code,
            verification_uri: body.verification_uri,
            message: body.message,
            expires_in,
            interval: Duration::from_secs(body.interval.unwrap_or(5)),
            device_code: SecretString::new(body.device_code),
            deadline: Instant::now()
                .checked_add(expires_in)
                .unwrap_or_else(Instant::now),
        })
    }

    /// Step 1b: polls until the user has signed in, the code expires or the user declines.
    ///
    /// Waits [`DeviceCode::interval`] before each poll and grows it on `slow_down`.
    pub async fn poll_device_code(
        &self,
        code: &DeviceCode,
    ) -> Result<MicrosoftToken, MicrosoftError> {
        let mut interval = code.interval;
        loop {
            if Instant::now() >= code.deadline {
                return Err(MicrosoftError::DeviceCodeExpired);
            }
            tokio::time::sleep(interval).await;
            match self.poll_device_code_once(code).await? {
                DevicePoll::Pending => {}
                DevicePoll::SlowDown => interval += SLOW_DOWN_STEP,
                DevicePoll::Complete(token) => return Ok(token),
            }
        }
    }

    /// One poll of the token endpoint (for applications that drive the loop themselves).
    pub async fn poll_device_code_once(
        &self,
        code: &DeviceCode,
    ) -> Result<DevicePoll, MicrosoftError> {
        let form = [
            ("client_id", self.config.client_id.as_str()),
            ("device_code", code.device_code.expose()),
            ("grant_type", DEVICE_CODE_GRANT),
        ];
        let resp = self
            .http
            .post(self.config.token_url())
            .form(&form)
            .send()
            .await?;
        match self.token_response(resp).await {
            Ok(token) => Ok(DevicePoll::Complete(token)),
            Err(MicrosoftError::OAuth { error, description }) => match error.as_str() {
                "authorization_pending" => Ok(DevicePoll::Pending),
                "slow_down" => Ok(DevicePoll::SlowDown),
                "expired_token" | "code_expired" => Err(MicrosoftError::DeviceCodeExpired),
                "authorization_declined" | "access_denied" => {
                    Err(MicrosoftError::AuthorizationDeclined)
                }
                _ => Err(MicrosoftError::OAuth { error, description }),
            },
            Err(e) => Err(e),
        }
    }

    /// Exchanges a refresh token for a new Microsoft token pair.
    pub async fn refresh(
        &self,
        refresh_token: &SecretString,
    ) -> Result<MicrosoftToken, MicrosoftError> {
        let form = [
            ("client_id", self.config.client_id.as_str()),
            ("scope", self.config.flavor.scope()),
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token.expose()),
        ];
        let resp = self
            .http
            .post(self.config.token_url())
            .form(&form)
            .send()
            .await?;
        match self.token_response(resp).await {
            Err(MicrosoftError::OAuth { error, .. }) if error == "invalid_grant" => {
                Err(MicrosoftError::RefreshTokenRejected)
            }
            other => other,
        }
    }

    /// Parses a token endpoint response; OAuth errors become [`MicrosoftError::OAuth`].
    async fn token_response(&self, resp: Response) -> Result<MicrosoftToken, MicrosoftError> {
        let stage = Stage::MicrosoftToken;
        let status = resp.status();
        if status == StatusCode::TOO_MANY_REQUESTS {
            return Err(rate_limited(&resp, stage));
        }
        let body = read_body(resp, stage).await?;
        if !status.is_success() {
            if let Ok(err) = serde_json::from_slice::<OAuthErrorResponse>(&body) {
                return Err(MicrosoftError::OAuth {
                    error: err.error,
                    description: err.error_description,
                });
            }
            return Err(status_error(stage, status, &body));
        }
        let token: TokenResponse = parse_json(&body, stage)?;
        Ok(MicrosoftToken {
            access_token: SecretString::new(token.access_token),
            refresh_token: token.refresh_token.map(SecretString::new),
            expires_at: expires_at(token.expires_in.unwrap_or(3600)),
        })
    }

    /// Step 2: Xbox Live user token from a Microsoft access token.
    pub async fn xbox_user_token(
        &self,
        microsoft_access_token: &SecretString,
    ) -> Result<XboxToken, MicrosoftError> {
        let body = json!({
            "Properties": {
                "AuthMethod": "RPS",
                "SiteName": "user.auth.xboxlive.com",
                "RpsTicket": self.config.flavor.rps_ticket(microsoft_access_token.expose()),
            },
            "RelyingParty": "http://auth.xboxlive.com",
            "TokenType": "JWT",
        });
        self.xbox_request(
            &self.config.endpoints.xbox_user_auth,
            &body,
            Stage::XboxUser,
        )
        .await
    }

    /// Step 3: XSTS token for Minecraft services from an Xbox Live user token.
    pub async fn xsts_token(&self, user_token: &SecretString) -> Result<XboxToken, MicrosoftError> {
        let body = json!({
            "Properties": {
                "SandboxId": "RETAIL",
                "UserTokens": [user_token.expose()],
            },
            "RelyingParty": "rp://api.minecraftservices.com/",
            "TokenType": "JWT",
        });
        self.xbox_request(&self.config.endpoints.xsts_authorize, &body, Stage::Xsts)
            .await
    }

    async fn xbox_request(
        &self,
        url: &str,
        body: &serde_json::Value,
        stage: Stage,
    ) -> Result<XboxToken, MicrosoftError> {
        let resp = self
            .http
            .post(url)
            .header("x-xbl-contract-version", "1")
            .header(reqwest::header::ACCEPT, "application/json")
            .json(body)
            .send()
            .await?;
        let status = resp.status();
        if status == StatusCode::TOO_MANY_REQUESTS {
            return Err(rate_limited(&resp, stage));
        }
        let bytes = read_body(resp, stage).await?;
        if status == StatusCode::UNAUTHORIZED
            && let Ok(err) = serde_json::from_slice::<XErrResponse>(&bytes)
        {
            return Err(MicrosoftError::Xbox(XboxError::from_code(err.xerr)));
        }
        if !status.is_success() {
            return Err(status_error(stage, status, &bytes));
        }
        let parsed: XboxResponse = parse_json(&bytes, stage)?;
        let user_hash = parsed
            .display_claims
            .xui
            .into_iter()
            .next()
            .map(|x| x.uhs)
            .ok_or_else(|| MicrosoftError::InvalidResponse {
                stage,
                reason: "no user hash in DisplayClaims.xui".into(),
            })?;
        Ok(XboxToken {
            token: SecretString::new(parsed.token),
            user_hash,
        })
    }

    /// Step 4: Minecraft access token from an XSTS token. Returns the token and its expiry.
    pub async fn minecraft_login(
        &self,
        xsts: &XboxToken,
    ) -> Result<(SecretString, SystemTime), MicrosoftError> {
        let stage = Stage::MinecraftLogin;
        let body = json!({
            "identityToken": format!("XBL3.0 x={};{}", xsts.user_hash, xsts.token.expose()),
        });
        let resp = self
            .http
            .post(&self.config.endpoints.minecraft_login)
            .json(&body)
            .send()
            .await?;
        let status = resp.status();
        if status == StatusCode::TOO_MANY_REQUESTS {
            return Err(rate_limited(&resp, stage));
        }
        let bytes = read_body(resp, stage).await?;
        if status == StatusCode::FORBIDDEN
            && String::from_utf8_lossy(&bytes).contains("Invalid app registration")
        {
            return Err(MicrosoftError::AppNotPermitted);
        }
        if !status.is_success() {
            return Err(status_error(stage, status, &bytes));
        }
        let parsed: MinecraftLoginResponse = parse_json(&bytes, stage)?;
        Ok((
            SecretString::new(parsed.access_token),
            expires_at(parsed.expires_in),
        ))
    }

    /// Step 5: the profile of the account owning `minecraft_access_token`.
    pub async fn profile(
        &self,
        minecraft_access_token: &SecretString,
    ) -> Result<MinecraftProfile, MicrosoftError> {
        let stage = Stage::Profile;
        let resp = self
            .http
            .get(&self.config.endpoints.minecraft_profile)
            .bearer_auth(minecraft_access_token.expose())
            .send()
            .await?;
        match resp.status() {
            StatusCode::NOT_FOUND => return Err(MicrosoftError::NoMinecraftLicense),
            StatusCode::UNAUTHORIZED => return Err(MicrosoftError::MinecraftTokenRejected),
            _ => {}
        }
        let resp = check_status(resp, stage).await?;
        let parsed: ProfileResponse = read_json(resp, stage).await?;
        let id = Uuid::parse_str(&parsed.id).map_err(|e| MicrosoftError::InvalidResponse {
            stage,
            reason: format!("bad profile id {:?}: {e}", parsed.id),
        })?;
        Ok(MinecraftProfile {
            id,
            name: parsed.name,
        })
    }

    /// The profile key pair for signed chat (`UserApiService.getKeyPair`: an empty `POST` to
    /// `player/certificates` with the Minecraft access token). Vanilla fetches it when it joins
    /// an online-mode server and again once [`crate::chat::ProfileKeys::due_refresh`].
    pub async fn profile_keys(
        &self,
        minecraft_access_token: &SecretString,
    ) -> Result<crate::chat::ProfileKeys, MicrosoftError> {
        let stage = Stage::ProfileKeys;
        let resp = self
            .http
            .post(self.certificates_url())
            .bearer_auth(minecraft_access_token.expose())
            .header("Content-Type", "application/json; charset=utf-8")
            .body("")
            .send()
            .await?;
        if resp.status() == StatusCode::UNAUTHORIZED {
            return Err(MicrosoftError::MinecraftTokenRejected);
        }
        if resp.status() == StatusCode::TOO_MANY_REQUESTS {
            return Err(rate_limited(&resp, stage));
        }
        let resp = check_status(resp, stage).await?;
        let parsed: CertificatesResponse = read_json(resp, stage).await?;
        let bad = |reason: String| MicrosoftError::InvalidResponse { stage, reason };
        let private = crate::chat::pem_body(&parsed.key_pair.private_key)
            .ok_or_else(|| bad("private key is not PEM/base64".into()))?;
        let public = crate::chat::pem_body(&parsed.key_pair.public_key)
            .ok_or_else(|| bad("public key is not PEM/base64".into()))?;
        let signature = crate::chat::base64_decode(parsed.public_key_signature_v2.trim())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| bad("missing public key signature".into()))?;
        let expires = parse_instant(&parsed.expires_at)
            .ok_or_else(|| bad(format!("bad expiresAt {:?}", parsed.expires_at)))?;
        let refresh = parse_instant(&parsed.refreshed_after)
            .ok_or_else(|| bad(format!("bad refreshedAfter {:?}", parsed.refreshed_after)))?;
        crate::chat::ProfileKeys::from_der(&private, &public, signature, expires, refresh)
            .map_err(|e| bad(e.to_string()))
    }

    /// The session-server `join` call an online-mode client makes during login, after
    /// receiving the Encryption Request and before sending the Encryption Response.
    ///
    /// `server_hash` comes from [`crate::crypto::server_hash`] (or
    /// [`crate::crypto::EncryptionResponse::server_hash`]).
    pub async fn join_server(
        &self,
        minecraft_access_token: &SecretString,
        profile_id: Uuid,
        server_hash: &str,
    ) -> Result<(), MicrosoftError> {
        let stage = Stage::Join;
        let body = json!({
            "accessToken": minecraft_access_token.expose(),
            "selectedProfile": profile_id.simple().to_string(),
            "serverId": server_hash,
        });
        let resp = self
            .http
            .post(&self.config.endpoints.session_join)
            .json(&body)
            .send()
            .await?;
        let status = resp.status();
        if status.is_success() {
            return Ok(());
        }
        if status == StatusCode::TOO_MANY_REQUESTS {
            return Err(rate_limited(&resp, stage));
        }
        let bytes = read_body(resp, stage).await?;
        if let Ok(MojangError {
            error: Some(error),
            error_message,
        }) = serde_json::from_slice::<MojangError>(&bytes)
        {
            let kind = match error.as_str() {
                "ForbiddenOperationException" => JoinRejection::InvalidToken,
                "InsufficientPrivilegesException" => JoinRejection::MultiplayerDisabled,
                "UserBannedException" => JoinRejection::Banned,
                _ => JoinRejection::Other(error),
            };
            return Err(MicrosoftError::JoinRejected {
                kind,
                message: error_message.unwrap_or_default(),
            });
        }
        Err(status_error(stage, status, &bytes))
    }
}

/// `now + expires_in`, truncated to whole seconds (the precision the cache stores).
fn expires_at(expires_in_secs: u64) -> SystemTime {
    let now_secs = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    SystemTime::UNIX_EPOCH
        .checked_add(Duration::from_secs(
            now_secs.saturating_add(expires_in_secs),
        ))
        .unwrap_or_else(SystemTime::now)
}

/// An ISO-8601 instant as `Instant.parse` reads it: `YYYY-MM-DDTHH:MM:SS[.fraction](Z|±HH:MM)`.
fn parse_instant(s: &str) -> Option<SystemTime> {
    let s = s.trim();
    let b = s.as_bytes();
    if b.len() < 20
        || b[4] != b'-'
        || b[7] != b'-'
        || b[10] != b'T'
        || b[13] != b':'
        || b[16] != b':'
    {
        return None;
    }
    let num = |r: std::ops::Range<usize>| s.get(r)?.parse::<i64>().ok();
    let (y, mo, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let (h, mi, sec) = (num(11..13)?, num(14..16)?, num(17..19)?);
    let mut rest = &s[19..];
    let mut nanos = 0i64;
    if let Some(f) = rest.strip_prefix('.') {
        let digits = f.bytes().take_while(u8::is_ascii_digit).count();
        if digits == 0 || digits > 9 {
            return None;
        }
        nanos = f[..digits].parse::<i64>().ok()? * 10i64.pow(9 - digits as u32);
        rest = &f[digits..];
    }
    let offset = match rest {
        "Z" | "z" => 0,
        o if o.len() == 6 && (o.starts_with('+') || o.starts_with('-')) && &o[3..4] == ":" => {
            let v = o[1..3].parse::<i64>().ok()? * 3600 + o[4..6].parse::<i64>().ok()? * 60;
            if o.starts_with('-') { -v } else { v }
        }
        _ => return None,
    };
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || sec > 60 {
        return None;
    }
    // Days from the civil date (Howard Hinnant's algorithm).
    let y2 = if mo <= 2 { y - 1 } else { y };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let mp = (mo + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let secs = days * 86_400 + h * 3600 + mi * 60 + sec - offset;
    let base = SystemTime::UNIX_EPOCH;
    if secs >= 0 {
        base.checked_add(Duration::new(secs as u64, nanos as u32))
    } else {
        base.checked_sub(Duration::from_secs(secs.unsigned_abs()))?
            .checked_add(Duration::from_nanos(nanos as u64))
    }
}

fn rate_limited(resp: &Response, stage: Stage) -> MicrosoftError {
    let retry_after = resp
        .headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<u64>().ok())
        .map(Duration::from_secs);
    MicrosoftError::RateLimited { stage, retry_after }
}

fn status_error(stage: Stage, status: StatusCode, body: &[u8]) -> MicrosoftError {
    let text = String::from_utf8_lossy(body);
    let mut end = text.len().min(MAX_ERROR_BODY);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    MicrosoftError::Status {
        stage,
        status: status.as_u16(),
        body: text[..end].to_owned(),
    }
}

/// Maps 429 and other non-success statuses to errors.
async fn check_status(resp: Response, stage: Stage) -> Result<Response, MicrosoftError> {
    let status = resp.status();
    if status.is_success() {
        return Ok(resp);
    }
    if status == StatusCode::TOO_MANY_REQUESTS {
        return Err(rate_limited(&resp, stage));
    }
    let body = read_body(resp, stage).await?;
    Err(status_error(stage, status, &body))
}

/// Reads the body, refusing anything larger than [`MAX_BODY`].
async fn read_body(mut resp: Response, stage: Stage) -> Result<Vec<u8>, MicrosoftError> {
    let too_large = || MicrosoftError::InvalidResponse {
        stage,
        reason: format!("body larger than {MAX_BODY} bytes"),
    };
    if resp
        .content_length()
        .is_some_and(|len| len > MAX_BODY as u64)
    {
        return Err(too_large());
    }
    let mut body = Vec::new();
    while let Some(chunk) = resp.chunk().await? {
        if body.len() + chunk.len() > MAX_BODY {
            return Err(too_large());
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

fn parse_json<T: DeserializeOwned>(body: &[u8], stage: Stage) -> Result<T, MicrosoftError> {
    serde_json::from_slice(body).map_err(|e| MicrosoftError::InvalidResponse {
        stage,
        reason: e.to_string(),
    })
}

async fn read_json<T: DeserializeOwned>(resp: Response, stage: Stage) -> Result<T, MicrosoftError> {
    let body = read_body(resp, stage).await?;
    parse_json(&body, stage)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xbox_error_codes() {
        assert_eq!(XboxError::from_code(2148916233), XboxError::NoXboxAccount);
        assert_eq!(
            XboxError::from_code(2148916235),
            XboxError::CountryUnavailable
        );
        assert_eq!(
            XboxError::from_code(2148916236),
            XboxError::AdultVerificationRequired
        );
        assert_eq!(
            XboxError::from_code(2148916237),
            XboxError::AdultVerificationRequired
        );
        assert_eq!(XboxError::from_code(2148916238), XboxError::ChildAccount);
        assert_eq!(XboxError::from_code(1), XboxError::Other(1));
    }

    #[test]
    fn rps_ticket_by_flavor() {
        assert_eq!(OAuthFlavor::Live.rps_ticket("abc"), "abc");
        assert_eq!(OAuthFlavor::AzureAd.rps_ticket("abc"), "d=abc");
    }

    #[test]
    fn error_body_is_truncated_on_char_boundary() {
        let body = "\u{e9}".repeat(MAX_ERROR_BODY);
        let MicrosoftError::Status { body, .. } =
            status_error(Stage::Profile, StatusCode::BAD_GATEWAY, body.as_bytes())
        else {
            panic!("wrong variant");
        };
        assert!(body.len() <= MAX_ERROR_BODY);
        assert!(body.chars().all(|c| c == '\u{e9}'));
    }

    #[test]
    fn session_serde_round_trip() {
        let session = MinecraftSession {
            access_token: "tok".into(),
            expires_at: SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000),
            profile: MinecraftProfile {
                id: Uuid::nil(),
                name: "Steve".into(),
            },
        };
        let json = serde_json::to_string(&session).unwrap();
        assert!(json.contains("1700000000"), "{json}");
        let back: MinecraftSession = serde_json::from_str(&json).unwrap();
        assert_eq!(back, session);
        assert!(!session.is_valid_for(Duration::ZERO));
    }
}
