"""Build release archives using only Python 3.11+ standard-library modules."""

import argparse
import hashlib
import os
from pathlib import Path
import re
import shutil
import subprocess
import tarfile
import tempfile
import tomllib
import zipfile


ROOT = Path(__file__).resolve().parent.parent
TARGETS = {
    "x86_64-pc-windows-msvc": "treerail-windows-x86_64.zip",
    "x86_64-unknown-linux-musl": "treerail-linux-x86_64.tar.gz",
    "aarch64-apple-darwin": "treerail-macos-aarch64.tar.gz",
    "x86_64-apple-darwin": "treerail-macos-x86_64.tar.gz",
}
WINDOWS_EXE = "treerail-windows-x86_64.exe"
SEMVER = re.compile(
    r"v(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)"
    r"(?:-((?:0|[1-9]\d*|\d*[A-Za-z-][0-9A-Za-z-]*)"
    r"(?:\.(?:0|[1-9]\d*|\d*[A-Za-z-][0-9A-Za-z-]*))*))?"
    r"(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?"
)


def version(root=ROOT):
    return tomllib.loads((root / "Cargo.toml").read_text(encoding="utf-8"))["package"]["version"]


def validate(tag, root=ROOT):
    match = SEMVER.fullmatch(tag)
    if not match or tag != f"v{version(root)}":
        raise ValueError("The tag must be v<package.version>, for example v0.1.0 or v0.2.0-rc.1")
    lock = tomllib.loads((root / "Cargo.lock").read_text(encoding="utf-8"))
    if not any(p["name"] == "treerail" and p["version"] == tag[1:] for p in lock["package"]):
        raise ValueError("Cargo.lock does not match the package version; run cargo check first")
    return {"tag": tag, "prerelease": str(match.group(4) is not None).lower()}


def package(target, binary, destination, root=ROOT):
    """Create an archive with a flat layout, preserving Unix executable permissions."""
    windows = target.endswith("windows-msvc")
    binary = binary.resolve(strict=True)
    destination.mkdir(parents=True, exist_ok=True)
    archive = destination / TARGETS[target]
    with tempfile.TemporaryDirectory(prefix="treerail-package-") as temporary:
        staging = Path(temporary)
        executable = staging / ("treerail.exe" if windows else "treerail")
        shutil.copyfile(binary, executable)
        executable.chmod(0o755)
        shutil.copyfile(root / "docs/usage.md", staging / "usage.md")
        shutil.copyfile(root / "examples/config.toml", staging / "config.example.toml")
        command = r".\treerail.exe 'C:\Code'" if windows else "./treerail ~/code"
        (staging / "README.txt").write_text(
            f"Tree Rail {version(root)}\n\n"
            f"Open a terminal in this directory and run:\n  {command}\n\n"
            "Git must be on PATH. No Rust installation is required.\n"
            "Add this directory to PATH to run treerail from anywhere.\n"
            "The compatible Copilot runtime is bundled. Copilot access requires GitHub authentication.\n"
            "Sign in with GitHub CLI or provide COPILOT_GITHUB_TOKEN for Copilot conversations.\n"
            "The right pane runs the real Copilot CLI. Ctrl+] returns to Tree Rail navigation.\n"
            "See usage.md and config.example.toml for configuration and keyboard controls.\n"
            "Project: https://github.com/andikon/treerails\n",
            encoding="utf-8",
        )
        if windows:
            with zipfile.ZipFile(archive, "w", compression=zipfile.ZIP_DEFLATED) as output:
                for entry in sorted(staging.iterdir()):
                    output.write(entry, entry.name)
            shutil.copyfile(binary, destination / WINDOWS_EXE)
        else:
            with tarfile.open(archive, "w:gz") as output:
                for entry in sorted(staging.iterdir()):
                    info = output.gettarinfo(str(entry), arcname=entry.name)
                    info.mode = 0o755 if entry == executable else 0o644
                    with entry.open("rb") as contents:
                        output.addfile(info, contents)
    return archive


def smoke(archive, target, expected_version):
    with tempfile.TemporaryDirectory(prefix="treerail-smoke-") as temporary:
        extracted = Path(temporary)
        if archive.suffix == ".zip":
            with zipfile.ZipFile(archive) as source:
                source.extractall(extracted)
        else:
            with tarfile.open(archive) as source:
                source.extractall(extracted, filter="data")
        executable = extracted / ("treerail.exe" if target.endswith("windows-msvc") else "treerail")
        result = subprocess.run([str(executable), "--version"], check=True, capture_output=True, text=True)
        if result.stdout.strip() != f"treerail {expected_version}":
            raise ValueError(f"Packaged version differs from Cargo.toml: {result.stdout!r}")
        subprocess.run([str(executable), "--help"], check=True, capture_output=True)


def checksums(directory):
    expected = set(TARGETS.values()) | {WINDOWS_EXE}
    actual = {p.name for p in directory.iterdir() if p.name != "SHA256SUMS"}
    if actual != expected:
        raise ValueError(f"Release assets differ: missing={expected - actual}, unexpected={actual - expected}")
    lines = []
    for name in sorted(expected):
        path = directory / name
        if not path.is_file() or path.stat().st_size == 0:
            raise ValueError(f"Empty or invalid release asset: {name}")
        lines.append(f"{hashlib.sha256(path.read_bytes()).hexdigest()}  {name}\n")
    (directory / "SHA256SUMS").write_text("".join(lines), encoding="utf-8", newline="\n")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    validation = commands.add_parser("validate")
    validation.add_argument("--tag", required=True)
    packaging = commands.add_parser("package")
    packaging.add_argument("--target", required=True, choices=TARGETS)
    packaging.add_argument("--binary", type=Path)
    packaging.add_argument("--directory", type=Path, default=ROOT / "dist")
    sums = commands.add_parser("checksums")
    sums.add_argument("--directory", type=Path, default=ROOT / "dist")
    args = parser.parse_args()
    if args.command == "validate":
        outputs = validate(args.tag)
        if "GITHUB_OUTPUT" in os.environ:
            with open(os.environ["GITHUB_OUTPUT"], "a", encoding="utf-8") as output:
                output.writelines(f"{key}={value}\n" for key, value in outputs.items())
        print(outputs)
    elif args.command == "package":
        executable = "treerail.exe" if args.target.endswith("windows-msvc") else "treerail"
        binary = args.binary or ROOT / "target" / args.target / "release" / executable
        archive = package(args.target, binary, args.directory)
        smoke(archive, args.target, version())
        print(f"Packaged and verified {archive.name}")
    else:
        checksums(args.directory)
        print("All platform downloads present; SHA256SUMS written")


if __name__ == "__main__":
    main()
