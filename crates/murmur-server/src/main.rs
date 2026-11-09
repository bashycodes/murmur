//! `murmur-server [--config PATH] [--token-file PATH] [--insecure-bind-any]`
//! `murmur-server gen-token --out PATH`
//! `murmur-server check-config [--config PATH]`
//! `murmur-server keys add <name> [--admin] | list | revoke <name|id> [--force]` (D38)
//! `murmur-server library list` (D40)

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Instant;

use murmur_server::{config, Config, Pool};

const DEFAULT_CONFIG: &str = "/etc/murmur-server/config.toml";
const SYSTEM_STATE_DIR: &str = "/var/lib/murmur-server";

fn usage() -> ExitCode {
    eprintln!(
        "usage:\n  murmur-server [--config PATH] [--token-file PATH] [--insecure-bind-any]\n  murmur-server gen-token --out PATH\n  murmur-server check-config [--config PATH] [--token-file PATH]\n  murmur-server keys add <name> [--admin] [--config PATH] [--state-dir DIR]\n  murmur-server keys list [--config PATH] [--state-dir DIR]\n  murmur-server keys revoke <name|id> [--force] [--config PATH] [--state-dir DIR]\n  murmur-server library list [--config PATH] [--state-dir DIR]"
    );
    ExitCode::from(2)
}

struct Args {
    cmd: String,
    config: PathBuf,
    token_file: Option<PathBuf>,
    out: Option<PathBuf>,
    insecure_bind_any: bool,
    state_dir: Option<PathBuf>,
    admin: bool,
    /// `keys revoke --force`: allow revoking the last admin key.
    force: bool,
    /// `keys` / `library` arguments (`add NAME`, `list`, `revoke NAME|ID`).
    rest: Vec<String>,
}

fn parse() -> Option<Args> {
    let mut a = Args {
        cmd: "serve".into(),
        config: DEFAULT_CONFIG.into(),
        token_file: None,
        out: None,
        insecure_bind_any: false,
        state_dir: None,
        admin: false,
        force: false,
        rest: Vec::new(),
    };
    let mut it = std::env::args().skip(1);
    while let Some(x) = it.next() {
        match x.as_str() {
            "--config" => a.config = it.next()?.into(),
            "--token-file" => a.token_file = Some(it.next()?.into()),
            "--out" => a.out = Some(it.next()?.into()),
            "--insecure-bind-any" => a.insecure_bind_any = true,
            "--state-dir" => a.state_dir = Some(it.next()?.into()),
            "--admin" => a.admin = true,
            "--force" => a.force = true,
            "gen-token" | "check-config" | "serve" | "keys" | "library" if a.cmd == "serve" => {
                a.cmd = x
            }
            _ if matches!(a.cmd.as_str(), "keys" | "library") && !x.starts_with("--") => {
                a.rest.push(x)
            }
            _ => return None,
        }
    }
    Some(a)
}

fn load_unchecked(a: &Args) -> Result<Config, String> {
    let mut c = Config::load(&a.config)?;
    if a.token_file.is_some() {
        c.token_file = a.token_file.clone();
    }
    if a.state_dir.is_some() {
        c.state_dir = a.state_dir.clone();
    }
    // A system install (config under /etc) keeps its state in the unit's
    // StateDirectory; the CLI, run by root outside the unit, has no
    // $STATE_DIRECTORY, so default to the same place (D38).
    if c.state_dir.is_none()
        && std::env::var_os("STATE_DIRECTORY").is_none()
        && a.config.starts_with("/etc")
    {
        c.state_dir = Some(SYSTEM_STATE_DIR.into());
    }
    c.insecure_bind_any = a.insecure_bind_any;
    Ok(c)
}

fn load(a: &Args) -> Result<Config, String> {
    let c = load_unchecked(a)?;
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
                for (lang, m) in &c.model.languages {
                    println!("language {lang}: asr={:?} {}", m.asr_backend, m.asr_path);
                }
                if let Some(d) = c.keys_dir().filter(|_| c.auth == config::AuthMode::Token) {
                    println!("keys: {}", d.join(murmur_server::keys::KEYS_FILE).display());
                }
                if c.debug.capture {
                    println!(
                        "debug capture: on, {} (newest {} files, {} h)",
                        c.debug.resolve_dir().expect("validated").display(),
                        c.debug.max_files,
                        c.debug.max_age_h
                    );
                }
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("config error: {e}");
                ExitCode::FAILURE
            }
        },
        "keys" => keys(&a),
        "library" => library(&a),
        _ => serve(&a),
    }
}

/// `murmur-server keys …` (D38). Works on the same files as the running
/// server, which picks the change up on the next connection.
fn keys(a: &Args) -> ExitCode {
    let run = || -> Result<(), String> {
        let c = load_unchecked(a)?;
        if c.auth == config::AuthMode::None {
            return Err("auth = \"none\": this server does not use keys".into());
        }
        let dir = c
            .keys_dir()
            .ok_or("no state directory for the keys file (pass --state-dir)")?;
        // Imports token_file as `owner` first, exactly as the server would.
        let store = murmur_server::keys::KeyStore::open(&dir, c.resolve_token()?.as_deref())?;
        match a.rest.iter().map(String::as_str).collect::<Vec<_>>()[..] {
            ["add", name] => {
                let (k, secret) = store.add(name, a.admin)?;
                eprintln!(
                    "added key {:?} (id {}{}). Its secret, shown only now; paste it into the app as the token:",
                    k.name,
                    k.id,
                    if k.admin { ", admin" } else { "" }
                );
                println!("{secret}");
            }
            ["list"] => print!("{}", murmur_server::keys::table(&store.list())),
            ["revoke", who] => {
                let k = store.revoke(who, a.force).map_err(|e| match e {
                    murmur_server::keys::KeyError::LastAdmin(m) => {
                        format!("{m} (or pass --force)")
                    }
                    e => e.to_string(),
                })?;
                eprintln!(
                    "revoked key {:?} (id {}): new connections with it get 401; open sessions finish",
                    k.name, k.id
                );
            }
            _ => {
                return Err(
                    "expected `keys add <name> [--admin]`, `keys list` or `keys revoke <name|id>`"
                        .into(),
                )
            }
        }
        Ok(())
    };
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("keys: {e}");
            ExitCode::FAILURE
        }
    }
}

/// `murmur-server library list` (D40): scans the library like the server does
/// (sharing its hash cache) and prints what is served and what is skipped.
fn library(a: &Args) -> ExitCode {
    let run = || -> Result<(), String> {
        if a.rest != ["list"] {
            return Err("expected `library list`".into());
        }
        let c = load_unchecked(a)?;
        let lib = murmur_server::library::Library::from_config(&c)
            .ok_or("no library folder: set [library] dir or state_dir (or pass --state-dir)")?;
        let idx = lib.rescan();
        print!("{}", murmur_server::library::table(lib.dir(), &idx));
        Ok(())
    };
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("library: {e}");
            ExitCode::FAILURE
        }
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
    murmur_server::logging::init(&cfg.log.level);

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
        let languages: Vec<String> = pool
            .languages
            .iter()
            .map(|(k, (b, _))| format!("{k}={b}"))
            .collect();
        let h = match murmur_server::serve_with_pool(cfg, pool, listeners).await {
            Ok(h) => h,
            Err(e) => {
                tracing::error!("{e}");
                return ExitCode::FAILURE;
            }
        };
        let addrs: Vec<String> = h.addrs.iter().map(|a| a.to_string()).collect();
        tracing::info!(
            version = murmur_build_info::FULL,
            workers,
            asr = %asr,
            languages = %languages.join(","),
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
