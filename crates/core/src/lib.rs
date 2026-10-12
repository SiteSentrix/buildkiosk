//! Foundation types shared by every BuildKiosk crate (layer 0: no workspace dependencies).
//!
//! Everything here is pure, deterministic and builds for wasm32: typed ids, money and percentages
//! with checked arithmetic, civil dates, UTC timestamps, and the text checks applied to untrusted
//! input. Nothing here panics on any input.
#![forbid(unsafe_code)]

pub mod date;
pub mod id;
pub mod money;
pub mod text;

pub use date::{Date, Timestamp, Weekday};
pub use id::Id;
pub use money::{Money, MoneyError, Percent};
