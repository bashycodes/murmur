"""Tests for tools/versioning/versioning.py (docs/versioning.md).

Run: python3 -m unittest discover -s tools/versioning   (from the repo root)
"""

import json
import os
import shutil
import sys
import tempfile
import unittest
from pathlib import Path

import versioning as v

HERE = Path(__file__).resolve().parent
VECTORS = json.loads((HERE / "fixtures" / "version_vectors.json").read_text())

CHANGELOG = """# Changelog

Intro.

## [Unreleased]

### Added

- Thing one.

## [0.0.9] - 2026-01-01

### Added

- Old.

[Unreleased]: https://example.invalid/compare/v0.0.9...HEAD
[0.0.9]: https://example.invalid/releases/tag/v0.0.9
"""


def make_repo(root: Path, version: str = "0.1.0") -> None:
    """A minimal tree with every file the version rules look at."""
    (root / "VERSION").write_text(version + "\n")
    (root / "Cargo.toml").write_text(
        '[workspace]\nmembers = ["crates/a"]\n\n[workspace.package]\nedition = "2021"\n'
        f'version = "{version}"\nlicense = "MIT"\n\n[workspace.dependencies]\nserde = {{ version = "1" }}\n'
    )
    lock = (
        'version = 4\n\n[[package]]\nname = "serde"\nversion = "1.0.0"\n'
        'source = "registry+https://github.com/rust-lang/crates.io-index"\n\n'
        f'[[package]]\nname = "flow-core"\nversion = "{version}"\ndependencies = [\n "serde",\n]\n\n'
        f'[[package]]\nname = "flow-server"\nversion = "{version}"\n'
    )
    (root / "Cargo.lock").write_text(lock)
    w = root / "apps" / "flow-windows"
    (w / "ui").mkdir(parents=True, exist_ok=True)
    (w / "Cargo.toml").write_text(
        f'[package]\nname = "flow-windows"\nversion = "{version}"\nedition = "2021"\n\n[dependencies]\nx = {{ version = "0.1.0" }}\n'
    )
    (w / "Cargo.lock").write_text(
        lock + f'\n[[package]]\nname = "flow-windows"\nversion = "{version}"\n'
        '\n[[package]]\nname = "windows-strings"\nversion = "0.1.0"\nsource = "registry+https://github.com/rust-lang/crates.io-index"\n'
    )
    (w / "tauri.conf.json").write_text('{\n  "productName": "Murmur",\n  "identifier": "dev.flowclone.windows"\n}\n')
    (w / "ui" / "package.json").write_text(
        json.dumps({"name": "flow-windows-ui", "private": True, "version": version}, indent=2) + "\n"
    )
    (w / "ui" / "package-lock.json").write_text(
        json.dumps(
            {
                "name": "flow-windows-ui",
                "version": version,
                "lockfileVersion": 3,
                "packages": {"": {"name": "flow-windows-ui", "version": version}, "node_modules/typescript": {"version": "5.6.3"}},
            },
            indent=2,
        )
        + "\n"
    )
    a = root / "apps" / "flow-android" / "app"
    a.mkdir(parents=True, exist_ok=True)
    (a / "build.gradle.kts").write_text(
        'val flowVersion = file("../../../VERSION").readText().trim()\n'
        "android {\n    defaultConfig {\n        versionCode = flowVersionCode(flowVersion)\n        versionName = flowVersion\n    }\n}\n"
    )
    (root / "CHANGELOG.md").write_text(CHANGELOG)


class ParseTest(unittest.TestCase):
    def test_valid_vectors(self):
        for case in VECTORS["valid"]:
            with self.subTest(case["version"]):
                self.assertEqual(v.version_code(case["version"]), case["code"])

    def test_invalid_vectors(self):
        for bad in VECTORS["invalid"]:
            with self.subTest(repr(bad)):
                with self.assertRaises(ValueError):
                    v.parse(bad)

    def test_describe_vectors(self):
        for case in VECTORS["describe"]:
            with self.subTest(case):
                self.assertEqual(v.describe(case["version"], case["sha"], case["dirty"]), (case["build"], case["full"]))

    def test_ordering(self):
        self.assertTrue(v.is_newer("0.2.0", "0.1.0"))
        self.assertTrue(v.is_newer("0.10.0", "0.9.9"))
        self.assertTrue(v.is_newer("1.0.0", "0.99.99"))
        self.assertFalse(v.is_newer("0.1.0", "0.1.0"))
        self.assertFalse(v.is_newer("0.1.9", "0.2.0"))

    def test_codes_increase_with_the_version(self):
        ordered = sorted((c["version"] for c in VECTORS["valid"]), key=v.parse)
        codes = [v.version_code(x) for x in ordered]
        self.assertEqual(codes, sorted(codes))
        self.assertLessEqual(max(codes), v.MAX_VERSION_CODE)


