//! D39 admin API through flow-ffi (feature `admin`): the JSON layer against
//! a real in-process flow-server, and (with `jni`) a real JVM driving the
//! `dev.flowclone.admin.NativeAdmin` exports. The JVM test skips when no
//! `java` is on PATH.
#![cfg(feature = "admin")]

use std::path::PathBuf;
use std::process::Command;

use flow_ffi::admin_api;

const TOKEN: &str = "admin-test-token-0123456789abcdef";

struct InProcessServer {
    addr: std::net::SocketAddr,
    _handle: flow_server::ServerHandle,
    _rt: tokio::runtime::Runtime,
}

impl InProcessServer {
    fn start() -> Self {
        use flow_core::config::{AsrBackend, ModelConfig};
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();
        // A token without state_dir = in-memory keys; the token is admin key `owner`.
        let cfg = flow_server::config::Config {
            bind: vec!["127.0.0.1:0".into()],
            token: Some(TOKEN.into()),
            model: ModelConfig {
                asr_backend: AsrBackend::Mock { text: "x".into() },
                ..Default::default()
            },
            ..Default::default()
        };
        let handle = rt.block_on(async {
            let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            flow_server::serve(cfg, vec![l])
                .await
                .expect("server starts")
        });
        Self {
            addr: handle.addr(),
            _handle: handle,
            _rt: rt,
        }
    }

    /// The `ServerConfig` JSON Android sends (`DevSettings.toServerConfigJson`).
    fn cfg(&self, token: &str) -> String {
        serde_json::json!({
            "host": self.addr.ip().to_string(), "port": self.addr.port(), "tls": false,
            "tls_pin_sha256": null, "token": token, "connect_timeout_ms": 1500,
            "commit_timeout_base_ms": 3000, "health_ttl_s": 30,
        })
        .to_string()
    }
}

fn json(line: &str) -> serde_json::Value {
    serde_json::from_str(line).unwrap_or_else(|e| panic!("{e}: {line}"))
}

#[test]
fn json_api_round_trip() {
    let s = InProcessServer::start();
    let cfg = s.cfg(TOKEN);
    let me = json(&admin_api::whoami(&cfg));
    assert_eq!(me["ok"], true, "{me}");
    assert_eq!(me["value"]["admin"], true, "{me}");
    let created = json(&admin_api::create_key(&cfg, " phone ", false));
    assert_eq!(created["ok"], true, "{created}");
    assert_eq!(created["value"]["key"]["name"], "phone");
    let secret = created["value"]["secret"].as_str().unwrap().to_owned();
    assert_eq!(secret.len(), 43);
    let list = json(&admin_api::list_keys(&cfg));
    let keys = list["value"]["keys"].as_array().unwrap();
    assert_eq!(keys.len(), 2, "{list}");
    assert!(keys.iter().all(|k| k.get("secret").is_none()), "{list}");
    assert!(keys
        .iter()
        .any(|k| k["current"] == true && k["id"] == me["value"]["id"]));
    assert!(
        keys[0]["downloads"].is_number(),
        "D40 download stats: {list}"
    );
    let phone = s.cfg(&secret);
    assert_eq!(json(&admin_api::whoami(&phone))["value"]["admin"], false);
    assert_eq!(json(&admin_api::list_keys(&phone))["code"], "forbidden");
    assert_eq!(
        json(&admin_api::create_key(&cfg, "phone", false))["code"],
        "name_taken"
    );
    let me_id = me["value"]["id"].as_str().unwrap();
    assert_eq!(
        json(&admin_api::revoke_key(&cfg, me_id))["code"],
        "last_admin"
    );
    let id = created["value"]["key"]["id"].as_str().unwrap();
    let r = json(&admin_api::revoke_key(&cfg, id));
    assert!(r["value"]["key"]["revoked"].is_number(), "{r}");
    assert_eq!(json(&admin_api::whoami(&phone))["code"], "unauthorized");
}

#[test]
fn contract_lists_every_admin_symbol() {
    let contract = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../apps/flow-android/JNI_CONTRACT.md"),
    )
    .unwrap();
    let want: std::collections::BTreeSet<&str> = contract
        .split('`')
        .filter(|w| w.starts_with("Java_dev_flowclone_admin_NativeAdmin_native"))
        .collect();
    assert_eq!(want.len(), 4, "{want:?}");
    let src = include_str!("../src/jni_admin.rs");
    let have: std::collections::BTreeSet<&str> = src
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|w| w.starts_with("Java_"))
        .collect();
    assert_eq!(have, want);
}

#[cfg(feature = "jni")]
#[test]
fn jvm_admin_round_trip() {
    if Command::new("java").arg("-version").output().is_err() {
        eprintln!("SKIP: no java on PATH");
        return;
    }
    let mut features = vec!["jni", "admin"];
    if cfg!(feature = "cloud") {
        features.push("cloud");
    }
    let st = Command::new(env!("CARGO"))
        .args(["build", "-q", "-p", "flow-ffi", "--lib", "--features"])
        .arg(features.join(","))
        .status()
        .unwrap();
    assert!(st.success());
    let exe = std::env::current_exe().unwrap();
    let lib_dir = exe.parent().unwrap().parent().unwrap().to_path_buf();
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/java/dev/flowclone/admin/JniAdmin.java");
    let s = InProcessServer::start();
    let out = Command::new("java")
        .arg(format!("-Djava.library.path={}", lib_dir.display()))
        .arg("--enable-native-access=ALL-UNNAMED")
        .arg(&src)
        .arg(s.cfg(TOKEN))
        .arg(s.cfg("TOKEN"))
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let l: Vec<&str> = stdout.lines().collect();
    assert_eq!(l.len(), 9, "{stdout}");
    assert_eq!(json(l[0])["value"]["admin"], true, "{}", l[0]);
    assert_eq!(json(l[1])["value"]["keys"].as_array().unwrap().len(), 1);
    let (created, len) = l[2].rsplit_once(" LEN ").unwrap();
    assert_eq!(len, "43");
    let created = json(created);
    assert_eq!(created["value"]["secret"], "<SECRET>");
    assert_eq!(created["value"]["key"]["name"], "phone");
    assert_eq!(json(l[3])["value"]["admin"], false);
    assert_eq!(json(l[4])["code"], "forbidden");
    assert!(
        json(l[5])["value"]["key"]["revoked"].is_number(),
        "{}",
        l[5]
    );
    assert_eq!(json(l[6])["code"], "unauthorized");
    assert_eq!(json(l[7])["code"], "bad_request");
    assert_eq!(json(l[8])["code"], "bad_request");
}
