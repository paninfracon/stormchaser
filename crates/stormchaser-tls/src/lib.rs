//! TLS configuration and hot-reloading utilities for Stormchaser.
//!
//! Provides functions to build `rustls` client and server configurations,
//! including support for hot-reloading certificates when files change.

#[cfg(feature = "listener")]
pub mod listener;
pub mod reloader;
pub use reloader::*;

/// Reads an on/off setting such as `STORMCHASER_LISTEN_TLS` or `STORMCHASER_DB_SSL`.
///
/// Only `true` and `false` (in any case) are accepted. Anything else is refused rather
/// than read as off, so a typo cannot quietly turn TLS off.
pub fn parse_switch(name: &str, value: &str) -> anyhow::Result<bool> {
    if value.eq_ignore_ascii_case("true") {
        Ok(true)
    } else if value.eq_ignore_ascii_case("false") {
        Ok(false)
    } else {
        anyhow::bail!("{name} must be true or false, not {value:?}")
    }
}
