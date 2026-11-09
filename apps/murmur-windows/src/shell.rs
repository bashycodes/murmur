//! Native file/folder pickers and "open this link in the browser".
//!
//! * [`PickRequest`]: what the page asked for (`pick_path`), validated here
//!   so the Win32 side only ever sees a short title and `*.ext` patterns.
//!   On Windows it is shown with `IFileOpenDialog` on its own STA thread,
//!   owned by the settings window; elsewhere [`pick`] returns an error.
//! * [`check_external_url`]: `open_external` opens only https/http links to
//!   the hosts the app itself points at (the project's GitHub repo, Hugging
//!   Face model pages, OpenRouter); anything else is refused, so a page can
//!   never use it to launch an arbitrary URL, file or program.

use serde::Deserialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PickKind {
    Folder,
    File,
}

/// One "Files of type" entry: `name` shown, `pattern` like `*.bin;*.gguf`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct PickFilter {
    pub name: String,
    pub pattern: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PickRequest {
    pub kind: PickKind,
    pub title: Option<String>,
    pub filters: Vec<PickFilter>,
}

const MAX_TITLE: usize = 120;
const MAX_FILTERS: usize = 8;

fn pattern_ok(p: &str) -> bool {
    !p.is_empty()
        && p.len() <= 120
        && p.split(';').all(|one| {
            one == "*.*"
                || one.strip_prefix("*.").is_some_and(|ext| {
                    !ext.is_empty() && ext.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                })
        })
}

impl PickRequest {
    /// Validate the page's arguments. Filters only make sense for files.
    pub fn new(
        kind: PickKind,
        title: Option<String>,
        filters: Option<Vec<PickFilter>>,
    ) -> Result<Self, String> {
        let title = title
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty());
        if let Some(t) = &title {
            if t.chars().count() > MAX_TITLE || t.chars().any(char::is_control) {
                return Err("bad picker title".into());
            }
        }
        let filters = filters.unwrap_or_default();
        if kind == PickKind::Folder && !filters.is_empty() {
            return Err("a folder picker takes no file types".into());
        }
        if filters.len() > MAX_FILTERS {
            return Err("too many file types".into());
        }
        for f in &filters {
            let name_ok = !f.name.trim().is_empty()
                && f.name.chars().count() <= 60
                && !f.name.chars().any(char::is_control);
            if !name_ok || !pattern_ok(&f.pattern) {
                return Err(format!("bad file type {:?}", f.name));
            }
        }
        Ok(PickRequest {
            kind,
            title,
            filters,
        })
    }
}

/// Hosts `open_external` may open, with the path prefix each must start
/// with (`""` = any path on that host).
const EXTERNAL: &[(&str, &str)] = &[
    ("github.com", "/bashycodes/murmur"),
    ("huggingface.co", ""),
    ("openrouter.ai", ""),
];

/// Ok when `url` is an http(s) link to an allowed host and path. No user
/// info, no port, no backslashes, quotes, spaces or control characters (so
/// the shell can't read it as anything but one URL).
pub fn check_external_url(url: &str) -> Result<(), String> {
    let refuse = || Err(format!("not an allowed link: {url}"));
    if url.len() > 2048
        || url.chars().any(|c| {
            c.is_control()
                || c.is_whitespace()
                || matches!(c, '\\' | '"' | '<' | '>' | '`' | '^' | '{' | '}' | '|')
        })
    {
        return refuse();
    }
    let Some(rest) = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
    else {
        return refuse();
    };
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (host, tail) = rest.split_at(end);
    // `user@host`, `host:port` or an empty host: no.
    if host.is_empty() || host.contains(['@', ':', '%']) {
        return refuse();
    }
    let host = host.to_ascii_lowercase();
    let path = &tail[..tail.find(['?', '#']).unwrap_or(tail.len())];
    // No `..` tricks to climb out of an allowed prefix.
    if path.split('/').any(|seg| seg == ".." || seg == ".") {
        return refuse();
    }
    let ok = EXTERNAL.iter().any(|(h, prefix)| {
        host == *h
            && (prefix.is_empty()
                || path == *prefix
                || path
                    .strip_prefix(prefix)
                    .is_some_and(|r| r.starts_with('/')))
    });
    if ok {
        Ok(())
    } else {
        refuse()
    }
}

