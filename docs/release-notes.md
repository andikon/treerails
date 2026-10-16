Download the build for your machine below. **No Rust installation is needed.**

| Machine | Download |
| --- | --- |
| Windows, Intel/AMD 64-bit | `treerail-windows-x86_64.exe` (standalone) or `treerail-windows-x86_64.zip` (with documentation) |
| Linux, Intel/AMD 64-bit | `treerail-linux-x86_64.tar.gz` (static musl build) |
| macOS, Apple Silicon | `treerail-macos-aarch64.tar.gz` |
| macOS, Intel | `treerail-macos-x86_64.tar.gz` |

For the standalone Windows download, rename the file to `treerail.exe`, put it in a folder on `PATH`, and run `treerail 'C:\Code'` in your terminal. For archives, extract first and run the executable inside. On Linux/macOS, use `./treerail ~/code`.

Git must be available on `PATH`. Copilot CLI, Neovim, and GitHub CLI are optional and used by the corresponding profiles/features. The archives include a usage guide and an example configuration. `SHA256SUMS` contains checksums for all five downloads. These binaries are not code-signed or notarized.

