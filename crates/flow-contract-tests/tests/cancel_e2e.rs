//! Review C4 end to end: the real flow-remote client cancels a commit that
//! the real in-process flow-server is still decoding. Client side: `cancel()`
//! sends `cancel` and wakes the waiting `commit`. Server side: the in-flight
//! core is cancelled and flow-core's abort flag stops the decode, so the
//! single worker is free long before the job would have ended.

use std::sync::Arc;
use std::time::{Duration, Instant};

use flow_contract_tests::*;
use flow_core::config::SessionConfig;
use flow_remote::RemoteBackend;
use flow_router::Backend;

const COMMIT: Duration = Duration::from_secs(4);

#[test]
fn client_cancel_during_commit_frees_the_server_worker() {
    let srv = spawn_slow_mock_server("slow words", COMMIT);
    let b = Arc::new(RemoteBackend::new(server_config(srv.addr(), TOKEN), "test"));
    b.start_session(&SessionConfig::default()).unwrap();
    for f in speech(500).chunks(FRAME) {
        assert!(b.push_audio(f));
    }
    let b2 = b.clone();
    let h = std::thread::spawn(move || {
        let t = Instant::now();
        let r = b2.commit(Instant::now() + Duration::from_secs(30));
        (r, t.elapsed())
    });
    // Wait until the server is decoding.
    let t0 = Instant::now();
    while srv.busy() == 0 {
        assert!(t0.elapsed() < Duration::from_secs(3), "commit never leased");
        std::thread::sleep(Duration::from_millis(10));
    }
    std::thread::sleep(Duration::from_millis(100));
    b.cancel();
    let (r, took) = h.join().unwrap();
    assert!(took < COMMIT / 2, "commit kept waiting {took:?}");
    assert!(r.is_err(), "a cancelled commit has no result");
    let t1 = Instant::now();
    while srv.busy() > 0 {
        assert!(
            t1.elapsed() < COMMIT / 3,
            "server worker still busy with a cancelled job"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(srv.cancels() >= 1, "server saw the cancel");
}
