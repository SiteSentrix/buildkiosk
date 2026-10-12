//! Last line of defence against bugs: a panic in a command never takes the server down.
//!
//! Code must not panic (AGENTS.md › Never crash); fallible work returns `Result`. Should a bug panic
//! anyway, [`crate::Engine::execute`] catches it with [`catch_panic`], drops the half-edited copy of
//! the company state and reports an internal error. The HTTP server, the MCP server and the CLI
//! wrap their own entry points the same way.
//!
//! On wasm the panic strategy is `abort`, so this only helps native builds; the rule against
//! panicking code is what protects a browser build.

use std::any::Any;
use std::panic::{AssertUnwindSafe, catch_unwind};

/// Runs `f`, turning a panic into `Err(message)`. The default panic hook still prints the message
/// and location to stderr.
pub fn catch_panic<T>(f: impl FnOnce() -> T) -> Result<T, String> {
    catch_unwind(AssertUnwindSafe(f)).map_err(|payload| {
        let msg = panic_message(payload.as_ref());
        log::error!("recovered from a panic: {msg}");
        msg
    })
}

/// The message of a panic payload (`panic!("…")` carries a `&str` or a `String`).
pub fn panic_message(payload: &(dyn Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "unknown panic".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catch_panic_returns_value_or_message() {
        assert_eq!(catch_panic(|| 7), Ok(7));
        assert_eq!(catch_panic(|| -> i32 { panic!("boom {}", 1) }), Err("boom 1".to_string()));
        assert_eq!(catch_panic(|| -> i32 { std::panic::panic_any(5_u8) }), Err("unknown panic".to_string()));
    }
}
