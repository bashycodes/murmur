//! Air-gapped end-to-end client (tools/e2e/airgap/run.sh): drives a real,
//! separately started flow-server through the same client code the apps
//! use (flow-remote's AdminClient, LibraryClient, RemoteBackend).
//!
//!   airgap_e2e --port N --admin-token-file F --wav F --expect-text WORD
//!              --out DIR --expect ID/FILE=SHA256 [--expect …]
//!
//! Steps, each printed as `PASS <step>: …` or `FAIL <step>: …`:
//! admin whoami → create a user key → the user key lists the library and
//! downloads every expected file (verified by LibraryClient and hashed
//! again here) → non-admin is refused the admin API → one dictation with
//! the user key → the admin's key list shows the user's dictation and
//! download counts → revoke → the user key is refused on every route.
//! Exit 0 only when every step passed. Secrets are never printed.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use flow_core::config::SessionConfig;
use flow_protocol::admin::code;
use flow_remote::admin::AdminClient;
use flow_remote::library::{ErrorKind, LibraryClient};
use flow_remote::RemoteBackend;
use flow_router::{Backend, Failure, ServerConfig};
use sha2::{Digest, Sha256};

const UA: &str = "airgap-e2e";
const USER_KEY: &str = "e2e-phone";

struct Args {
    port: u16,
    admin_token: String,
    wav: PathBuf,
    expect_text: String,
    out: PathBuf,
    expect: Vec<(String, String, String)>,
}

fn args() -> Result<Args, String> {
    let mut it = std::env::args().skip(1);
    let (mut port, mut tok, mut wav, mut text, mut out) = (None, None, None, None, None);
    let mut expect = Vec::new();
    while let Some(a) = it.next() {
        let mut v = || it.next().ok_or(format!("{a} needs a value"));
        match a.as_str() {
            "--port" => port = Some(v()?.parse::<u16>().map_err(|e| e.to_string())?),
            "--admin-token-file" => {
                let p = v()?;
                tok = Some(
                    std::fs::read_to_string(&p)
                        .map_err(|e| format!("{p}: {e}"))?
                        .trim()
                        .to_string(),
                )
            }
            "--wav" => wav = Some(PathBuf::from(v()?)),
            "--expect-text" => text = Some(v()?.to_lowercase()),
            "--out" => out = Some(PathBuf::from(v()?)),
            "--expect" => {
                let s = v()?;
                let (path, sha) = s.split_once('=').ok_or("--expect ID/FILE=SHA")?;
                let (id, file) = path.split_once('/').ok_or("--expect ID/FILE=SHA")?;
                expect.push((id.into(), file.into(), sha.to_lowercase()));
            }
            _ => return Err(format!("unknown argument {a}")),
        }
    }
    Ok(Args {
        port: port.ok_or("--port")?,
        admin_token: tok.ok_or("--admin-token-file")?,
        wav: wav.ok_or("--wav")?,
        expect_text: text.ok_or("--expect-text")?,
        out: out.ok_or("--out")?,
        expect,
    })
}

fn cfg(port: u16, token: &str) -> ServerConfig {
    ServerConfig {
        host: "127.0.0.1".into(),
        port,
        token: token.into(),
        connect_timeout_ms: 3000,
        ..Default::default()
    }
}

