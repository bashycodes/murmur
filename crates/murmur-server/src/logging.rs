//! Log setup with a privacy cap (S9).
//!
//! `RUST_LOG` (or `[log] level`) chooses what is printed, but no setting can
//! make a dependency print payloads: tungstenite logs every received text
//! frame at trace (dictionary, context), hyper/axum can dump requests
//! (the bearer token). Events from any target outside `murmur_server` are
//! capped at INFO, whatever the filter says. The only way to get text into
//! the log is `log_text = true`, which makes murmur-server itself print the
//! result at debug.

use tracing::{Level, Metadata};
use tracing_subscriber::filter::{filter_fn, EnvFilter};
use tracing_subscriber::prelude::*;

/// Most verbose level a dependency may log at.
pub const DEPENDENCY_MAX_LEVEL: Level = Level::INFO;

/// True when an event or span with this metadata may be logged at all.
pub fn allowed(meta: &Metadata<'_>) -> bool {
    let t = meta.target();
    let ours = t == "murmur_server" || t.starts_with("murmur_server::");
    ours || *meta.level() <= DEPENDENCY_MAX_LEVEL
}

/// Install the global subscriber: `RUST_LOG` if set, else `level`, then
/// the dependency cap. Logs go to stderr without colour.
pub fn init(level: &str) {
    let env = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(level));
    let layer = tracing_subscriber::fmt::layer()
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .with_filter(env)
        .with_filter(filter_fn(allowed));
    tracing_subscriber::registry().with(layer).init();
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracing::callsite::Identifier;
    use tracing::field::FieldSet;
    use tracing::metadata::Kind;

    struct Cs;
    impl tracing::callsite::Callsite for Cs {
        fn set_interest(&self, _: tracing::subscriber::Interest) {}
        fn metadata(&self) -> &Metadata<'_> {
            unreachable!()
        }
    }
    static CS: Cs = Cs;

    fn meta(target: &'static str, level: Level) -> Metadata<'static> {
        Metadata::new(
            "e",
            target,
            level,
            None,
            None,
            None,
            FieldSet::new(&[], Identifier(&CS)),
            Kind::EVENT,
        )
    }

    #[test]
    fn dependencies_are_capped_at_info() {
        for t in [
            "tungstenite::protocol",
            "tokio_tungstenite",
            "hyper::proto",
            "axum::rejection",
            "murmur_core",
            "murmur_serverx",
        ] {
            assert!(!allowed(&meta(t, Level::TRACE)), "{t}");
            assert!(!allowed(&meta(t, Level::DEBUG)), "{t}");
            assert!(allowed(&meta(t, Level::INFO)), "{t}");
            assert!(allowed(&meta(t, Level::WARN)), "{t}");
        }
    }

    #[test]
    fn murmur_server_keeps_its_own_levels() {
        assert!(allowed(&meta("murmur_server", Level::TRACE)));
        assert!(allowed(&meta("murmur_server::ws", Level::DEBUG)));
    }
}
