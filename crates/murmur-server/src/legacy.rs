//! The server's names before the Murmur rename (D83) that it still answers
//! to for one transition period, so an app from before the rename keeps
//! working against the renamed server: the old WebSocket subprotocol
//! ([`murmur_protocol::OLD_SUBPROTOCOL`]), accepted next to the new one
//! ([`note_subprotocol`] logs the first use), and the old `FLOW_*`
//! environment variables (`murmur_build_info::env`).

use std::sync::atomic::{AtomicBool, Ordering};

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

    #[test]
    fn only_an_old_only_offer_counts_as_old() {
        assert!(uses_old_subprotocol(Some("flow.v1")));
        assert!(!uses_old_subprotocol(Some("murmur.v1, flow.v1")));
        assert!(!uses_old_subprotocol(Some("murmur.v1")));
        assert!(!uses_old_subprotocol(None));
    }
}