class CheckTest(unittest.TestCase):
    def setUp(self):
        self.root = Path(tempfile.mkdtemp(prefix="versioning-test-"))
        make_repo(self.root)

    def tearDown(self):
        shutil.rmtree(self.root)

    def test_consistent_tree_passes(self):
        self.assertEqual(v.check(self.root), [])

    def test_every_copy_is_found(self):
        found = dict(v.collect(self.root))
        self.assertEqual(set(found.values()), {"0.1.0"})
        for key in [
            "VERSION",
            "Cargo.toml [workspace.package]",
            "Cargo.lock flow-core",
            "Cargo.lock flow-server",
            "apps/flow-windows/Cargo.toml",
            "apps/flow-windows/Cargo.lock flow-windows",
            "apps/flow-windows/Cargo.lock flow-core",
            "apps/flow-windows/ui/package.json",
            "apps/flow-windows/ui/package-lock.json",
            "apps/flow-windows/ui/package-lock.json packages[\"\"]",
        ]:
            self.assertIn(key, found)
        # registry packages are not ours
        self.assertNotIn("apps/flow-windows/Cargo.lock windows-strings", found)

    def test_each_drifted_copy_fails(self):
        edits = [
            ("Cargo.toml", 'version = "0.1.0"\nlicense', 'version = "0.1.1"\nlicense'),
            ("Cargo.lock", 'name = "flow-server"\nversion = "0.1.0"', 'name = "flow-server"\nversion = "0.0.1"'),
            ("apps/flow-windows/Cargo.toml", 'name = "flow-windows"\nversion = "0.1.0"', 'name = "flow-windows"\nversion = "0.3.0"'),
            ("apps/flow-windows/ui/package.json", '"version": "0.1.0"', '"version": "9.9.9"'),
        ]
        for path, old, new in edits:
            with self.subTest(path):
                make_repo(self.root)
                p = self.root / path
                p.write_text(p.read_text().replace(old, new, 1))
                errors = v.check(self.root)
                self.assertTrue(any(path in e for e in errors), errors)

    def test_bad_version_file_fails(self):
        (self.root / "VERSION").write_text("0.1\n")
        self.assertTrue(any("VERSION" in e for e in v.check(self.root)))

    def test_literal_android_version_fails(self):
        g = self.root / "apps/flow-android/app/build.gradle.kts"
        g.write_text(g.read_text().replace("versionName = flowVersion", 'versionName = "0.1.0"'))
        self.assertTrue(any("build.gradle.kts" in e for e in v.check(self.root)))

    def test_tauri_version_must_not_be_pinned(self):
        t = self.root / "apps/flow-windows/tauri.conf.json"
        t.write_text('{"productName": "Murmur", "version": "0.1.0"}\n')
        self.assertTrue(any("tauri.conf.json" in e for e in v.check(self.root)))

    def test_tag_must_match_and_have_a_changelog_section(self):
        self.assertTrue(any("tag" in e for e in v.check(self.root, tag="v0.2.0")))
        # matches VERSION but CHANGELOG has no [0.1.0] section
        self.assertTrue(any("CHANGELOG" in e for e in v.check(self.root, tag="v0.1.0")))
        c = self.root / "CHANGELOG.md"
        c.write_text(c.read_text().replace("## [0.0.9]", "## [0.1.0] - 2026-02-02\n\n- x\n\n## [0.0.9]"))
        self.assertEqual(v.check(self.root, tag="v0.1.0"), [])
        self.assertEqual(v.check(self.root, tag="refs/tags/v0.1.0"), [])


