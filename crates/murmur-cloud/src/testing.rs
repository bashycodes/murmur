//! In-process mock HTTP server for contract tests (feature `mock-server`).
//! std only: a thread on `127.0.0.1:0` that records every request and
//! answers from a handler. One request per connection (`Connection:
//! close`), plain HTTP. Point the crate at it with [`override_base_url`]
//! (the production base URL is a constant; this seam exists only with the
//! `mock-server` feature).

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

static OVERRIDE: Mutex<Option<String>> = Mutex::new(None);
static SERIAL: Mutex<()> = Mutex::new(());

/// While the guard lives, every [`crate::api::Api`] request goes to `base`
/// (e.g. `srv.url("/api/v1")`) instead of `https://openrouter.ai/api/v1`.
/// Guards are serialised process-wide, so tests using it run one at a time.
pub fn override_base_url(base: &str) -> BaseOverride {
    let serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    *OVERRIDE.lock().unwrap_or_else(|e| e.into_inner()) =
        Some(base.trim_end_matches('/').to_string());
    BaseOverride { _serial: serial }
}

/// See [`override_base_url`].
pub struct BaseOverride {
    _serial: MutexGuard<'static, ()>,
}

impl BaseOverride {
    /// Point the override elsewhere (e.g. a closed port) while holding it.
    pub fn set(&self, base: &str) {
        *OVERRIDE.lock().unwrap_or_else(|e| e.into_inner()) =
            Some(base.trim_end_matches('/').to_string());
    }
}

impl Drop for BaseOverride {
    fn drop(&mut self) {
        *OVERRIDE.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }
}

