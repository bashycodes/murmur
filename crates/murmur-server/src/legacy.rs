//! The server's names before the Murmur rename (D83), still honoured for one
//! transition period so a running deployment and the old apps keep working:
//!
//! * the old WebSocket subprotocol ([`murmur_protocol::OLD_SUBPROTOCOL`]),
//!   accepted next to the new one ([`note_subprotocol`] logs the first use);
//! * the old config, state and system directories (`…/flow-server`): read
//!   when the new one is missing ([`config_path`], [`state_dir`]), never moved
//!   or deleted;
//! * the old `FLOW_*` environment variables (`murmur_build_info::env`).
//!
//! Every fallback logs a notice that says what to rename.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

/// The server's directory name before D83 (`/etc/<name>`, `~/.config/<name>`,
/// `/var/lib/<name>`, `~/.local/state/<name>`).
pub const OLD_DIR: &str = "flow-server";
/// The server's directory name now.
pub const DIR: &str = "murmur-server";

/// `…/murmur-server/<rest>` → `…/flow-server/<rest>`: the same path with the
/// last `murmur-server` component spelled the old way; `None` without one.
pub fn old_spelling(p: &Path) -> Option<PathBuf> {
    let comps: Vec<_> = p.components().collect();
    let i = comps.iter().rposition(|c| c.as_os_str() == DIR)?;
    let mut out = PathBuf::new();
    for (j, c) in comps.iter().enumerate() {
        if j == i {
            out.push(OLD_DIR);
        } else {
            out.push(c.as_os_str());
        }
    }
    Some(out)
}

/// The config file to read: `path` itself, unless it is missing and the old
/// install's file (same path under `flow-server`) exists. Second value: the
/// notice to log when the old one is used.
pub fn config_path(path: &Path) -> (PathBuf, Option<String>) {
    config_path_with(path, &|p| p.is_file())
}

fn config_path_with(path: &Path, exists: &dyn Fn(&Path) -> bool) -> (PathBuf, Option<String>) {
    if exists(path) {
        return (path.to_path_buf(), None);
    }
    match old_spelling(path).filter(|o| exists(o)) {
        Some(old) => {
            let note = format!(
                "reading the config of the earlier install, {} ({} does not exist). \
                 Copy it there (and point the unit at it) to silence this (D83)",
                old.display(),
                path.display()
            );
            (old, Some(note))
        }
        None => (path.to_path_buf(), None),
    }
}

/// The state directory to use: `dir`, unless the old install's directory
/// holds the keys file and `dir` does not (systemd creates an empty
/// `StateDirectory=` for the new unit, so "exists" is not enough).
pub fn state_dir(dir: PathBuf) -> (PathBuf, Option<String>) {
    state_dir_with(dir, &|p| p.is_file())
}

fn state_dir_with(dir: PathBuf, exists: &dyn Fn(&Path) -> bool) -> (PathBuf, Option<String>) {
    let keys = crate::keys::KEYS_FILE;
    if exists(&dir.join(keys)) {
        return (dir, None);
    }
    match old_spelling(&dir).filter(|o| exists(&o.join(keys))) {
        Some(old) => {
            let note = format!(
                "using the keys of the earlier install in {} ({} has none). \
                 Copy {keys} and key-usage.json there to silence this (D83)",
                old.display(),
                dir.display()
            );
            (old, Some(note))
        }
        None => (dir, None),
    }
}

/// Print a fallback notice to stderr (the journal), once per text: the
/// config and state lookups run before logging is set up, and more than once.
pub fn notice(text: &str) {
    static SEEN: Mutex<BTreeSet<String>> = Mutex::new(BTreeSet::new());
    if SEEN
        .lock()
        .map(|mut s| s.insert(text.to_string()))
        .unwrap_or(false)
    {
        eprintln!("notice: {text}");
    }
}

static OLD_PROTO_NOTED: AtomicBool = AtomicBool::new(false);

