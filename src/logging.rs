use tracing_subscriber::{EnvFilter, fmt, prelude::*};

/// Initialize tracing.
///
/// Precedence for level filter:
///   1. Explicit `override_filter` (from --log-level / NAVIO_SIGNER_LOG)
///   2. `RUST_LOG` env var
///   3. Compile-time default: `debug` in debug builds, `info` in release.
pub fn init(override_filter: Option<&str>) {
    let default_level = if cfg!(debug_assertions) {
        "debug"
    } else {
        "info"
    };

    let filter = match override_filter {
        Some(s) => EnvFilter::try_new(s).unwrap_or_else(|_| EnvFilter::new(default_level)),
        None => EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_level)),
    };

    let fmt_layer = fmt::layer()
        .with_target(false)
        .with_thread_ids(false)
        .with_level(true);

    tracing_subscriber::registry()
        .with(filter)
        .with(fmt_layer)
        .init();
}
