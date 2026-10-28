//! Application services shared by tuner front-ends (the TUI today, a GUI
//! later): the config file, loading channels from sources over HTTP,
//! stream-limit enforcement and the play/stop flow. No terminal or GUI code.

mod batches;
pub mod config;
pub mod guard;
pub mod playback;
pub mod source;
pub mod update;

pub use batches::Flow;
pub use config::{Config, Resolved};
pub use guard::{Begin, LimitMode, StreamGate, StreamGuard};
pub use playback::{Controller, Playback, Request, Tick, status_line};
pub use source::{Catalog, Loaded, Opened, Source, Usage};
