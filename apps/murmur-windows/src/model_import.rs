//! Manual model import (air-gapped): install a catalogue model from a
//! folder, a `.zip` of its files, or (single-file models) the file itself,
//! with no network call anywhere on the path.
//!
//! * Only [`crate::model_download::CATALOG`] entries: every file must have the
//!   pinned size and SHA-256 the downloader checks. A source with none of the
//!   model's files is refused ("custom models come from the server library").
//! * Files are found by name anywhere in the folder / zip (a zip often has a
//!   top folder), else by their exact pinned size (a renamed copy); the hash
//!   decides either way.
//! * All or nothing: every needed file is copied to `<name>.part` and hashed
//!   on the way (`model_download::copy_verified`, the downloader's code);
//!   only when every one matches are they renamed into place. Any mismatch
//!   deletes all `.part` files, so nothing is installed.
//! * A file already in place and intact is not copied again (so importing
//!   just `ggml-tiny-q5_1.bin` completes a Hebrew model that lacks the
//!   language-ID extra). Extras are optional: missing ones are skipped.
//! * Zip entries are streamed straight into the `.part` path; their names
//!   never become paths (no zip-slip).

use crate::model_download::{
    copy_verified, intact, part_path, CatalogEntry, ModelFile, Progress, State, Stop,
};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

/// How deep a picked folder is searched, and how many files at most.
const MAX_DEPTH: usize = 4;
const MAX_FILES: usize = 10_000;

/// A file the source offers.
#[derive(Debug, Clone)]
struct Candidate {
    /// Base name (no folders).
    name: String,
    size: u64,
    at: Loc,
}

#[derive(Debug, Clone)]
enum Loc {
    Path(PathBuf),
    /// Index into the zip.
    Zip(usize),
}

/// What the user picked.
enum Source {
    Files(Vec<Candidate>),
    Zip(zip::ZipArchive<std::fs::File>, Vec<Candidate>),
}

impl Source {
    fn open(src: &Path) -> Result<Source, String> {
        let meta =
            std::fs::metadata(src).map_err(|e| format!("could not open {}: {e}", src.display()))?;
        if meta.is_dir() {
            let mut out = vec![];
            walk(src, 0, &mut out)?;
            return Ok(Source::Files(out));
        }
        if is_zip(src) {
            let f = std::fs::File::open(src)
                .map_err(|e| format!("could not open {}: {e}", src.display()))?;
            let mut z = zip::ZipArchive::new(f)
                .map_err(|e| format!("{} is not a readable zip: {e}", src.display()))?;
            let mut out = vec![];
            for i in 0..z.len().min(MAX_FILES) {
                let e = z
                    .by_index_raw(i)
                    .map_err(|e| format!("{} is not a readable zip: {e}", src.display()))?;
                if e.is_dir() {
                    continue;
                }
                let name = e
                    .name()
                    .rsplit(['/', '\\'])
                    .next()
                    .unwrap_or_default()
                    .to_string();
                if !name.is_empty() {
                    out.push(Candidate {
                        name,
                        size: e.size(),
                        at: Loc::Zip(i),
                    });
                }
            }
            return Ok(Source::Zip(z, out));
        }
        let name = src
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        Ok(Source::Files(vec![Candidate {
            name,
            size: meta.len(),
            at: Loc::Path(src.to_path_buf()),
        }]))
    }

    fn candidates(&self) -> &[Candidate] {
        match self {
            Source::Files(c) | Source::Zip(_, c) => c,
        }
    }
}

/// A zip by its magic (`PK\x03\x04`, or `PK\x05\x06` for an empty one).
fn is_zip(p: &Path) -> bool {
    let mut head = [0u8; 4];
    std::fs::File::open(p)
        .and_then(|mut f| f.read_exact(&mut head))
        .is_ok()
        && (head == *b"PK\x03\x04" || head == *b"PK\x05\x06")
}

fn walk(dir: &Path, depth: usize, out: &mut Vec<Candidate>) -> Result<(), String> {
    let rd =
        std::fs::read_dir(dir).map_err(|e| format!("could not read {}: {e}", dir.display()))?;
    for ent in rd.flatten() {
        if out.len() >= MAX_FILES {
            break;
        }
        // file_type() does not follow symlinks: a linked folder is skipped.
        let Ok(ft) = ent.file_type() else { continue };
        let p = ent.path();
        if ft.is_dir() {
            if depth + 1 < MAX_DEPTH {
                walk(&p, depth + 1, out)?;
            }
        } else if let Ok(m) = std::fs::metadata(&p) {
            if m.is_file() {
                let name = ent.file_name().to_string_lossy().into_owned();
                // Our own leftovers are never a source.
                if !name.ends_with(".part") {
                    out.push(Candidate {
                        name,
                        size: m.len(),
                        at: Loc::Path(p),
                    });
                }
            }
        }
    }
    Ok(())
}