fn sha_file(p: &std::path::Path) -> std::io::Result<String> {
    let mut h = Sha256::new();
    let mut f = std::fs::File::open(p)?;
    std::io::copy(&mut f, &mut h)?;
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

struct Run {
    failed: bool,
}

impl Run {
    fn step(&mut self, name: &str, r: Result<String, String>) -> bool {
        match r {
            Ok(m) => {
                println!("PASS {name}: {m}");
                true
            }
            Err(m) => {
                println!("FAIL {name}: {m}");
                self.failed = true;
                false
            }
        }
    }
}

fn main() -> std::process::ExitCode {
    let a = match args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("airgap_e2e: {e}");
            return std::process::ExitCode::from(2);
        }
    };
    let mut run = Run { failed: false };
    let admin = AdminClient::new(cfg(a.port, &a.admin_token), UA);

    // 1. The admin key (made with the CLI) is an admin over the API.
    if !run.step(
        "admin-whoami",
        match admin.whoami() {
            Ok(w) if w.admin => Ok(format!("key {:?} ({}) is admin", w.name, w.id)),
            Ok(w) => Err(format!("key {:?} is not admin", w.name)),
            Err(e) => Err(format!("{}: {}", e.code, e.message)),
        },
    ) {
        return std::process::ExitCode::FAILURE;
    }

    // 2. The admin API creates a user key; its secret stays in memory.
    let created = match admin.create_key(USER_KEY, false) {
        Ok(c) => c,
        Err(e) => {
            run.step("create-user-key", Err(format!("{}: {}", e.code, e.message)));
            return std::process::ExitCode::FAILURE;
        }
    };
    run.step(
        "create-user-key",
        if created.secret.len() == 43 && !created.key.admin {
            Ok(format!(
                "{:?} id {} (secret 43 chars, not shown)",
                created.key.name, created.key.id
            ))
        } else {
            Err("unexpected key shape".into())
        },
    );
    let user_cfg = cfg(a.port, &created.secret);

    // 3. The user key lists the library (the first scan hashes in the
    // background, so wait for it): exactly the expected models (untagged
    // folders are skipped), then download every expected file.
    let lib = match LibraryClient::new(user_cfg.clone(), UA) {
        Ok(l) => l,
        Err(e) => {
            run.step("library-list", Err(e.message));
            return std::process::ExitCode::FAILURE;
        }
    };
    let want_ids: Vec<&str> = a.expect.iter().map(|e| e.0.as_str()).collect();
    let t0 = Instant::now();
    let list = loop {
        match lib.list() {
            Ok(l)
                if want_ids
                    .iter()
                    .all(|id| l.models.iter().any(|m| m.id == *id)) =>
            {
                break Ok(l)
            }
            Ok(_) | Err(_) if t0.elapsed() < Duration::from_secs(60) => {
                std::thread::sleep(Duration::from_millis(250))
            }
            Ok(l) => {
                break Err(format!(
                    "listed {:?}, expected {want_ids:?}",
                    l.models.iter().map(|m| &m.id).collect::<Vec<_>>()
                ))
            }
            Err(e) => break Err(e.message),
        }
    };
    let list =
        list.and_then(
            |l| match l.models.iter().find(|m| !want_ids.contains(&m.id.as_str())) {
                Some(m) => Err(format!("unexpected model {:?} listed", m.id)),
                None => Ok(l),
            },
        );
    let list = match list {
        Ok(l) => {
            run.step(
                "library-list",
                Ok(format!(
                    "{} model(s): {}",
                    l.models.len(),
                    l.models
                        .iter()
                        .map(|m| format!("{} ({:?}, {} file(s))", m.id, m.kind, m.files.len()))
                        .collect::<Vec<_>>()
                        .join(", ")
                )),
            );
            l
        }
        Err(e) => {
            run.step("library-list", Err(e));
            return std::process::ExitCode::FAILURE;
        }
    };
    let mut total_bytes = 0u64;
    for (id, name, sha) in &a.expect {
        let step = format!("download {id}/{name}");
        let Some(f) = list
            .models
            .iter()
            .find(|m| &m.id == id)
            .and_then(|m| m.files.iter().find(|f| &f.name == name))
        else {
            run.step(&step, Err("not in the list".into()));
            continue;
        };
        if &f.sha256 != sha {
            run.step(
                &step,
                Err(format!(
                    "server lists sha256 {}, source file is {sha}",
                    f.sha256
                )),
            );
            continue;
        }
        let dest = a.out.join(id).join(name);
        let part = a.out.join(id).join(format!("{name}.part"));
        let t = Instant::now();
        let r = lib.download(id, f, &dest, &part, &mut |_| {}, &|| false);
        let r = r
            .map_err(|e| format!("{:?}: {}", e.kind, e.message))
            .and_then(|()| {
                let got = sha_file(&dest).map_err(|e| e.to_string())?;
                if &got == sha {
                    total_bytes += f.size;
                    Ok(format!(
                        "{} bytes in {} ms, sha256 {}… matches the source file",
                        f.size,
                        t.elapsed().as_millis(),
                        &got[..12]
                    ))
                } else {
                    Err(format!("downloaded sha256 {got} != {sha}"))
                }
            });
        run.step(&step, r);
    }

    // 4. A non-admin key gets 403 from the admin API.
    let user_admin = AdminClient::new(user_cfg.clone(), UA);
    run.step(
        "user-not-admin",
        match user_admin.list_keys() {
            Err(e) if e.code == code::FORBIDDEN => Ok("list_keys → forbidden".into()),
            Err(e) => Err(format!("{}: {}", e.code, e.message)),
            Ok(_) => Err("a user key listed the keys".into()),
        },
    );

    // 5. One dictation over the user key.
    let pcm = match flow_core::audio::read_wav_16k_mono(&a.wav) {
        Ok(p) => p,
        Err(e) => {
            run.step("dictation", Err(format!("read {}: {e}", a.wav.display())));
            return std::process::ExitCode::FAILURE;
        }
    };
    let audio_ms = pcm.len() as u64 / 16;
    let remote = RemoteBackend::new(user_cfg.clone(), UA);
    let session = SessionConfig {
        languages: vec!["en".into()],
        ..Default::default()
    };
    let r = remote
        .start_session(&session)
        .map_err(|e| format!("start: {:?} {}", e.kind, e.message))
        .and_then(|_| {
            for c in pcm.chunks(1600) {
                if !remote.push_audio(c) {
                    return Err(format!("push failed: {:?}", remote.session_failure()));
                }
            }
            let t = Instant::now();
            let res = remote
                .commit(Instant::now() + Duration::from_secs(60))
                .map_err(|e| format!("commit: {:?} {}", e.kind, e.message))?;
            let text = res.final_text.to_lowercase();
            if text.contains(&a.expect_text) {
                Ok(format!(
                    "{audio_ms} ms of audio → {} chars containing {:?} (commit→result {} ms)",
                    res.final_text.len(),
                    a.expect_text,
                    t.elapsed().as_millis()
                ))
            } else {
                Err(format!(
                    "status {:?}, text without {:?}: {:?}",
                    res.status, a.expect_text, res.final_text
                ))
            }
        });
    run.step("dictation", r);
    drop(remote);

    // 6. The admin sees the user's usage: dictations and downloads.
    let r = admin
        .list_keys()
        .map_err(|e| format!("{}: {}", e.code, e.message))
        .and_then(|l| {
            let k = l
                .keys
                .iter()
                .find(|k| k.id == created.key.id)
                .ok_or("user key missing from the list")?;
            let ok = k.dictations >= 1
                && k.audio_ms > 0
                && k.downloads >= a.expect.len() as u64
                && k.download_bytes == total_bytes
                && k.client.as_deref().is_some_and(|c| c.contains(UA));
            let m = format!(
                "{} keys; {:?}: dictations={} audio_ms={} downloads={} download_bytes={} client={:?} last_seen={:?}",
                l.keys.len(),
                k.name,
                k.dictations,
                k.audio_ms,
                k.downloads,
                k.download_bytes,
                k.client,
                k.last_seen
            );
            if ok {
                Ok(m)
            } else {
                Err(format!("usage short of expected (≥1 dictation, {} downloads, {total_bytes} bytes): {m}", a.expect.len()))
            }
        });
    run.step("admin-usage", r);

    // 7. Revoke, then the user key is refused everywhere.
    run.step(
        "revoke",
        match admin.revoke_key(&created.key.id) {
            Ok(r) if r.key.revoked.is_some() => Ok(format!("{} revoked", r.key.id)),
            Ok(_) => Err("answer has no revoked time".into()),
            Err(e) => Err(format!("{}: {}", e.code, e.message)),
        },
    );
    run.step(
        "refused-admin-api",
        match user_admin.whoami() {
            Err(e) if e.code == code::UNAUTHORIZED => Ok("whoami → unauthorized".into()),
            Err(e) => Err(format!("{}: {}", e.code, e.message)),
            Ok(_) => Err("revoked key still accepted".into()),
        },
    );
    run.step(
        "refused-library",
        match lib.list() {
            Err(e) if e.kind == ErrorKind::Auth => Ok("list → auth refused".into()),
            Err(e) => Err(format!("{:?}: {}", e.kind, e.message)),
            Ok(_) => Err("revoked key still lists the library".into()),
        },
    );
    let remote = RemoteBackend::new(user_cfg, UA);
    run.step(
        "refused-dictation",
        match remote.start_session(&session) {
            Err(e) if e.kind == Failure::AuthFailed => Ok("start_session → auth_failed".into()),
            Err(e) => Err(format!("{:?}: {}", e.kind, e.message)),
            Ok(_) => Err("revoked key started a session".into()),
        },
    );
    run.step(
        "admin-still-works",
        admin
            .whoami()
            .map(|w| format!("{:?} still accepted", w.name))
            .map_err(|e| format!("{}: {}", e.code, e.message)),
    );

    if run.failed {
        std::process::ExitCode::FAILURE
    } else {
        println!("ALL PASS");
        std::process::ExitCode::SUCCESS
    }
}
