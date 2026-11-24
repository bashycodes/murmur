//! The `result` and `cleaned` log lines say how much context arrived and
//! was used (`ctx_screen_words`, `hotwords`, `dict_terms_in_prompt`) as
//! numbers only: the screen words and dictionary terms themselves never
//! reach the log.

mod common;

use std::net::SocketAddr;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use common::*;
use murmur_core::asr::{AbortFlag, AsrEngine, AsrError, AsrOutput, AsrRequest};
use murmur_core::cleanup::{CleanError, CleanPrompt, CleanRequest, CleanUsage, Cleaner};
use murmur_core::config::{CleanupLevel, FieldContext, SessionConfig};
use murmur_core::dictionary::{DictEntry, Dictionary};
use murmur_core::MurmurCore;
use murmur_protocol::{ClientMsg, ServerMsg};
use murmur_server::{Pool, ServerHandle};
use tokio_tungstenite::MaybeTlsStream;

/// Invented words: none of them may appear anywhere in the log.
const SCREEN: [&str; 3] = ["Zorbletash", "Quindravel", "Plumtackery"];
const TERM: &str = "Vexmorrin";
/// What the test ASR hears: near `TERM`, so the cleanup prompt names it.
const HEARD: &str = "please ask vexmorin about it";

/// Everything the server logs in this test binary.
fn logs() -> &'static Arc<Mutex<Vec<u8>>> {
    static LOGS: OnceLock<Arc<Mutex<Vec<u8>>>> = OnceLock::new();
    LOGS.get_or_init(|| {
        #[derive(Clone)]
        struct Buf(Arc<Mutex<Vec<u8>>>);
        impl std::io::Write for Buf {
            fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(b);
                Ok(b.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let buf = Arc::new(Mutex::new(Vec::new()));
        let w = Buf(buf.clone());
        tracing_subscriber::fmt()
            .with_ansi(false)
            .with_max_level(tracing::Level::DEBUG)
            .with_writer(move || w.clone())
            .init();
        buf
    })
}

/// A fixed transcript; takes every distinct dictionary and screen phrase
/// as a hotword, like a sherpa engine under its cap.
struct HotwordAsr;

impl AsrEngine for HotwordAsr {
    fn transcribe(&self, _: &[f32], _: &AsrRequest) -> Result<AsrOutput, AsrError> {
        Ok(AsrOutput {
            text: HEARD.into(),
            ..Default::default()
        })
    }

    fn hotwords_used(&self, req: &AsrRequest) -> usize {
        req.dictionary_terms.len() + req.screen_terms.len()
    }
}

/// Echoes the transcript upper-cased.
struct Upper;

impl Cleaner for Upper {
    fn clean(&self, p: &CleanPrompt, _: Instant) -> Result<String, CleanError> {
        Ok(p.transcript.to_uppercase())
    }
}

/// Refuses every request up front: no LLM call runs.
struct Refuse;

impl Cleaner for Refuse {
    fn clean(&self, _: &CleanPrompt, _: Instant) -> Result<String, CleanError> {
        Err(CleanError::Unavailable("down".into()))
    }

    fn clean_metered(
        &self,
        p: &CleanPrompt,
        d: Instant,
        _: &AbortFlag,
    ) -> (Result<String, CleanError>, CleanUsage) {
        (self.clean(p, d), CleanUsage::default())
    }
}

async fn server(cleaner: impl Cleaner + 'static) -> ServerHandle {
    logs();
    let cfg = mock_cfg("unused");
    let core = MurmurCore::from_engines(
        Arc::new(HotwordAsr),
        Some(Arc::new(cleaner) as Arc<dyn Cleaner>),
    );
    let pool = Pool::from_cores(vec![Arc::new(core)], &cfg.model, cfg.max_queue);
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    murmur_server::serve_with_pool(cfg, Arc::new(pool), vec![l])
        .await
        .unwrap()
}

fn dictionary() -> Dictionary {
    Dictionary {
        entries: vec![DictEntry::word(TERM)],
    }
}

fn screen_ctx() -> FieldContext {
    FieldContext {
        screen_words: SCREEN.iter().map(|w| w.to_string()).collect(),
        ..Default::default()
    }
}

fn local_addr(ws: &Ws) -> SocketAddr {
    match ws.get_ref() {
        MaybeTlsStream::Plain(t) => t.local_addr().unwrap(),
        _ => unreachable!("tests connect over plain TCP"),
    }
}

/// The one line logged with `msg` from `peer` (waits up to 3 s for it),
/// after checking that no invented word is anywhere in the log.
async fn line(msg: &str, peer: SocketAddr) -> String {
    let needle = format!("peer={peer} ");
    let find = || -> Vec<String> {
        String::from_utf8_lossy(&logs().lock().unwrap())
            .lines()
            .filter(|l| l.contains(msg) && l.contains(&needle))
            .map(str::to_string)
            .collect()
    };
    let t0 = Instant::now();
    while find().is_empty() && t0.elapsed() < Duration::from_secs(3) {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let all = String::from_utf8_lossy(&logs().lock().unwrap()).to_lowercase();
    for w in SCREEN.iter().chain([&TERM, &"vexmorin"]) {
        assert!(
            !all.contains(&w.to_lowercase()),
            "{w} reached the log:\n{all}"
        );
    }
    let v = find();
    assert_eq!(v.len(), 1, "{v:?}");
    v.into_iter().next().unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn result_line_counts_screen_words_hotwords_and_prompt_terms() {
    let s = server(Upper).await;
    let mut ws = connect(s.addr(), Some(TOKEN)).await.unwrap();
    let peer = local_addr(&ws);
    hello(&mut ws).await;
    let config = SessionConfig {
        dictionary: dictionary(),
        llm_skip_clean: false,
        ..Default::default()
    };
    send(&mut ws, &ClientMsg::Start { sid: 1, config }).await;
    assert!(matches!(
        recv(&mut ws).await,
        Some(ServerMsg::Started { .. })
    ));
    send(
        &mut ws,
        &ClientMsg::Context {
            sid: 1,
            ctx: screen_ctx(),
        },
    )
    .await;
    send_audio(&mut ws, &speech(500)).await;
    send(&mut ws, &ClientMsg::Commit { sid: 1 }).await;
    match recv_final(&mut ws).await {
        Some(ServerMsg::Result { result, .. }) => {
            assert_eq!(
                result.status,
                murmur_core::CommitStatus::Formatted,
                "the LLM ran"
            )
        }
        other => panic!("expected a result, got {other:?}"),
    }
    let l = line(" result", peer).await;
    // 3 screen words; the dictionary term plus them as hotwords; the
    // term the transcript comes near plus the screen words in the prompt.
    assert!(
        l.contains("ctx_screen_words=3 hotwords=4 dict_terms_in_prompt=4"),
        "{l}"
    );
}

/// Sends a text-only cleanup of `HEARD` with the invented context; returns
/// whether it came back cleaned, and the `cleaned` log line.
async fn clean_with(cleaner: impl Cleaner + 'static) -> (bool, String) {
    let s = server(cleaner).await;
    let mut ws = connect(s.addr(), Some(TOKEN)).await.unwrap();
    let peer = local_addr(&ws);
    hello(&mut ws).await;
    let req = CleanRequest {
        text: HEARD.into(),
        level: CleanupLevel::Light,
        timeout_ms: 2000,
        ctx: screen_ctx(),
        dictionary: dictionary(),
        ..Default::default()
    };
    send(
        &mut ws,
        &ClientMsg::Clean {
            id: 5,
            req: Box::new(req),
        },
    )
    .await;
    let ok = match recv(&mut ws).await {
        Some(ServerMsg::Cleaned(c)) => c.text.is_some(),
        other => panic!("expected cleaned, got {other:?}"),
    };
    (ok, line(" cleaned", peer).await)
}

#[tokio::test(flavor = "multi_thread")]
async fn cleaned_line_counts_screen_words_and_prompt_terms() {
    let (ok, l) = clean_with(Upper).await;
    assert!(ok, "{l}");
    assert!(l.contains("task=\"cleanup\""), "{l}");
    // No ASR on a text-only cleanup: no hotwords.
    assert!(
        l.contains("ctx_screen_words=3 hotwords=0 dict_terms_in_prompt=4"),
        "{l}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn cleaned_line_counts_no_prompt_terms_when_no_llm_ran() {
    let (ok, l) = clean_with(Refuse).await;
    assert!(!ok, "{l}");
    assert!(
        l.contains("ctx_screen_words=3 hotwords=0 dict_terms_in_prompt=0"),
        "{l}"
    );
}
