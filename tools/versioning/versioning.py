#!/usr/bin/env python3
"""Product version rules for Flow Clone (docs/versioning.md, DECISIONS D26).

One version for the whole repo lives in the top-level ``VERSION`` file.
Everything else either derives it at build time (Android Gradle, Tauri via
Cargo.toml, every crate via ``version.workspace``) or is a stamped copy that
``tools/release.sh`` rewrites and ``check`` verifies (the two Cargo
manifests, their lockfiles' path packages, the Windows UI package files).

Used by tools/check-version.sh and tools/release.sh; no dependencies beyond
the standard library.

  versioning.py check [--root R] [--tag vX.Y.Z]
  versioning.py stamp X.Y.Z [--root R]
  versioning.py changelog-release X.Y.Z YYYY-MM-DD [--root R] [--repo-url URL]
  versioning.py section X.Y.Z [--root R]      # release notes body
  versioning.py unreleased [--root R]         # body of [Unreleased]
  versioning.py current [--root R]
  versioning.py code X.Y.Z                    # Android versionCode
  versioning.py is-newer A B                  # exit 0 when A > B
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path

# Android's versionCode ceiling is 2_100_000_000; MAJOR*10000 + MINOR*100 + PATCH
# stays below it for MAJOR <= 209999.
MAX_MAJOR = 209_999
MAX_MINOR = 99
MAX_PATCH = 99
MAX_VERSION_CODE = MAX_MAJOR * 10_000 + MAX_MINOR * 100 + MAX_PATCH

_SEMVER = re.compile(r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)")

WIN = "apps/flow-windows"
GRADLE = "apps/flow-android/app/build.gradle.kts"


def parse(version: str) -> tuple[int, int, int]:
    """``MAJOR.MINOR.PATCH`` (no pre-release, no build metadata, no ``v``)."""
    m = _SEMVER.fullmatch(version)
    if not m:
        raise ValueError(f"not a MAJOR.MINOR.PATCH version: {version!r}")
    major, minor, patch = (int(x) for x in m.groups())
    if major > MAX_MAJOR or minor > MAX_MINOR or patch > MAX_PATCH:
        raise ValueError(
            f"{version}: MINOR and PATCH must be <= 99 and MAJOR <= {MAX_MAJOR} (Android versionCode)"
        )
    return major, minor, patch


def version_code(version: str) -> int:
    major, minor, patch = parse(version)
    return major * 10_000 + minor * 100 + patch


def is_newer(a: str, b: str) -> bool:
    return parse(a) > parse(b)


def describe(version: str, sha: str, dirty: bool) -> tuple[str, str]:
    """``(build, full)``: ``("abc1234-dirty", "0.2.0+abc1234.dirty")``."""
    parse(version)
    if not sha:
        return "unknown", f"{version}+unknown"
    return (f"{sha}-dirty", f"{version}+{sha}.dirty") if dirty else (sha, f"{version}+{sha}")


# ---------------------------------------------------------------- reading


def read_version(root: Path) -> str:
    return (root / "VERSION").read_text().strip()


def _section_span(text: str, header: str) -> tuple[int, int] | None:
    m = re.search(r"^" + re.escape(header) + r"[ \t]*$", text, re.M)
    if not m:
        return None
    nxt = re.search(r"^\[", text[m.end():], re.M)
    return m.end(), (m.end() + nxt.start()) if nxt else len(text)


_TOML_VERSION = re.compile(r'^(version\s*=\s*")([^"]*)(")', re.M)


def _toml_section_version(text: str, header: str) -> str | None:
    span = _section_span(text, header)
    if not span:
        return None
    m = _TOML_VERSION.search(text, span[0], span[1])
    return m.group(2) if m else None


def _set_toml_section_version(text: str, header: str, new: str) -> str:
    span = _section_span(text, header)
    if not span:
        raise ValueError(f"no {header} section")
    m = _TOML_VERSION.search(text, span[0], span[1])
    if not m:
        raise ValueError(f"no version in {header}")
    return text[: m.start(2)] + new + text[m.end(2):]


def _lock_blocks(text: str):
    """(start, end, name, version_span) of each path package named flow-*."""
    starts = [m.start() for m in re.finditer(r"^\[\[package\]\]", text, re.M)] + [len(text)]
    for a, b in zip(starts, starts[1:]):
        block = text[a:b]
        name = re.search(r'^name = "([^"]*)"', block, re.M)
        ver = re.search(r'^version = "([^"]*)"', block, re.M)
        if not name or not ver or not name.group(1).startswith("flow-"):
            continue
        if re.search(r"^source = ", block, re.M):
            continue  # a registry/git package that happens to be called flow-*
        yield name.group(1), (a + ver.start(1), a + ver.end(1))


def collect(root: Path) -> list[tuple[str, str]]:
    """Every stamped copy of the product version: ``[(location, value)]``."""
    out: list[tuple[str, str]] = []
    if (root / "VERSION").is_file():
        out.append(("VERSION", read_version(root)))
    cargo = root / "Cargo.toml"
    if cargo.is_file():
        val = _toml_section_version(cargo.read_text(), "[workspace.package]")
        out.append(("Cargo.toml [workspace.package]", val or "<missing>"))
    wcargo = root / WIN / "Cargo.toml"
    if wcargo.is_file():
        val = _toml_section_version(wcargo.read_text(), "[package]")
        out.append((f"{WIN}/Cargo.toml", val or "<missing>"))
    for lock in ("Cargo.lock", f"{WIN}/Cargo.lock"):
        p = root / lock
        if p.is_file():
            text = p.read_text()
            for name, (s, e) in _lock_blocks(text):
                out.append((f"{lock} {name}", text[s:e]))
    pkg = root / WIN / "ui" / "package.json"
    if pkg.is_file():
        out.append((f"{WIN}/ui/package.json", json.loads(pkg.read_text()).get("version", "<missing>")))
    plock = root / WIN / "ui" / "package-lock.json"
    if plock.is_file():
        data = json.loads(plock.read_text())
        out.append((f"{WIN}/ui/package-lock.json", data.get("version", "<missing>")))
        out.append(
            (f'{WIN}/ui/package-lock.json packages[""]', data.get("packages", {}).get("", {}).get("version", "<missing>"))
        )
    return out


def check(root: Path, tag: str | None = None) -> list[str]:
    """Problems with the version wiring; empty = consistent."""
    errors: list[str] = []
    if not (root / "VERSION").is_file():
        return ["VERSION: missing (the single source of the product version)"]
    version = read_version(root)
    try:
        parse(version)
    except ValueError as e:
        errors.append(f"VERSION: {e}")
    for where, val in collect(root):
        if where != "VERSION" and val != version:
            errors.append(f"{where}: {val} != VERSION {version} (run tools/release.sh, never edit copies by hand)")
    tauri = root / WIN / "tauri.conf.json"
    if tauri.is_file() and "version" in json.loads(tauri.read_text()):
        errors.append(f"{WIN}/tauri.conf.json: has a \"version\"; drop it so Tauri takes Cargo.toml's")
    gradle = root / GRADLE
    if gradle.is_file():
        g = gradle.read_text()
        if re.search(r'^\s*versionName\s*=\s*"', g, re.M) or re.search(r"^\s*versionCode\s*=\s*[0-9]", g, re.M):
            errors.append(f"{GRADLE}: literal versionName/versionCode; derive both from VERSION")
        if "VERSION" not in g:
            errors.append(f"{GRADLE}: does not read the VERSION file")
    if tag is not None:
        t = tag.removeprefix("refs/tags/")
        if t != f"v{version}":
            errors.append(f"tag {t} does not match VERSION {version} (expected v{version})")
        cl = root / "CHANGELOG.md"
        if not cl.is_file() or changelog_section(cl.read_text(), version) is None:
            errors.append(f"CHANGELOG.md: no '## [{version}] - <date>' section for tag {t}")
    return errors


# ---------------------------------------------------------------- writing


def stamp(root: Path, new: str) -> None:
    """Rewrite VERSION and every copy in :func:`collect` to ``new``."""
    parse(new)
    (root / "VERSION").write_text(new + "\n")
    cargo = root / "Cargo.toml"
    cargo.write_text(_set_toml_section_version(cargo.read_text(), "[workspace.package]", new))
    wcargo = root / WIN / "Cargo.toml"
    if wcargo.is_file():
        wcargo.write_text(_set_toml_section_version(wcargo.read_text(), "[package]", new))
    for lock in ("Cargo.lock", f"{WIN}/Cargo.lock"):
        p = root / lock
        if not p.is_file():
            continue
        text = p.read_text()
        for _, (s, e) in sorted(_lock_blocks(text), key=lambda x: -x[1][0]):
            text = text[:s] + new + text[e:]
        p.write_text(text)
    for name in ("package.json", "package-lock.json"):
        p = root / WIN / "ui" / name
        if not p.is_file():
            continue
        data = json.loads(p.read_text())
        data["version"] = new
        if name == "package-lock.json" and "" in data.get("packages", {}):
            data["packages"][""]["version"] = new
        p.write_text(json.dumps(data, indent=2, ensure_ascii=False) + "\n")


# ---------------------------------------------------------------- changelog

_HEADING = re.compile(r"^## \[([^\]]+)\][^\n]*$", re.M)
_LINK = re.compile(r"^\[([^\]]+)\]: (\S+)[ \t]*$", re.M)


def _body_end(text: str, start: int) -> int:
    ends = [m.start() for m in (_HEADING.search(text, start), _LINK.search(text, start)) if m]
    return min(ends) if ends else len(text)


def _find_heading(text: str, name: str):
    for m in _HEADING.finditer(text):
        if m.group(1) == name:
            return m
    return None


def changelog_section(text: str, version: str) -> str | None:
    m = _find_heading(text, version)
    if not m or not re.match(r"^## \[[^\]]+\] - \S", m.group(0)):
        return None
    return text[m.end(): _body_end(text, m.end())].strip()


def changelog_unreleased(text: str) -> str | None:
    m = _find_heading(text, "Unreleased")
    return None if not m else text[m.end(): _body_end(text, m.end())].strip()


def changelog_release(text: str, version: str, date: str, repo_url: str | None = None) -> str:
    """Move ``[Unreleased]`` into ``## [version] - date`` (Keep a Changelog)."""
    parse(version)
    if _find_heading(text, version):
        raise ValueError(f"CHANGELOG.md already has a [{version}] section")
    m = _find_heading(text, "Unreleased")
    if not m:
        raise ValueError("CHANGELOG.md has no '## [Unreleased]' heading")
    end = _body_end(text, m.end())
    body = text[m.end(): end].strip()
    if not body:
        raise ValueError("CHANGELOG.md [Unreleased] is empty: write the release notes first")
    rest = text[end:]
    prev = next((h.group(1) for h in _HEADING.finditer(rest) if h.group(1) != "Unreleased"), None)
    out = text[: m.start()] + f"## [Unreleased]\n\n## [{version}] - {date}\n\n{body}\n" + ("\n" + rest if rest else "")

    base = None
    link = next((lm for lm in _LINK.finditer(out) if lm.group(1) == "Unreleased"), None)
    if link:
        cm = re.match(r"(.*)/compare/\S+\.\.\.HEAD$", link.group(2))
        base = cm.group(1) if cm else None
    base = base or (repo_url.rstrip("/") if repo_url else None)
    if not base:
        return out
    new_unrel = f"[Unreleased]: {base}/compare/v{version}...HEAD"
    new_ver = f"[{version}]: {base}/compare/v{prev}...v{version}" if prev else f"[{version}]: {base}/releases/tag/v{version}"
    if link:
        return out[: link.start()] + new_unrel + "\n" + new_ver + out[link.end():]
    return out.rstrip("\n") + "\n\n" + new_unrel + "\n" + new_ver + "\n"


