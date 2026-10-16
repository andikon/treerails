"""Check the download contract independently of the GitHub runner."""

import hashlib
from pathlib import Path
import tarfile
import tempfile
import unittest
import zipfile

import release


class ReleaseTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        (self.root / "docs").mkdir()
        (self.root / "examples").mkdir()
        (self.root / "docs/usage.md").write_text("Usage", encoding="utf-8")
        (self.root / "examples/config.toml").write_text("# Config", encoding="utf-8")
        self.binary = self.root / "binary"
        self.binary.write_bytes(b"executable contents")
        self.dist = self.root / "dist"
        self.set_version("1.2.3")

    def set_version(self, version, lock_version=None):
        (self.root / "Cargo.toml").write_text(
            f'[package]\nname = "treerail"\nversion = "{version}"\n', encoding="utf-8"
        )
        (self.root / "Cargo.lock").write_text(
            f'[[package]]\nname = "treerail"\nversion = "{lock_version or version}"\n', encoding="utf-8"
        )

    def test_rejects_invalid_tags_and_mismatched_versions(self):
        for tag in ["1.2.3", "v1.2", "v01.2.3", "v1.2.4", "v1.2.3\ninjected"]:
            with self.subTest(tag=tag), self.assertRaises(ValueError):
                release.validate(tag, self.root)
        self.set_version("1.2.3", "1.2.2")
        with self.assertRaises(ValueError):
            release.validate("v1.2.3", self.root)

    def test_prerelease_classification(self):
        self.assertEqual(release.validate("v1.2.3", self.root)["prerelease"], "false")
        self.set_version("1.2.3-rc.1")
        self.assertEqual(release.validate("v1.2.3-rc.1", self.root)["prerelease"], "true")
        self.set_version("1.2.3-01")
        with self.assertRaises(ValueError):
            release.validate("v1.2.3-01", self.root)

    def test_download_layout_binary_contents_and_unix_permissions(self):
        for target in release.TARGETS:
            archive = release.package(target, self.binary, self.dist, self.root)
            executable = "treerail.exe" if archive.suffix == ".zip" else "treerail"
            expected = {executable, "README.txt", "usage.md", "config.example.toml"}
            if archive.suffix == ".zip":
                with zipfile.ZipFile(archive) as source:
                    self.assertEqual(set(source.namelist()), expected)
                    self.assertEqual(source.read(executable), self.binary.read_bytes())
                self.assertEqual((self.dist / release.WINDOWS_EXE).read_bytes(), self.binary.read_bytes())
            else:
                with tarfile.open(archive) as source:
                    self.assertEqual(set(source.getnames()), expected)
                    self.assertEqual(source.extractfile(executable).read(), self.binary.read_bytes())
                    self.assertEqual(source.getmember(executable).mode & 0o777, 0o755)

    def test_checksums_require_complete_nonempty_downloads(self):
        self.dist.mkdir()
        with self.assertRaises(ValueError):
            release.checksums(self.dist)
        names = set(release.TARGETS.values()) | {release.WINDOWS_EXE}
        for name in names:
            (self.dist / name).write_bytes(name.encode())
        release.checksums(self.dist)
        for line in (self.dist / "SHA256SUMS").read_text().splitlines():
            digest, name = line.split("  ")
            self.assertEqual(digest, hashlib.sha256((self.dist / name).read_bytes()).hexdigest())
        (self.dist / release.WINDOWS_EXE).write_bytes(b"")
        with self.assertRaises(ValueError):
            release.checksums(self.dist)


if __name__ == "__main__":
    unittest.main()