/// Log, once per process, that a client asked only for the old subprotocol
/// (an app from before the rename). `offered` is its `Sec-WebSocket-Protocol`.
pub fn note_subprotocol(offered: Option<&str>, peer: std::net::SocketAddr) {
    if uses_old_subprotocol(offered) && !OLD_PROTO_NOTED.swap(true, Ordering::Relaxed) {
        tracing::info!(
            %peer,
            "client speaks the old subprotocol {}: an app from before the Murmur rename; still accepted for now (D83)",
            murmur_protocol::OLD_SUBPROTOCOL
        );
    }
}

/// The client offered the old subprotocol and not the new one.
pub fn uses_old_subprotocol(offered: Option<&str>) -> bool {
    let all: Vec<&str> = offered.unwrap_or("").split(',').map(str::trim).collect();
    all.contains(&murmur_protocol::OLD_SUBPROTOCOL) && !all.contains(&murmur_protocol::SUBPROTOCOL)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fs(files: &[&str]) -> impl Fn(&Path) -> bool {
        let set: BTreeSet<PathBuf> = files.iter().map(PathBuf::from).collect();
        move |p| set.contains(p)
    }

    #[test]
    fn old_spelling_swaps_the_last_server_dir() {
        assert_eq!(
            old_spelling(Path::new("/etc/murmur-server/config.toml")),
            Some(PathBuf::from("/etc/flow-server/config.toml"))
        );
        assert_eq!(
            old_spelling(Path::new("/home/u/.local/state/murmur-server")),
            Some(PathBuf::from("/home/u/.local/state/flow-server"))
        );
        assert_eq!(old_spelling(Path::new("/srv/config.toml")), None);
    }

    #[test]
    fn config_prefers_the_new_file_and_falls_back_once_to_the_old_one() {
        let new = Path::new("/etc/murmur-server/config.toml");
        let both = fs(&[
            "/etc/murmur-server/config.toml",
            "/etc/flow-server/config.toml",
        ]);
        assert_eq!(config_path_with(new, &both), (new.to_path_buf(), None));

        let old_only = fs(&["/etc/flow-server/config.toml"]);
        let (p, note) = config_path_with(new, &old_only);
        assert_eq!(p, PathBuf::from("/etc/flow-server/config.toml"));
        assert!(note.unwrap().contains("/etc/murmur-server/config.toml"));

        // Neither: the new path, so the error names it.
        assert_eq!(config_path_with(new, &fs(&[])), (new.to_path_buf(), None));
        // A config outside a server dir has no old spelling.
        let other = Path::new("/srv/my.toml");
        assert_eq!(
            config_path_with(other, &fs(&[])),
            (other.to_path_buf(), None)
        );
    }

    #[test]
    fn state_dir_follows_the_keys_file() {
        let new = PathBuf::from("/var/lib/murmur-server");
        // systemd made the new dir, the keys are still in the old one.
        let old_keys = fs(&["/var/lib/flow-server/keys.json"]);
        let (d, note) = state_dir_with(new.clone(), &old_keys);
        assert_eq!(d, PathBuf::from("/var/lib/flow-server"));
        assert!(note.is_some());
        // Keys in the new dir win.
        let both = fs(&[
            "/var/lib/murmur-server/keys.json",
            "/var/lib/flow-server/keys.json",
        ]);
        assert_eq!(state_dir_with(new.clone(), &both), (new.clone(), None));
        // A fresh install: the new dir.
        assert_eq!(state_dir_with(new.clone(), &fs(&[])), (new, None));
    }

    #[test]
    fn only_an_old_only_offer_counts_as_old() {
        assert!(uses_old_subprotocol(Some("flow.v1")));
        assert!(!uses_old_subprotocol(Some("murmur.v1, flow.v1")));
        assert!(!uses_old_subprotocol(Some("murmur.v1")));
        assert!(!uses_old_subprotocol(None));
    }
}
