# Publishing Tree Rail

The [Release workflow](../.github/workflows/release.yml) publishes prebuilt downloads to [GitHub Releases](https://github.com/andikon/treerails/releases). A pushed version tag triggers it automatically. Ordinary branch commits run validation without publishing a release.

## Publish a version

1. Update `package.version` in `Cargo.toml`, then run `cargo check` to refresh Tree Rail's version in `Cargo.lock`.
2. Commit and push the changes, including the release workflow for the first release. Wait for **Validate and build** to pass on GitHub.
3. Create and push a matching version tag. For the initial `0.1.0` release:

   ```powershell
   git tag -a v0.1.0 -m 'Tree Rail v0.1.0'
   git push origin v0.1.0
   ```

4. Watch **Actions → Release**. When every platform passes, the release appears with all downloads, SHA-256 checksums, installation instructions, and generated change notes.

Use a new version and tag for subsequent releases, for example `0.1.1` / `v0.1.1`. For a preview, use a Cargo version such as `0.2.0-rc.1` and tag `v0.2.0-rc.1`; it becomes a prerelease and does not replace the latest stable download. Stable releases use GitHub's automatic latest-release selection. Keep release tags on the main branch's history and publish versions in increasing order.

No extra secret is needed: only the publishing job receives `contents: write` on GitHub's automatic `GITHUB_TOKEN`. Repository or organization policies must allow GitHub Actions and that permission. Tag creation is the release decision; there is no additional manual publishing step.

## Downloads

| Asset | Target |
| --- | --- |
| `treerail-windows-x86_64.exe` | Standalone executable for 64-bit Intel/AMD Windows |
| `treerail-windows-x86_64.zip` | The same executable plus usage guide and example config |
| `treerail-linux-x86_64.tar.gz` | 64-bit Intel/AMD Linux, statically linked with musl |
| `treerail-macos-aarch64.tar.gz` | Apple Silicon Macs |
| `treerail-macos-x86_64.tar.gz` | Intel Macs |
| `SHA256SUMS` | SHA-256 digests of the five downloads above |

Asset names remain constant across releases, so README download links always point to the latest stable version. Each tagged release keeps its own downloads. GitHub's automatic source ZIP/TAR files contain source code; choose one of the explicitly named builds above to run Tree Rail without Rust.

Builds run formatting, Clippy, Rust tests, and release-packaging tests on native Windows, Linux, Apple Silicon, and Intel Mac runners. The downloaded Linux binary uses musl to avoid depending on a particular glibc version. Each archive is extracted on its runner and its executable is checked with `--version` and `--help` before upload. These checks do not exercise interactive terminal launchers or authenticated external tools.

## Retry and diagnose

If a job fails, GitHub does not publish the release. Fix a code problem with a new version/tag; for a transient runner/network failure, use **Re-run failed jobs**. You can also select **Actions → Release → Run workflow**, enter an existing version tag, and run the full pipeline again. The tagged commit must contain the release scripts. The workflow validates the tag against both Cargo files and builds every platform from the same resolved commit.

The publisher refuses to overwrite an existing release. A failed upload can leave a draft release; inspect that draft before retrying. If no published release exists, remove the incomplete draft through GitHub's release UI and rerun. Do not move published tags or replace published downloads; issue a new version instead.

## Check packaging locally

With Python 3.11+ and an existing Windows release build:

```powershell
python -m unittest discover -s scripts -p 'test_*.py' -v
python scripts/release.py validate --tag v0.1.0
cargo build --release --locked
python scripts/release.py package --target x86_64-pc-windows-msvc --binary target/release/treerail.exe
```

The output goes into ignored `dist/`. Other platforms use the corresponding Rust target and executable path. The `checksums` command intentionally requires the complete set of release downloads.