/// Show the picker owned by `owner` (an HWND as an integer, 0 = none).
/// Blocks until the user closes it: call it off the UI thread.
/// `Ok(None)` = cancelled.
#[cfg(windows)]
pub fn pick(owner: isize, req: PickRequest) -> Result<Option<String>, String> {
    // A fresh thread: the dialog needs a single-threaded apartment, which a
    // pooled worker may already have set up differently.
    std::thread::spawn(move || win::pick(owner, &req))
        .join()
        .map_err(|_| "the picker crashed".to_string())?
}

#[cfg(not(windows))]
pub fn pick(_owner: isize, _req: PickRequest) -> Result<Option<String>, String> {
    Err("file pickers exist only on Windows".into())
}

/// Open an allowed link in the default browser.
#[cfg(windows)]
pub fn open_external(url: &str) -> Result<(), String> {
    check_external_url(url)?;
    let url = url.to_string();
    std::thread::spawn(move || win::open(&url))
        .join()
        .map_err(|_| "could not open the link".to_string())?
}

#[cfg(not(windows))]
pub fn open_external(url: &str) -> Result<(), String> {
    check_external_url(url)?;
    Err("opening links exists only on Windows".into())
}

#[cfg(windows)]
mod win {
    use super::{PickKind, PickRequest};
    use windows::core::{HSTRING, PCWSTR};
    use windows::Win32::Foundation::{ERROR_CANCELLED, HWND};
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_INPROC_SERVER,
        COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE,
    };
    use windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;
    use windows::Win32::UI::Shell::{
        FileOpenDialog, IFileOpenDialog, ShellExecuteW, FOS_FILEMUSTEXIST, FOS_FORCEFILESYSTEM,
        FOS_PATHMUSTEXIST, FOS_PICKFOLDERS, SIGDN_FILESYSPATH,
    };
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    struct Com(bool);
    impl Drop for Com {
        fn drop(&mut self) {
            if self.0 {
                unsafe { CoUninitialize() };
            }
        }
    }

    fn com() -> Com {
        Com(
            unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) }
                .is_ok(),
        )
    }

    pub fn pick(owner: isize, req: &PickRequest) -> Result<Option<String>, String> {
        let _com = com();
        unsafe { show(owner, req) }.or_else(|e| {
            if e.code() == ERROR_CANCELLED.to_hresult() {
                Ok(None)
            } else {
                Err(format!("picker: {e}"))
            }
        })
    }

    unsafe fn show(owner: isize, req: &PickRequest) -> windows::core::Result<Option<String>> {
        let dlg: IFileOpenDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)?;
        let mut opts = dlg.GetOptions()? | FOS_FORCEFILESYSTEM | FOS_PATHMUSTEXIST;
        if req.kind == PickKind::Folder {
            opts |= FOS_PICKFOLDERS;
        } else {
            opts |= FOS_FILEMUSTEXIST;
        }
        dlg.SetOptions(opts)?;
        if let Some(t) = &req.title {
            dlg.SetTitle(&HSTRING::from(t.as_str()))?;
        }
        // The HSTRINGs must outlive Show (the specs point into them).
        let names: Vec<HSTRING> = req
            .filters
            .iter()
            .map(|f| HSTRING::from(f.name.as_str()))
            .collect();
        let pats: Vec<HSTRING> = req
            .filters
            .iter()
            .map(|f| HSTRING::from(f.pattern.as_str()))
            .collect();
        if !names.is_empty() {
            let specs: Vec<COMDLG_FILTERSPEC> = names
                .iter()
                .zip(&pats)
                .map(|(n, p)| COMDLG_FILTERSPEC {
                    pszName: PCWSTR(n.as_ptr()),
                    pszSpec: PCWSTR(p.as_ptr()),
                })
                .collect();
            dlg.SetFileTypes(&specs)?;
        }
        dlg.Show(HWND(owner as _))?;
        let item = dlg.GetResult()?;
        let p = item.GetDisplayName(SIGDN_FILESYSPATH)?;
        let s = p.to_string();
        CoTaskMemFree(Some(p.0 as _));
        // A file-system path is valid UTF-16; anything else = no usable path.
        Ok(s.ok())
    }

    pub fn open(url: &str) -> Result<(), String> {
        let _com = com();
        let r = unsafe {
            ShellExecuteW(
                HWND::default(),
                &HSTRING::from("open"),
                &HSTRING::from(url),
                PCWSTR::null(),
                PCWSTR::null(),
                SW_SHOWNORMAL,
            )
        };
        // > 32 = success (the old HINSTANCE convention).
        if r.0 as isize > 32 {
            Ok(())
        } else {
            Err(format!("could not open the link (code {})", r.0 as isize))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allowed_links_open() {
        for u in [
            "https://github.com/bashycodes/murmur",
            "https://github.com/bashycodes/murmur/",
            "https://github.com/bashycodes/murmur#set-up-a-server",
            "https://github.com/bashycodes/murmur/blob/integration/docs/hybrid-architecture.md",
            "https://huggingface.co/ggerganov/whisper.cpp",
            "https://HuggingFace.co/csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8",
            "https://openrouter.ai/settings/keys",
            "https://openrouter.ai",
            "http://openrouter.ai/keys?x=1",
        ] {
            assert_eq!(check_external_url(u), Ok(()), "{u}");
        }
    }

    #[test]
    fn everything_else_is_refused() {
        for u in [
            "",
            "file:///C:/Windows/System32/calc.exe",
            "C:\\Windows\\System32\\calc.exe",
            "calc.exe",
            "ms-settings:privacy",
            "javascript:alert(1)",
            "https://github.com/someone-else/repo",
            "https://github.com/bashycodes/murmur-evil",
            "https://github.com/bashycodes/murmur/../../evil/x",
            "https://github.com",
            "https://evil.com/huggingface.co",
            "https://huggingface.co.evil.com/x",
            "https://huggingface.co@evil.com/x",
            "https://evil.com#@huggingface.co",
            "https://huggingface.co:8443/x",
            "https://huggingface.co/x y",
            "https://huggingface.co/x\"&calc",
            "https://huggingface.co/a\\b",
            "https://huggingface.co/a\nb",
            "https://%68uggingface.co/x",
            "ftp://huggingface.co/x",
            "//huggingface.co/x",
        ] {
            assert!(check_external_url(u).is_err(), "{u:?} accepted");
        }
    }

    #[test]
    fn picker_requests_are_validated() {
        let bin = PickFilter {
            name: "Whisper model".into(),
            pattern: "*.bin".into(),
        };
        let r = PickRequest::new(
            PickKind::File,
            Some("  Choose a model  ".into()),
            Some(vec![bin.clone()]),
        )
        .unwrap();
        assert_eq!(r.title.as_deref(), Some("Choose a model"));
        assert_eq!(r.filters, vec![bin.clone()]);
        assert!(PickRequest::new(PickKind::Folder, None, None).is_ok());
        assert!(PickRequest::new(PickKind::Folder, None, Some(vec![bin.clone()])).is_err());
        assert!(PickRequest::new(PickKind::File, Some("a\u{7}".into()), None).is_err());
        assert!(PickRequest::new(PickKind::File, Some("x".repeat(200)), None).is_err());
        for bad in [
            "",
            "*",
            "calc.exe",
            "*.b in",
            "*.bin;",
            "C:\\*.bin",
            "*.bin;../x",
        ] {
            let f = PickFilter {
                name: "x".into(),
                pattern: bad.into(),
            };
            assert!(
                PickRequest::new(PickKind::File, None, Some(vec![f])).is_err(),
                "{bad}"
            );
        }
        let ok = PickFilter {
            name: "Models".into(),
            pattern: "*.bin;*.gguf;*.*".into(),
        };
        assert!(PickRequest::new(PickKind::File, None, Some(vec![ok])).is_ok());
        let kind: PickKind = serde_json::from_str("\"folder\"").unwrap();
        assert_eq!(kind, PickKind::Folder);
    }

    #[cfg(not(windows))]
    #[test]
    fn non_windows_builds_refuse() {
        let r = PickRequest::new(PickKind::Folder, None, None).unwrap();
        assert!(pick(0, r).is_err());
        assert!(open_external("https://openrouter.ai").is_err());
        assert!(open_external("calc.exe")
            .unwrap_err()
            .contains("not an allowed"));
    }
}