/// The source's file for `f`: same name and size, else the first one with
/// the pinned size (a renamed copy). `Err(size)`: a same-named file of the wrong size.
fn find<'a>(cands: &'a [Candidate], f: &ModelFile) -> Result<Option<&'a Candidate>, u64> {
    let named = |c: &&Candidate| c.name.eq_ignore_ascii_case(f.name);
    if let Some(c) = cands.iter().filter(named).find(|c| c.size == f.size) {
        return Ok(Some(c));
    }
    if let Some(c) = cands.iter().find(|c| c.size == f.size) {
        return Ok(Some(c));
    }
    match cands.iter().find(named) {
        Some(c) => Err(c.size),
        None => Ok(None),
    }
}

/// One file to copy.
struct Step<'a> {
    f: &'static ModelFile,
    target: PathBuf,
    from: &'a Candidate,
}

/// Import `e` from `src` (folder, zip or file) into `models_dir`. Emits
/// `importing` / `verifying` progress through `emit`, but not the final
/// state: the caller sends that once prefs are updated (as with downloads).
pub fn run_import(
    e: &CatalogEntry,
    models_dir: &Path,
    src: &Path,
    cancel: &AtomicBool,
    emit: &mut dyn FnMut(Progress),
) -> Result<PathBuf, Stop> {
    let mut source = Source::open(src).map_err(Stop::Failed)?;
    let cands = source.candidates().to_vec();
    let mut steps: Vec<Step> = vec![];
    let mut missing: Vec<&str> = vec![];
    let mut wrong: Vec<String> = vec![];
    let mut matched = 0;
    for (f, extra) in e.all_files() {
        let target = e.file_path(models_dir, f, extra);
        match find(&cands, f) {
            Ok(Some(c)) => {
                matched += 1;
                // The picked file is the installed one itself: nothing to do.
                if let Loc::Path(p) = &c.at {
                    if same_file(p, &target) {
                        continue;
                    }
                }
                steps.push(Step { f, target, from: c });
            }
            Ok(None) | Err(_) if extra => {}
            Ok(None) | Err(_) if intact(&target, f) => {}
            Ok(None) => missing.push(f.name),
            Err(size) => {
                matched += 1;
                wrong.push(format!(
                    "{} is {size} bytes, {} needs {}",
                    f.name, e.title, f.size
                ));
            }
        }
    }
    if matched == 0 {
        return Err(Stop::Failed(format!(
            "{} has none of the files of {} ({}). Only the models listed here can be imported, \
             each checked against its built-in fingerprint; custom models will come from your server's library.",
            src.display(),
            e.title,
            e.files.iter().map(|f| f.name).collect::<Vec<_>>().join(", ")
        )));
    }
    if !wrong.is_empty() {
        return Err(Stop::Failed(format!(
            "{}: a different version or an incomplete copy. Nothing was installed.",
            wrong.join("; ")
        )));
    }
    if !missing.is_empty() {
        return Err(Stop::Failed(format!(
            "{} is missing {} of {}. Nothing was installed.",
            src.display(),
            missing.join(", "),
            e.title
        )));
    }
    if e.is_dir() {
        std::fs::create_dir_all(e.dest_path(models_dir))
            .map_err(|er| Stop::Failed(format!("could not create the models folder: {er}")))?;
    } else {
        std::fs::create_dir_all(models_dir)
            .map_err(|er| Stop::Failed(format!("could not create the models folder: {er}")))?;
    }
    let total: u64 = steps.iter().map(|s| s.f.size).sum();
    let id = e.id.to_string();
    let mut report = |received: u64, state: State| {
        emit(Progress {
            id: id.clone(),
            received,
            total,
            state,
            error: None,
            path: None,
        })
    };
    report(0, State::Importing);
    let mut parts: Vec<PathBuf> = vec![];
    let cleanup = |parts: &[PathBuf]| {
        for p in parts {
            let _ = std::fs::remove_file(p);
        }
    };
    let mut done = 0u64;
    for s in &steps {
        let part = part_path(&s.target, s.f);
        parts.push(part.clone());
        let r = copy_one(&mut source, s, &part, cancel, &mut |n| {
            report(done + n, State::Importing)
        });
        report(done + s.f.size, State::Verifying);
        match r {
            Ok(digest) if digest == s.f.sha256 => {}
            Ok(_) => {
                cleanup(&parts);
                return Err(Stop::Failed(format!(
                    "{} did not match its fingerprint (a different or damaged file). Nothing was installed.",
                    s.f.name
                )));
            }
            Err(stop) => {
                cleanup(&parts);
                return Err(stop);
            }
        }
        done += s.f.size;
    }
    if cancel.load(Ordering::SeqCst) {
        cleanup(&parts);
        return Err(Stop::Cancelled);
    }
    for (i, s) in steps.iter().enumerate() {
        if let Err(er) = std::fs::rename(&parts[i], &s.target) {
            cleanup(&parts[i..]);
            return Err(Stop::Failed(format!("could not save {}: {er}", s.f.name)));
        }
    }
    Ok(e.dest_path(models_dir))
}

