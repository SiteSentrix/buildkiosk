//! The web interface (layer 6): server-rendered HTML pages with no JavaScript.
//!
//! Pages read the company state (through [`buildkiosk_engine::view`] helpers, so they follow the
//! same visibility rules as the queries) and act only through commands: every button is a plain
//! form that posts to `/do/{command}`. This crate renders; the server crate owns sessions, CSRF
//! checks and I/O.
#![forbid(unsafe_code)]

pub mod icons;
pub mod layout;
pub mod pages;
pub mod style;
pub mod submittals;
pub mod theme;

pub use layout::{Flash, Req};
