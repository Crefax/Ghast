//! Errors and session outcomes.

use ghast_protocol::{DecodeError, EncodeError, FrameError, State};
use thiserror::Error;

/// How a session that reached the play state ended without an error.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum SessionEnd {
    /// The server kicked the player.
    Kicked {
        /// The reason as plain text.
        reason: String,
        /// The reason as the server sent it (chat JSON).
        json: String,
    },
    /// The server closed the connection without a reason.
    ServerClosed,
    /// The behaviour called [`crate::TickContext::disconnect`].
    Quit(String),
    /// The shutdown future given to [`crate::Client::run_until`] completed.
    Shutdown,
}

/// A session failed.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ClientError {
    /// Opening the connection (direct or through the proxy) failed.
    #[error(transparent)]
    Connect(#[from] ghast_auth::connect::ConnectError),
    /// The username is not valid for an offline login.
    #[error("invalid username: {0}")]
    Username(#[from] ghast_auth::offline::UsernameError),
    /// Socket error.
    #[error("connection error: {0}")]
    Io(#[from] std::io::Error),
    /// Framing / compression / encryption error (the stream is unusable).
    #[error("bad frame from the server: {0}")]
    Frame(#[source] FrameError),
    /// A packet the client needs could not be decoded.
    #[error("could not decode {state:?} packet 0x{id:02x}: {source}")]
    Decode {
        /// Connection state.
        state: State,
        /// Packet id.
        id: i32,
        /// What was wrong.
        #[source]
        source: DecodeError,
    },
    /// Chunk data from the server was malformed. Since 0.1.3 a session no longer ends with
    /// it: the column is dropped, the error logged and play goes on (like vanilla). Kept for
    /// compatibility.
    #[error("bad chunk data: {0}")]
    World(#[source] ghast_world::WorldError),
    /// A packet could not be encoded (a value the protocol cannot carry).
    #[error("could not encode a packet: {0}")]
    Encode(#[from] EncodeError),
    /// The server refused the login.
    #[error("login refused: {0}")]
    LoginRefused(String),
    /// The server closed the connection during login.
    #[error("the server closed the connection during login")]
    LoginClosed,
    /// The server is in online mode but the account is an offline one.
    #[error("the server requires online mode (a Microsoft account)")]
    OnlineModeRequired,
    /// The encryption handshake failed.
    #[error("encryption handshake: {0}")]
    Crypto(#[from] ghast_auth::crypto::CryptoError),
    /// The session server refused the join (online mode).
    #[cfg(feature = "microsoft")]
    #[error("session server: {0}")]
    Session(#[from] ghast_auth::microsoft::MicrosoftError),
    /// No data from the server for too long.
    #[error("timed out: {0}")]
    Timeout(&'static str),
    /// The version's cargo feature is off in this build of the library.
    #[error("Minecraft {0} is not compiled in (enable the ghast feature for it)")]
    VersionNotEnabled(crate::GameVersion),
}

impl From<FrameError> for ClientError {
    fn from(e: FrameError) -> Self {
        match e {
            FrameError::Io(io) => ClientError::Io(io),
            e => ClientError::Frame(e),
        }
    }
}
