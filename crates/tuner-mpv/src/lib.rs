//! Control an external mpv process over its JSON IPC socket.
//!
//! tuner never decodes video itself. It spawns `mpv --idle --input-ipc-server`
//! and sends commands such as `loadfile` over a Unix socket. See
//! `docs/mpv-ipc.md` for the protocol subset in use.

mod ipc;
mod load;
mod process;

pub use ipc::Connection;
pub use load::{
    CONNECT_TIMEOUT, LoadProgress, LoadStage, NO_DATA_TIMEOUT, RESUME_NO_DATA_TIMEOUT, StreamEnd,
};
pub use process::{Mpv, MpvOptions, Player, socket_path};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("failed to start mpv ({program}): {source}")]
    Spawn {
        program: String,
        source: std::io::Error,
    },
    #[error("mpv exited before its IPC socket became available")]
    ExitedEarly,
    #[error("timed out waiting for mpv IPC socket at {0}")]
    SocketTimeout(std::path::PathBuf),
    #[error("mpv closed the IPC connection")]
    Closed,
    #[error("mpv rejected command: {0}")]
    Command(String),
    #[error("malformed reply from mpv: {0}")]
    Protocol(#[from] serde_json::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

impl Error {
    /// True when the mpv process or socket is gone and a respawn may help.
    pub fn is_disconnect(&self) -> bool {
        match self {
            Error::Closed => true,
            Error::Io(e) => matches!(
                e.kind(),
                std::io::ErrorKind::BrokenPipe
                    | std::io::ErrorKind::ConnectionReset
                    | std::io::ErrorKind::NotConnected
                    | std::io::ErrorKind::UnexpectedEof
            ),
            _ => false,
        }
    }
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