fn copy_one(
    source: &mut Source,
    s: &Step,
    part: &Path,
    cancel: &AtomicBool,
    tick: &mut dyn FnMut(u64),
) -> Result<String, Stop> {
    let fail = |er: String| Stop::Failed(format!("could not read {}: {er}", s.from.name));
    match (&s.from.at, source) {
        (Loc::Path(p), _) => {
            let mut r = std::fs::File::open(p).map_err(|er| fail(er.to_string()))?;
            copy_verified(&mut r, s.f, part, "reading", cancel, tick)
        }
        (Loc::Zip(i), Source::Zip(z, _)) => {
            let mut r = z.by_index(*i).map_err(|er| fail(er.to_string()))?;
            copy_verified(&mut r, s.f, part, "unzipping", cancel, tick)
        }
        (Loc::Zip(_), Source::Files(_)) => Err(fail("not in a zip".into())),
    }
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(x), Ok(y)) => x == y,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model_download::ModelEngine;
    use sha2::{Digest, Sha256};
    use std::io::Write;

    fn sha(b: &[u8]) -> &'static str {
        let d = Sha256::digest(b);
        Box::leak(
            d.iter()
                .map(|x| format!("{x:02x}"))
                .collect::<String>()
                .into_boxed_str(),
        )
    }

    fn mf(name: &'static str, b: &[u8]) -> ModelFile {
        ModelFile {
            name,
            url: "https://huggingface.co/r/resolve/x/f",
            size: b.len() as u64,
            sha256: sha(b),
        }
    }

    fn entry(engine: ModelEngine, files: Vec<ModelFile>, extras: Vec<ModelFile>) -> CatalogEntry {
        CatalogEntry {
            id: "t",
            title: "Test model",
            desc: "",
            engine,
            dest: match engine {
                ModelEngine::Parakeet | ModelEngine::SherpaOnline => "pk-dir",
                ModelEngine::Llm => "m.gguf",
                ModelEngine::Whisper => "m.bin",
            },
            page: "https://huggingface.co/r",
            recommended: false,
            recommended_gpu: false,
            language: None,
            license: None,
            files: Box::leak(files.into_boxed_slice()),
            extras: Box::leak(extras.into_boxed_slice()),
        }
    }

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("murmur-imp-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn files_in(d: &Path) -> Vec<String> {
        fn walk(d: &Path, out: &mut Vec<PathBuf>) {
            for e in std::fs::read_dir(d).into_iter().flatten().flatten() {
                if e.path().is_dir() {
                    walk(&e.path(), out);
                } else {
                    out.push(e.path());
                }
            }
        }
        let mut v = vec![];
        walk(d, &mut v);
        let mut s: Vec<String> = v
            .iter()
            .map(|p| {
                p.strip_prefix(d)
                    .unwrap()
                    .display()
                    .to_string()
                    .replace('\\', "/")
            })
            .collect();
        s.sort();
        s
    }

    fn zip_of(path: &Path, files: &[(&str, &[u8])]) {
        let mut z = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
        let opt = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        for (name, b) in files {
            z.start_file(*name, opt).unwrap();
            z.write_all(b).unwrap();
        }
        z.finish().unwrap();
    }

    fn import(e: &CatalogEntry, models: &Path, src: &Path) -> (Result<PathBuf, Stop>, Vec<State>) {
        let mut seen = vec![];
        let r = run_import(e, models, src, &AtomicBool::new(false), &mut |p| {
            seen.push(p.state)
        });
        (r, seen)
    }

    fn failed(r: &Result<PathBuf, Stop>, needle: &str) -> bool {
        matches!(r, Err(Stop::Failed(m)) if m.contains(needle))
    }

    #[test]
    fn a_single_model_file_imports_and_is_installed() {
        let e = entry(ModelEngine::Llm, vec![mf("m.gguf", b"gguf bytes")], vec![]);
        let d = tmp("single");
        let src = d.join("src");
        std::fs::create_dir_all(&src).unwrap();
        // A renamed copy is found by its pinned size; the hash decides.
        std::fs::write(src.join("my-model.gguf"), b"gguf bytes").unwrap();
        let models = d.join("models");
        let (r, seen) = import(&e, &models, &src.join("my-model.gguf"));
        assert_eq!(r.unwrap(), models.join("m.gguf"));
        assert_eq!(files_in(&models), vec!["m.gguf"]);
        assert!(e.installed(&models));
        assert_eq!(seen.first(), Some(&State::Importing));
        assert!(seen.contains(&State::Verifying));
        // The same state as after a download: prefs point at it.
        let mut prefs = crate::prefs::Prefs::default();
        e.apply_to(&mut prefs, &models);
        assert!(prefs.llm_enabled);
        assert_eq!(
            prefs.llm_model_path.as_deref(),
            Some(models.join("m.gguf").display().to_string().as_str())
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_folder_model_imports_from_a_nested_folder() {
        let e = entry(
            ModelEngine::Parakeet,
            vec![mf("encoder.onnx", b"enc"), mf("tokens.txt", b"tokens")],
            vec![],
        );
        let d = tmp("folder");
        let src = d.join("src/sherpa-onnx-thing");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("encoder.onnx"), b"enc").unwrap();
        std::fs::write(src.join("tokens.txt"), b"tokens").unwrap();
        std::fs::write(src.join("README.md"), b"other stuff").unwrap();
        let models = d.join("models");
        let (r, _) = import(&e, &models, &d.join("src"));
        assert_eq!(r.unwrap(), models.join("pk-dir"));
        assert_eq!(
            files_in(&models),
            vec!["pk-dir/encoder.onnx", "pk-dir/tokens.txt"]
        );
        assert!(e.installed(&models));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_zip_with_a_top_folder_imports_with_its_extra() {
        let e = entry(
            ModelEngine::SherpaOnline,
            vec![
                mf("encoder.int8.onnx", b"encoder bytes"),
                mf("tokens.txt", b"tok"),
            ],
            vec![mf("lid.bin", b"language id")],
        );
        let d = tmp("zip");
        let z = d.join("model.zip");
        zip_of(
            &z,
            &[
                ("he/encoder.int8.onnx", b"encoder bytes"),
                ("he/tokens.txt", b"tok"),
                ("he/lid.bin", b"language id"),
            ],
        );
        let models = d.join("models");
        let (r, _) = import(&e, &models, &z);
        r.unwrap();
        assert_eq!(
            files_in(&models),
            vec!["lid.bin", "pk-dir/encoder.int8.onnx", "pk-dir/tokens.txt"]
        );
        assert!(e.complete(&models));
        // A zip entry's name never becomes a path: "../x" lands as the model file.
        let evil = d.join("evil.zip");
        zip_of(&evil, &[("../../m.bin", b"whisper")]);
        let w = entry(ModelEngine::Whisper, vec![mf("m.bin", b"whisper")], vec![]);
        import(&w, &models, &evil).0.unwrap();
        assert!(models.join("m.bin").is_file());
        assert!(!d.join("m.bin").exists() && !d.parent().unwrap().join("m.bin").exists());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn missing_extras_are_skipped_and_can_be_imported_later() {
        let e = entry(
            ModelEngine::SherpaOnline,
            vec![
                mf("encoder.int8.onnx", b"encoder bytes"),
                mf("tokens.txt", b"tok"),
            ],
            vec![mf("lid.bin", b"language id")],
        );
        let d = tmp("extras");
        let z = d.join("model.zip");
        zip_of(
            &z,
            &[
                ("encoder.int8.onnx", b"encoder bytes"),
                ("tokens.txt", b"tok"),
            ],
        );
        let models = d.join("models");
        import(&e, &models, &z).0.unwrap();
        assert!(e.installed(&models) && !e.complete(&models));
        // Later: just the language-ID file; the model files stay as they are.
        std::fs::write(d.join("lid.bin"), b"language id").unwrap();
        import(&e, &models, &d.join("lid.bin")).0.unwrap();
        assert!(e.complete(&models));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_hash_mismatch_installs_nothing() {
        // Two files: the first matches, the second has the right size but
        // other bytes. Neither may land, and no .part is left.
        let e = entry(
            ModelEngine::Parakeet,
            vec![mf("encoder.onnx", b"enc"), mf("tokens.txt", b"tokens")],
            vec![],
        );
        let d = tmp("badhash");
        let src = d.join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("encoder.onnx"), b"enc").unwrap();
        std::fs::write(src.join("tokens.txt"), b"TOKENS").unwrap();
        let models = d.join("models");
        let (r, _) = import(&e, &models, &src);
        assert!(
            failed(&r, "tokens.txt did not match its fingerprint"),
            "{r:?}"
        );
        assert!(failed(&r, "Nothing was installed"));
        assert!(files_in(&models).is_empty(), "{:?}", files_in(&models));
        assert!(!e.installed(&models));
        // Same through a zip.
        let z = d.join("m.zip");
        zip_of(&z, &[("encoder.onnx", b"enc"), ("tokens.txt", b"TOKENS")]);
        assert!(failed(&import(&e, &models, &z).0, "fingerprint"));
        assert!(files_in(&models).is_empty());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn unknown_files_are_refused() {
        let e = entry(ModelEngine::Llm, vec![mf("m.gguf", b"gguf bytes")], vec![]);
        let d = tmp("unknown");
        std::fs::write(
            d.join("custom-finetune.gguf"),
            b"some other model, not ours",
        )
        .unwrap();
        let models = d.join("models");
        let (r, seen) = import(&e, &models, &d.join("custom-finetune.gguf"));
        assert!(failed(&r, "none of the files of Test model"), "{r:?}");
        assert!(failed(&r, "server's library"));
        assert!(seen.is_empty());
        assert!(!models.exists());
        // A same-named file of the wrong size: says so.
        std::fs::write(d.join("m.gguf"), b"short").unwrap();
        let src = d.join("m.gguf");
        let (r, _) = import(&e, &models, &src);
        assert!(
            failed(&r, "m.gguf is 5 bytes, Test model needs 10"),
            "{r:?}"
        );
        assert!(!models.exists());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_folder_missing_a_file_or_with_a_wrong_size_installs_nothing() {
        let e = entry(
            ModelEngine::Parakeet,
            vec![mf("encoder.onnx", b"enc"), mf("tokens.txt", b"tokens")],
            vec![],
        );
        let d = tmp("partial");
        let src = d.join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("encoder.onnx"), b"enc").unwrap();
        let models = d.join("models");
        let (r, _) = import(&e, &models, &src);
        assert!(failed(&r, "is missing tokens.txt of Test model"), "{r:?}");
        assert!(files_in(&models).is_empty());
        std::fs::write(src.join("tokens.txt"), b"tokens, but a newer version").unwrap();
        let (r, _) = import(&e, &models, &src);
        assert!(
            failed(&r, "tokens.txt is 27 bytes, Test model needs 6"),
            "{r:?}"
        );
        assert!(files_in(&models).is_empty());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_cancel_leaves_no_part_file() {
        let e = entry(
            ModelEngine::Whisper,
            vec![mf("m.bin", b"whisper bytes")],
            vec![],
        );
        let d = tmp("cancel");
        std::fs::write(d.join("m.bin"), b"whisper bytes").unwrap();
        let models = d.join("models");
        let r = run_import(
            &e,
            &models,
            &d.join("m.bin"),
            &AtomicBool::new(true),
            &mut |_| {},
        );
        assert_eq!(r, Err(Stop::Cancelled));
        assert!(files_in(&models).is_empty());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn importing_the_installed_file_itself_is_a_no_op() {
        let e = entry(
            ModelEngine::Whisper,
            vec![mf("m.bin", b"whisper bytes")],
            vec![],
        );
        let d = tmp("self");
        let models = d.join("models");
        std::fs::create_dir_all(&models).unwrap();
        std::fs::write(models.join("m.bin"), b"whisper bytes").unwrap();
        import(&e, &models, &models.join("m.bin")).0.unwrap();
        assert_eq!(
            std::fs::read(models.join("m.bin")).unwrap(),
            b"whisper bytes"
        );
        assert_eq!(files_in(&models), vec!["m.bin"]);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn works_for_every_catalogue_kind() {
        // Every downloadable entry resolves its files from a folder named
        // like the download (sizes faked: only the matching is checked).
        for e in crate::model_download::CATALOG {
            let cands: Vec<Candidate> = e
                .all_files()
                .map(|(f, _)| Candidate {
                    name: f.name.into(),
                    size: f.size,
                    at: Loc::Path(PathBuf::from(f.name)),
                })
                .collect();
            for (f, _) in e.all_files() {
                assert!(
                    matches!(find(&cands, f), Ok(Some(c)) if c.name == f.name),
                    "{} {}",
                    e.id,
                    f.name
                );
            }
        }
    }
}