# ---------------------------------------------------------------- CLI


def main(argv: list[str]) -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    sub = ap.add_subparsers(dest="cmd", required=True)
    root_default = Path(__file__).resolve().parent.parent.parent

    def with_root(p):
        p.add_argument("--root", type=Path, default=root_default)
        return p

    c = with_root(sub.add_parser("check"))
    c.add_argument("--tag")
    s = with_root(sub.add_parser("stamp"))
    s.add_argument("version")
    r = with_root(sub.add_parser("changelog-release"))
    r.add_argument("version")
    r.add_argument("date")
    r.add_argument("--repo-url")
    sec = with_root(sub.add_parser("section"))
    sec.add_argument("version")
    with_root(sub.add_parser("unreleased"))
    with_root(sub.add_parser("current"))
    code = sub.add_parser("code")
    code.add_argument("version")
    nw = sub.add_parser("is-newer")
    nw.add_argument("a")
    nw.add_argument("b")
    a = ap.parse_args(argv)
    try:
        if a.cmd == "check":
            errors = check(a.root, a.tag)
            for where, val in collect(a.root):
                print(f"  {val:<10} {where}")
            for e in errors:
                print(f"ERROR {e}", file=sys.stderr)
            if errors:
                return 1
            print(f"ok    product version {read_version(a.root)} (versionCode {version_code(read_version(a.root))})")
            return 0
        if a.cmd == "stamp":
            stamp(a.root, a.version)
            return 0
        if a.cmd == "changelog-release":
            p = a.root / "CHANGELOG.md"
            p.write_text(changelog_release(p.read_text(), a.version, a.date, a.repo_url))
            return 0
        if a.cmd == "section":
            body = changelog_section((a.root / "CHANGELOG.md").read_text(), a.version)
            if body is None:
                print(f"no [{a.version}] section", file=sys.stderr)
                return 1
            print(body)
            return 0
        if a.cmd == "unreleased":
            print(changelog_unreleased((a.root / "CHANGELOG.md").read_text()) or "")
            return 0
        if a.cmd == "current":
            print(read_version(a.root))
            return 0
        if a.cmd == "code":
            print(version_code(a.version))
            return 0
        if a.cmd == "is-newer":
            return 0 if is_newer(a.a, a.b) else 1
    except ValueError as e:
        print(f"error: {e}", file=sys.stderr)
        return 2
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