pub(crate) fn current_override() -> Option<String> {
    OVERRIDE.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// A request as the server saw it.
#[derive(Debug, Clone, Default)]
pub struct Recorded {
    pub method: String,
    /// Path including the query string.
    pub target: String,
    /// Header names lower-cased.
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Recorded {
    pub fn path(&self) -> &str {
        self.target.split('?').next().unwrap_or("")
    }
    pub fn header(&self, name: &str) -> Option<&str> {
        let n = name.to_ascii_lowercase();
        self.headers
            .iter()
            .find(|(k, _)| *k == n)
            .map(|(_, v)| v.as_str())
    }
    /// Decoded query pairs, in order (repeated keys kept).
    pub fn query(&self) -> Vec<(String, String)> {
        let Some(q) = self.target.split_once('?').map(|x| x.1) else {
            return Vec::new();
        };
        q.split('&')
            .filter(|s| !s.is_empty())
            .map(|kv| {
                let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
                (pct_decode(k), pct_decode(v))
            })
            .collect()
    }
    pub fn query_all(&self, key: &str) -> Vec<String> {
        self.query()
            .into_iter()
            .filter(|(k, _)| k == key)
            .map(|(_, v)| v)
            .collect()
    }
    pub fn json(&self) -> serde_json::Value {
        serde_json::from_slice(&self.body).expect("JSON body")
    }
    /// Multipart parts (`name`, `filename`, `content-type`, data).
    pub fn parts(&self) -> Vec<Part> {
        let ct = self.header("content-type").unwrap_or("");
        let Some(b) = ct.split("boundary=").nth(1) else {
            return Vec::new();
        };
        let boundary = format!("--{}", b.trim_matches('"'));
        let mut out = Vec::new();
        let body = &self.body;
        let mut pos = 0;
        let find = |hay: &[u8], needle: &[u8], from: usize| {
            hay[from..]
                .windows(needle.len())
                .position(|w| w == needle)
                .map(|p| p + from)
        };
        while let Some(start) = find(body, boundary.as_bytes(), pos) {
            let after = start + boundary.len();
            if body[after..].starts_with(b"--") {
                break;
            }
            let hdr_start = after + 2;
            let Some(hdr_end) = find(body, b"\r\n\r\n", hdr_start) else {
                break;
            };
            let headers = String::from_utf8_lossy(&body[hdr_start..hdr_end]).to_string();
            let data_start = hdr_end + 4;
            let next =
                find(body, format!("\r\n{boundary}").as_bytes(), data_start).unwrap_or(body.len());
            let mut p = Part {
                data: body[data_start..next].to_vec(),
                ..Default::default()
            };
            for line in headers.split("\r\n") {
                let l = line.to_ascii_lowercase();
                if l.starts_with("content-disposition") {
                    p.name = attr(line, "name").unwrap_or_default();
                    p.filename = attr(line, "filename");
                } else if l.starts_with("content-type") {
                    p.content_type = line.split_once(':').map(|x| x.1.trim().to_string());
                }
            }
            out.push(p);
            pos = next + 2;
        }
        out
    }
    /// Text values of all parts named `name`.
    pub fn fields(&self, name: &str) -> Vec<String> {
        self.parts()
            .into_iter()
            .filter(|p| p.name == name)
            .map(|p| String::from_utf8_lossy(&p.data).to_string())
            .collect()
    }
    pub fn field(&self, name: &str) -> Option<String> {
        self.fields(name).into_iter().next()
    }
    /// Everything the server received, as lossy text (leak checks).
    pub fn raw_text(&self) -> String {
        let mut s = format!("{} {}\n", self.method, self.target);
        for (k, v) in &self.headers {
            s.push_str(&format!("{k}: {v}\n"));
        }
        s.push_str(&String::from_utf8_lossy(&self.body));
        s
    }
}

fn attr(line: &str, key: &str) -> Option<String> {
    let pat = format!("{key}=\"");
    let i = line
        .match_indices(&pat)
        .find(|(i, _)| *i == 0 || !line.as_bytes()[i - 1].is_ascii_alphanumeric())?
        .0
        + pat.len();
    let rest = &line[i..];
    Some(rest[..rest.find('"')?].to_string())
}

fn pct_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < b.len() => {
                let h = u8::from_str_radix(&s[i + 1..i + 3], 16).unwrap_or(b'?');
                out.push(h);
                i += 2;
            }
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

#[derive(Debug, Clone, Default)]
pub struct Part {
    pub name: String,
    pub filename: Option<String>,
    pub content_type: Option<String>,
    pub data: Vec<u8>,
}

/// The server's answer.
#[derive(Debug, Clone)]
pub struct Reply {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    /// Sleep before answering (timeouts).
    pub delay: Duration,
    /// Streamed body: (pause before, bytes) pieces sent after `body`, with
    /// no Content-Length (the connection close ends the body).
    pub chunks: Vec<(Duration, Vec<u8>)>,
}

impl Reply {
    pub fn json(status: u16, v: serde_json::Value) -> Self {
        Self {
            status,
            headers: vec![("Content-Type".into(), "application/json".into())],
            body: v.to_string().into_bytes(),
            delay: Duration::ZERO,
            chunks: Vec::new(),
        }
    }
    pub fn raw(status: u16, body: &str) -> Self {
        Self {
            status,
            headers: vec![("Content-Type".into(), "text/plain".into())],
            body: body.as_bytes().to_vec(),
            delay: Duration::ZERO,
            chunks: Vec::new(),
        }
    }
    /// A `text/event-stream` answer: `events` are sent one by one, `gap`
    /// apart (each is written as is; include the trailing blank line).
    pub fn sse(events: &[&str], gap: Duration) -> Self {
        Self {
            status: 200,
            headers: vec![("Content-Type".into(), "text/event-stream".into())],
            body: Vec::new(),
            delay: Duration::ZERO,
            chunks: events
                .iter()
                .map(|e| (gap, e.as_bytes().to_vec()))
                .collect(),
        }
    }
    pub fn header(mut self, k: &str, v: &str) -> Self {
        self.headers.push((k.into(), v.into()));
        self
    }
    pub fn delayed(mut self, d: Duration) -> Self {
        self.delay = d;
        self
    }
}

type Handler = dyn Fn(&Recorded, usize) -> Reply + Send + Sync;

/// A running mock server; stops when dropped.
pub struct MockServer {
    pub addr: std::net::SocketAddr,
    requests: Arc<Mutex<Vec<Recorded>>>,
    stop: Arc<AtomicBool>,
    aborted: Arc<AtomicUsize>,
}

impl MockServer {
    /// `handler(request, index)` answers request number `index` (0-based).
    pub fn start(handler: impl Fn(&Recorded, usize) -> Reply + Send + Sync + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock server");
        let addr = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let aborted = Arc::new(AtomicUsize::new(0));
        let (r2, s2, a2) = (requests.clone(), stop.clone(), aborted.clone());
        let handler: Arc<Handler> = Arc::new(handler);
        std::thread::spawn(move || {
            while !s2.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let (r3, h, a3) = (r2.clone(), handler.clone(), a2.clone());
                        std::thread::spawn(move || serve(stream, &r3, &*h, &a3));
                    }
                    Err(_) => std::thread::sleep(Duration::from_millis(2)),
                }
            }
        });
        Self {
            addr,
            requests,
            stop,
            aborted,
        }
    }

    /// Streamed replies whose client went away before the last piece.
    pub fn aborted_streams(&self) -> usize {
        self.aborted.load(Ordering::SeqCst)
    }

    /// Always the same reply.
    pub fn fixed(reply: Reply) -> Self {
        Self::start(move |_, _| reply.clone())
    }

    /// Replies in order; the last one repeats.
    pub fn sequence(replies: Vec<Reply>) -> Self {
        Self::start(move |_, i| replies[i.min(replies.len() - 1)].clone())
    }

    /// `http://127.0.0.1:port` + `path`.
    pub fn url(&self, path: &str) -> String {
        format!("http://{}{}", self.addr, path)
    }

    pub fn requests(&self) -> Vec<Recorded> {
        self.requests.lock().unwrap().clone()
    }

    pub fn count(&self) -> usize {
        self.requests.lock().unwrap().len()
    }
}

