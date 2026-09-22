//! The version the binary reports: `v1.0.0` for a release build, `v1.0.0-dev`
//! otherwise. Cargo.toml is the source of truth; the release workflow refuses
//! a tag that differs from it.

/// The version string, for `harness --version` and the Shell header.
pub fn version() -> String {
    label(env!("CARGO_PKG_VERSION"), option_env!("HARNESS_RELEASE"))
}

/// Maps Cargo's version and the compile-time HARNESS_RELEASE to the string.
/// A dev build carries `-dev` so it never looks like a release.
fn label(pkg_version: &str, release: Option<&str>) -> String {
    match release {
        Some(_) => format!("v{pkg_version}"),
        None => format!("v{pkg_version}-dev"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_build_is_bare_version() {
        assert_eq!(label("1.0.0", Some("1")), "v1.0.0");
    }

    #[test]
    fn dev_build_carries_dev_marker() {
        assert_eq!(label("1.0.0", None), "v1.0.0-dev");
    }

    #[test]
    fn version_starts_with_cargo_version() {
        assert!(version().starts_with("v1.0.0"), "version() = {}", version());
    }
}
