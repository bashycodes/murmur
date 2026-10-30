//! Bounds on client-controlled sizes (S8). A token holder or a buggy client
//! must not be able to make the server allocate without limit or pin a
//! worker for minutes. Out-of-range values are rejected with a non-fatal
//! `too_large`; `max_samples` and `partial_interval_ms` are clamped because
//! the protocol already defines what happens past them (truncation /
//! slower partials).

use flow_core::config::{FieldContext, SessionConfig, SAMPLE_RATE};

/// Extra ring room on top of the session cap: the router enlarges the ring
/// to replay buffered audio after a fallback (`REPLAY_HEADROOM`).
pub const RING_HEADROOM_SAMPLES: usize = SAMPLE_RATE * 30;
/// Longest LLM cleanup budget a client may ask for (contract tests use 60 s
/// for a deterministic CPU decode).
pub const MAX_LLM_TIMEOUT_MS: u64 = 120_000;
/// Partials are polled every 100 ms; asking for more is wasted decodes.
pub const MIN_PARTIAL_INTERVAL_MS: u32 = 100;
/// `before` / `selected` / `after` / `app_id` / `url_host`, each (bytes).
/// Shells send at most 500 characters of field text.
pub const MAX_CONTEXT_TEXT_BYTES: usize = 8 * 1024;
/// `screen_words` + `file_names` entries.
pub const MAX_CONTEXT_WORDS: usize = 1000;
/// One WebSocket message or frame (audio frames are at most 64 KiB; the
/// dictionary travels in `start`).
pub const MAX_WS_MESSAGE_BYTES: usize = flow_protocol::MAX_FRAME_BYTES * 2;

/// Validate and clamp a client's session config against the server's
/// sample cap (`max_session_s` × 16 kHz).
pub fn session(
    mut cfg: SessionConfig,
    max_session_samples: usize,
) -> Result<SessionConfig, String> {
    let ring_max = max_session_samples + RING_HEADROOM_SAMPLES;
    if cfg.ring_capacity_samples > ring_max {
        return Err(format!(
            "ring_capacity_samples over the server limit of {ring_max}"
        ));
    }
    if cfg.llm_timeout_ms > MAX_LLM_TIMEOUT_MS {
        return Err(format!(
            "llm_timeout_ms over the server limit of {MAX_LLM_TIMEOUT_MS}"
        ));
    }
    cfg.max_samples = cfg.max_samples.min(max_session_samples);
    if let Some(ms) = cfg.partial_interval_ms.as_mut() {
        *ms = (*ms).max(MIN_PARTIAL_INTERVAL_MS);
    }
    Ok(cfg)
}

/// Reject a context whose text or word lists exceed the server limits.
pub fn context(ctx: &FieldContext) -> Result<(), String> {
    let texts = [
        &ctx.before,
        &ctx.selected,
        &ctx.after,
        &ctx.app_id,
        &ctx.url_host,
    ];
    if texts.iter().any(|t| t.len() > MAX_CONTEXT_TEXT_BYTES) {
        return Err(format!("context text over {MAX_CONTEXT_TEXT_BYTES} bytes"));
    }
    let words = ctx.screen_words.len() + ctx.file_names.len();
    let word_bytes: usize = ctx
        .screen_words
        .iter()
        .chain(&ctx.file_names)
        .map(String::len)
        .sum();
    if words > MAX_CONTEXT_WORDS || word_bytes > MAX_CONTEXT_WORDS * 64 {
        return Err(format!("context over {MAX_CONTEXT_WORDS} words"));
    }
    Ok(())
}

/// Reject a text-only cleanup request (D36) over the server limits.
pub fn clean(req: &flow_core::cleanup::CleanRequest) -> Result<(), String> {
    if req.text.len() > flow_protocol::MAX_CLEAN_TEXT_BYTES {
        return Err(format!(
            "text over {} bytes",
            flow_protocol::MAX_CLEAN_TEXT_BYTES
        ));
    }
    if req.timeout_ms > MAX_LLM_TIMEOUT_MS {
        return Err(format!(
            "timeout_ms over the server limit of {MAX_LLM_TIMEOUT_MS}"
        ));
    }
    context(&req.ctx)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CAP: usize = 1200 * SAMPLE_RATE;

    #[test]
    fn defaults_pass_unchanged() {
        let d = SessionConfig::default();
        let got = session(d.clone(), CAP).unwrap();
        assert_eq!(got, d);
        assert!(context(&FieldContext::default()).is_ok());
    }

    #[test]
    fn ring_and_llm_budget_are_rejected_over_the_limit() {
        let c = SessionConfig {
            ring_capacity_samples: CAP + RING_HEADROOM_SAMPLES + 1,
            ..Default::default()
        };
        assert!(session(c, CAP).is_err());
        let c = SessionConfig {
            ring_capacity_samples: CAP + RING_HEADROOM_SAMPLES,
            ..Default::default()
        };
        assert!(session(c, CAP).is_ok());
        let c = SessionConfig {
            llm_timeout_ms: MAX_LLM_TIMEOUT_MS + 1,
            ..Default::default()
        };
        assert!(session(c, CAP).is_err());
    }

    #[test]
    fn max_samples_and_partial_interval_are_clamped() {
        let c = SessionConfig {
            max_samples: usize::MAX,
            partial_interval_ms: Some(0),
            ..Default::default()
        };
        let got = session(c, 1000).unwrap();
        assert_eq!(got.max_samples, 1000);
        assert_eq!(got.partial_interval_ms, Some(MIN_PARTIAL_INTERVAL_MS));
    }

    #[test]
    fn context_limits() {
        let mut c = FieldContext {
            after: "x".repeat(MAX_CONTEXT_TEXT_BYTES),
            ..Default::default()
        };
        assert!(context(&c).is_ok());
        c.after.push('x');
        assert!(context(&c).is_err());
        let c = FieldContext {
            file_names: vec!["f".into(); MAX_CONTEXT_WORDS + 1],
            ..Default::default()
        };
        assert!(context(&c).is_err());
        let c = FieldContext {
            screen_words: vec!["w".repeat(MAX_CONTEXT_TEXT_BYTES); 10],
            ..Default::default()
        };
        assert!(context(&c).is_err());
    }

    #[test]
    fn clean_limits() {
        let mut r = flow_core::cleanup::CleanRequest {
            text: "x".repeat(flow_protocol::MAX_CLEAN_TEXT_BYTES),
            timeout_ms: MAX_LLM_TIMEOUT_MS,
            ..Default::default()
        };
        assert!(clean(&r).is_ok());
        r.text.push('x');
        assert!(clean(&r).is_err());
        r.text.clear();
        r.timeout_ms += 1;
        assert!(clean(&r).is_err());
        r.timeout_ms = 0;
        r.ctx.before = "x".repeat(MAX_CONTEXT_TEXT_BYTES + 1);
        assert!(clean(&r).is_err());
    }
}
