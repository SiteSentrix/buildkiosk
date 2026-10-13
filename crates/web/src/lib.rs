//! The web interface (layer 6): server-rendered HTML pages with no JavaScript.
//!
//! Pages read the company state (through [`buildkiosk_engine::view`] helpers, so they follow the
//! same visibility rules as the queries) and act only through commands: every button is a plain
//! form that posts to `/do/{command}`. This crate renders; the server crate owns sessions, CSRF
//! checks and I/O.
#![forbid(unsafe_code)]

pub mod correspondence;
pub mod daily_logs;
pub mod documents;
pub mod drawings;
pub mod finance;
pub mod forms;
pub mod icons;
pub mod inbox;
pub mod inspections;
pub mod invoicing;
pub mod layout;
pub mod linedocs;
pub mod meetings;
pub mod pages;
pub mod photos;
pub mod reports;
pub mod schedule;
pub mod specs;
pub mod style;
pub mod submittals;
pub mod theme;
pub mod timesheets;
pub mod transmittals;
pub mod workitems;

pub use layout::{Flash, Req};