class StampTest(unittest.TestCase):
    def setUp(self):
        self.root = Path(tempfile.mkdtemp(prefix="versioning-test-"))
        make_repo(self.root)

    def tearDown(self):
        shutil.rmtree(self.root)

    def test_stamp_moves_every_copy_and_only_ours(self):
        before_dep = (self.root / "apps/flow-windows/Cargo.toml").read_text().count('x = { version = "0.1.0" }')
        v.stamp(self.root, "0.2.0")
        self.assertEqual(v.check(self.root), [])
        self.assertEqual(set(dict(v.collect(self.root)).values()), {"0.2.0"})
        lock = (self.root / "apps/flow-windows/Cargo.lock").read_text()
        self.assertIn('name = "windows-strings"\nversion = "0.1.0"', lock)
        self.assertIn('name = "serde"\nversion = "1.0.0"', lock)
        self.assertEqual(
            (self.root / "apps/flow-windows/Cargo.toml").read_text().count('x = { version = "0.1.0" }'), before_dep
        )
        self.assertIn('serde = { version = "1" }', (self.root / "Cargo.toml").read_text())

    def test_stamp_rejects_an_invalid_version(self):
        with self.assertRaises(ValueError):
            v.stamp(self.root, "0.2")


class ChangelogTest(unittest.TestCase):
    def test_release_moves_unreleased_into_a_dated_section(self):
        out = v.changelog_release(CHANGELOG, "0.1.0", "2026-10-24")
        self.assertIn("## [Unreleased]\n\n## [0.1.0] - 2026-10-24\n\n### Added\n\n- Thing one.\n\n## [0.0.9]", out)
        self.assertIn("[Unreleased]: https://example.invalid/compare/v0.1.0...HEAD", out)
        self.assertIn("[0.1.0]: https://example.invalid/compare/v0.0.9...v0.1.0", out)
        self.assertIn("[0.0.9]: https://example.invalid/releases/tag/v0.0.9", out)
        self.assertEqual(v.changelog_section(out, "0.1.0"), "### Added\n\n- Thing one.")
        self.assertEqual(v.changelog_unreleased(out), "")

    def test_first_release_without_links(self):
        text = "# Changelog\n\n## [Unreleased]\n\n- a\n"
        out = v.changelog_release(text, "0.2.0", "2026-10-24")
        self.assertEqual(out, "# Changelog\n\n## [Unreleased]\n\n## [0.2.0] - 2026-10-24\n\n- a\n")

    def test_first_release_with_a_repo_url_adds_links(self):
        text = "# Changelog\n\n## [Unreleased]\n\n- a\n"
        out = v.changelog_release(text, "0.2.0", "2026-10-24", repo_url="https://example.invalid/r")
        self.assertIn("[Unreleased]: https://example.invalid/r/compare/v0.2.0...HEAD", out)
        self.assertIn("[0.2.0]: https://example.invalid/r/releases/tag/v0.2.0", out)

    def test_empty_unreleased_is_refused(self):
        with self.assertRaises(ValueError):
            v.changelog_release("# C\n\n## [Unreleased]\n\n## [0.0.9] - x\n", "0.1.0", "2026-10-24")

    def test_duplicate_version_is_refused(self):
        with self.assertRaises(ValueError):
            v.changelog_release(CHANGELOG, "0.0.9", "2026-10-24")


class EncodingTest(unittest.TestCase):
    """Windows runners default to cp1252: every file read/write must say utf-8 (v0.7.0 CI failure)."""

    def test_every_file_call_names_utf8(self):
        import ast

        src = (HERE / "versioning.py").read_text(encoding="utf-8")
        missing = [
            n.lineno
            for n in ast.walk(ast.parse(src))
            if isinstance(n, ast.Call)
            and isinstance(n.func, ast.Attribute)
            and n.func.attr in ("read_text", "write_text", "open")
            and not any(k.arg == "encoding" for k in n.keywords)
        ]
        self.assertEqual(missing, [], "calls without encoding=\"utf-8\" on these lines")

    def test_section_prints_non_ascii_through_a_cp1252_stdout(self):
        import subprocess

        with tempfile.TemporaryDirectory() as d:
            Path(d, "CHANGELOG.md").write_text("# Changelog\n\n## [1.0.0] - 2026-10-28\n\n- Bubble \u270e button \u2192 rewrite\n", encoding="utf-8")
            env = dict(os.environ, PYTHONIOENCODING="cp1252")
            out = subprocess.run(
                [sys.executable, str(HERE / "versioning.py"), "section", "1.0.0", "--root", d],
                capture_output=True, env=env, check=True,
            ).stdout.decode("utf-8")
        self.assertIn("\u270e button \u2192 rewrite", out)


if __name__ == "__main__":
    unittest.main()
