//! Core domain for tuner: channel and series models, playlist and Xtream parsers and the
//! in-memory library.
//!
//! This crate is pure: it never touches the network, filesystem, mpv or the
//! terminal. Callers hand it strings and get data back.

pub mod channel;
pub mod library;
pub mod m3u;
pub mod series;
pub mod xtream;

pub use channel::Channel;
pub use library::{Entry, Library};
pub use series::Series;
