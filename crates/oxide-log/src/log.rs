//! The [`Log`] facade: Laravel-style `Log::info(..)` and `Log::channel("audit").info(..)`.
//!
//! These are thin wrappers over `tracing`; use the `tracing` macros directly (re-exported as
//! `oxide_log::tracing`) when you want structured fields.

use std::fmt::Display;

/// Logs a message to the default channels.
#[derive(Debug, Clone, Copy)]
pub struct Log;

macro_rules! levels {
    ($($name:ident => $macro:ident),* $(,)?) => {
        impl Log {$(
            #[doc = concat!("Logs `message` at `", stringify!($name), "` level to the default channels.")]
            pub fn $name(message: impl Display) {
                tracing::$macro!("{message}");
            }
        )*}

        impl ChannelLog<'_> {$(
            #[doc = concat!("Logs `message` at `", stringify!($name), "` level to this channel.")]
            pub fn $name(&self, message: impl Display) {
                tracing::$macro!(channel = self.name, "{message}");
            }
        )*}
    };
}

impl Log {
    /// A handle for one configured channel: `Log::channel("audit").info("exported report")`.
    /// A name that isn't configured falls back to the default channels.
    pub fn channel(name: &str) -> ChannelLog<'_> {
        ChannelLog { name }
    }
}

/// A named channel, from [`Log::channel`].
#[derive(Debug, Clone, Copy)]
pub struct ChannelLog<'a> {
    name: &'a str,
}

levels! {
    error => error,
    warn => warn,
    info => info,
    debug => debug,
    trace => trace,
}
