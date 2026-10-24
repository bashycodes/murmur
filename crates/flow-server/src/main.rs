//! `flow-server [--config PATH] [--token-file PATH] [--insecure-bind-any]`
//! `flow-server gen-token --out PATH`
//! `flow-server check-config [--config PATH]`

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Instant;

use flow_server::{config, Config, Pool};

const DEFAULT_CONFIG: &str = "/etc/flow-server/config.toml";

fn usage() -> ExitCode {
    eprintln!(
        "usage:\n  flow-server [--config PATH] [--token-file PATH] [--insecure-bind-any]\n  flow-server gen-token --out PATH\n  flow-server check-config [--config PATH] [--token-file PATH]"
    );
    ExitCode::from(2)
}

struct Args {
    cmd: String,
    config: PathBuf,
    token_file: Option<PathBuf>,
    out: Option<PathBuf>,
    insecure_bind_any: bool,
}

fn parse() -> Option<Args> {
    let mut a = Args {
        cmd: "serve".into(),
        config: DEFAULT_CONFIG.into(),
        token_file: None,
        out: None,
        insecure_bind_any: false,
    };
    let mut it = std::env::args().skip(1);
    while let Some(x) = it.next() {
        match x.as_str() {
            "--config" => a.config = it.next()?.into(),
            "--token-file" => a.token_file = Some(it.next()?.into()),
            "--out" => a.out = Some(it.next()?.into()),
            "--insecure-bind-any" => a.insecure_bind_any = true,
            "gen-token" | "check-config" | "serve" => a.cmd = x,
            _ => return None,
        }
    }
    Some(a)
}

fn load(a: &Args) -> Result<Config, String> {
    let mut c = Config::load(&a.config)?;
    if a.token_file.is_some() {
        c.token_file = a.token_file.clone();
    }
    c.insecure_bind_any = a.insecure_bind_any;
    c.validate()?;
    Ok(c)
}

fn main() -> ExitCode {
    let Some(a) = parse() else { return usage() };
    match a.cmd.as_str() {
        "gen-token" => {
            let Some(out) = &a.out else { return usage() };
            match config::write_token(out) {
                Ok(_) => {
                    eprintln!("wrote a new token to {} (mode 0600)", out.display());
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("gen-token: {e}");
                    ExitCode::FAILURE
                }
            }
        }
        "check-config" => match load(&a) {
            Ok(c) => {
                println!(
                    "config ok: bind={:?} workers={} asr={:?}",
                    c.bind, c.workers, c.model.asr_backend
                );
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("config error: {e}");
                ExitCode::FAILURE
            }
        },
        _ => serve(&a),
    }
}

fn serve(a: &Args) -> ExitCode {
    let cfg = match load(a) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("config error: {e}");
            return ExitCode::FAILURE;
        }
    };
    flow_server::logging::init(&cfg.log.level);

    // Models load before any listener binds.
    let t0 = Instant::now();
    let pool = match Pool::load(&cfg.model, cfg.workers, cfg.max_queue) {
        Ok(p) => Arc::new(p),
        Err(e) => {
            tracing::error!(kind = e.kind(), "model load failed: {e}");
            return ExitCode::FAILURE;
        }
    };
    let load_ms = t0.elapsed().as_millis();
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .worker_threads(2)
        .build()
        .expect("tokio runtime");
    rt.block_on(async move {
        let mut listeners = Vec::new();
        for addr in cfg.bind_addrs().expect("validated") {
            match tokio::net::TcpListener::bind(addr).await {
                Ok(l) => listeners.push(l),
                Err(e) => {
                    tracing::error!("bind {addr}: {e}");
                    return ExitCode::FAILURE;
                }
            }
        }
        let workers = pool.workers();
        let asr = pool.asr_backend.clone();
        let h = match flow_server::serve_with_pool(cfg, pool, listeners).await {
            Ok(h) => h,
            Err(e) => {
                tracing::error!("{e}");
                return ExitCode::FAILURE;
            }
        };
        let addrs: Vec<String> = h.addrs.iter().map(|a| a.to_string()).collect();
        tracing::info!(
            version = flow_build_info::FULL,
            workers,
            asr = %asr,
            load_ms,
            listening = %addrs.join(","),
            "ready"
        );
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("SIGTERM handler");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = term.recv() => {}
        }
        tracing::info!("shutting down");
        h.kill();
        ExitCode::SUCCESS
    })
}