impl Drop for MockServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

fn serve(stream: TcpStream, log: &Mutex<Vec<Recorded>>, handler: &Handler, aborted: &AtomicUsize) {
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
    let mut w = match stream.try_clone() {
        Ok(w) => w,
        Err(_) => return,
    };
    let mut r = BufReader::new(stream);
    let mut line = String::new();
    if r.read_line(&mut line).unwrap_or(0) == 0 {
        return;
    }
    let mut it = line.split_whitespace();
    let mut rec = Recorded {
        method: it.next().unwrap_or("").to_string(),
        target: it.next().unwrap_or("").to_string(),
        ..Default::default()
    };
    let mut len = 0usize;
    let mut chunked = false;
    loop {
        let mut h = String::new();
        if r.read_line(&mut h).unwrap_or(0) == 0 {
            return;
        }
        let h = h.trim_end();
        if h.is_empty() {
            break;
        }
        if let Some((k, v)) = h.split_once(':') {
            let (k, v) = (k.trim().to_ascii_lowercase(), v.trim().to_string());
            if k == "content-length" {
                len = v.parse().unwrap_or(0);
            }
            if k == "transfer-encoding" && v.eq_ignore_ascii_case("chunked") {
                chunked = true;
            }
            rec.headers.push((k, v));
        }
    }
    if chunked {
        loop {
            let mut sz = String::new();
            if r.read_line(&mut sz).unwrap_or(0) == 0 {
                break;
            }
            let n =
                usize::from_str_radix(sz.trim().split(';').next().unwrap_or("0"), 16).unwrap_or(0);
            let mut buf = vec![0; n + 2];
            if r.read_exact(&mut buf).is_err() {
                break;
            }
            if n == 0 {
                break;
            }
            rec.body.extend_from_slice(&buf[..n]);
        }
    } else {
        let mut buf = vec![0; len];
        if r.read_exact(&mut buf).is_err() {
            return;
        }
        rec.body = buf;
    }
    let idx = {
        let mut l = log.lock().unwrap();
        l.push(rec.clone());
        l.len() - 1
    };
    let reply = handler(&rec, idx);
    std::thread::sleep(reply.delay);
    let mut out = if reply.chunks.is_empty() {
        format!(
            "HTTP/1.1 {} X\r\nContent-Length: {}\r\nConnection: close\r\n",
            reply.status,
            reply.body.len()
        )
    } else {
        format!("HTTP/1.1 {} X\r\nConnection: close\r\n", reply.status)
    };
    for (k, v) in &reply.headers {
        out.push_str(&format!("{k}: {v}\r\n"));
    }
    out.push_str("\r\n");
    let _ = w.write_all(out.as_bytes());
    let _ = w.write_all(&reply.body);
    let _ = w.flush();
    for (gap, piece) in &reply.chunks {
        std::thread::sleep(*gap);
        // A closed peer shows up as a write error (maybe one write late).
        if w.write_all(piece).and_then(|_| w.flush()).is_err() {
            aborted.fetch_add(1, Ordering::SeqCst);
            return;
        }
    }
    let _ = w.shutdown(std::net::Shutdown::Write);
}
