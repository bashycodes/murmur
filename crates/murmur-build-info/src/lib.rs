//! What this binary is (D26, docs/versioning.md): the product version from
//! the repo's `VERSION` file and the git commit it was built from.
//!
//! * [`VERSION`] `0.2.0` — the product version (= every crate's version).
//! * [`BUILD`] `abc1234` / `abc1234-dirty` / `unknown`.
//! * [`FULL`] `0.2.0+abc1234.dirty` — SemVer with build metadata; what logs,
//!   the server's `hello_ok.server` and the shells' About rows show.
//!
//! The protocol version (`murmur_protocol::PROTO_*`) is separate on purpose.

include!("describe.rs");

pub mod env;

/// Product version, `MAJOR.MINOR.PATCH`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// Short commit (7 hex) or empty when git was unavailable.
pub const GIT_SHA: &str = env!("MURMUR_GIT_SHA");
/// Tracked files differed from `GIT_SHA` when this was built.
pub const GIT_DIRTY: bool = matches!(env!("MURMUR_GIT_DIRTY").as_bytes(), b"1");
/// `abc1234`, `abc1234-dirty` or `unknown`.
pub const BUILD: &str = env!("MURMUR_BUILD");
/// `0.2.0+abc1234` / `0.2.0+abc1234.dirty` / `0.2.0+unknown`.
pub const FULL: &str = env!("MURMUR_FULL");

#[cfg(test)]
mod tests {
    use super::*;

    fn vectors() -> serde_json::Value {
        let p = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tools/versioning/fixtures/version_vectors.json"
        );
        serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
    }

    #[test]
    fn describe_matches_the_shared_vectors() {
        let v = vectors();
        let cases = v["describe"].as_array().unwrap();
        assert!(cases.len() >= 3);
        for c in cases {
            let (build, full) = describe(
                c["version"].as_str().unwrap(),
                c["sha"].as_str().unwrap(),
                c["dirty"].as_bool().unwrap(),
            );
            assert_eq!(build, c["build"].as_str().unwrap(), "{c}");
            assert_eq!(full, c["full"].as_str().unwrap(), "{c}");
        }
    }

    #[test]
    fn baked_constants_agree() {
        assert_eq!(
            (BUILD.to_string(), FULL.to_string()),
            describe(VERSION, GIT_SHA, GIT_DIRTY)
        );
        let file = concat!(env!("CARGO_MANIFEST_DIR"), "/../../VERSION");
        assert_eq!(std::fs::read_to_string(file).unwrap().trim(), VERSION);
        assert!(
            GIT_SHA.is_empty()
                || (GIT_SHA.len() >= 7 && GIT_SHA.chars().all(|c| c.is_ascii_hexdigit())),
            "{GIT_SHA}"
        );
    }
}
