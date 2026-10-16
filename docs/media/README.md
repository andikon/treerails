# README screenshots and demo

These assets show Tree Rail's actual Ratatui renderer. The capture example creates disposable local repositories, drives the application's keyboard handlers, and executes real Git operations: fetch, worktree creation from `origin/develop`, status/diff queries, staging, and a commit. It does not access user projects, contact GitHub, or launch Copilot or an editor.

Temporary repository paths are normalized to `C:\Code` before rendering. A terminal frame, scene captions, and keystroke labels are added around the captured interface. The media is a scripted UI capture, rather than a recording of a user's desktop or an external agent session.

## Included assets

| Asset | Content |
| --- | --- |
| `workspace.png` | Parallel worktrees, added/modified/deleted files, and an unstaged diff |
| `projects.png` | Project discovery |
| `create-worktree.png` | Branch, base, directory, and optional fetch settings |
| `staged-diff.png` | Staged changes after a real `git add` |
| `commit.png` | Subject and multiline commit body |
| `launch-profiles.png` | Copilot, editor, and terminal profile selection |
| `demo.gif` | Inline animated walkthrough, approximately 24 seconds |
| `demo.mp4` | Higher-resolution H.264 walkthrough with playback controls |

## Regenerate

Install the application build prerequisites, Python with Pillow, FFmpeg, and a monospace font. Then run from the repository root:

```text
cargo run --locked --example capture_demo -- target/readme-frames.json
python scripts/render_readme_media.py
```

Capture data and intermediate PNG frames remain under the ignored `target/` directory. Final assets are written here. The renderer uses argument arrays for FFmpeg and does not invoke a shell.

Optional settings:

```text
python scripts/render_readme_media.py --font /path/to/monospace.ttf
python scripts/render_readme_media.py --ffmpeg /path/to/ffmpeg
python scripts/render_readme_media.py --stills-only
```

Automatic font detection checks Consolas on Windows, DejaVu Sans Mono on Linux, and Menlo on macOS. Matching bold fonts are used where available. Screenshots use the application's default colors interpreted through a dark terminal palette; terminal colors and font rendering can differ on a user's machine.
