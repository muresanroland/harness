//! The version the binary reports: `v1.0.0` for a release build, `v1.0.0-dev`
//! otherwise. Cargo.toml is the source of truth; the release workflow refuses
//! a tag that differs from it.

/// The version string, for `harness --version` and the Shell header. A dev
/// build carries `-dev` so it never looks like a release, and never updates.
pub fn version() -> String {
    let dev = option_env!("HARNESS_RELEASE").map_or("-dev", |_| "");
    format!("v{}{dev}", env!("CARGO_PKG_VERSION"))
}
