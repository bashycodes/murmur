// Shared by build.rs (via include!) and lib.rs, so the strings the build
// bakes in and the tested function are the same code.

/// `(build, full)`: `("abc1234-dirty", "0.2.0+abc1234.dirty")`; an empty
/// sha (no git, no MURMUR_GIT_SHA) gives `("unknown", "0.2.0+unknown")`.
pub fn describe(version: &str, sha: &str, dirty: bool) -> (String, String) {
    if sha.is_empty() {
        ("unknown".to_string(), format!("{version}+unknown"))
    } else if dirty {
        (format!("{sha}-dirty"), format!("{version}+{sha}.dirty"))
    } else {
        (sha.to_string(), format!("{version}+{sha}"))
    }
}
