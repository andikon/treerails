"""Render captured Ratatui cells into README screenshots, GIF, and MP4.

Requires Pillow and FFmpeg. Input comes from `cargo run --example capture_demo`.
The terminal content is the actual application UI; only the surrounding frame,
scene captions, and keystroke labels are added here.
"""
from __future__ import annotations

import argparse
import json
from pathlib import Path
import shutil
import subprocess

from PIL import Image, ImageDraw, ImageFont

BACKGROUND = "#0b111c"
SURFACE = "#101a29"
TEXT = "#d9e3ee"
MUTED = "#8d9db2"
ACCENT = "#7dd3fc"
CELL_WIDTH, CELL_HEIGHT = 11, 24


def find_font(explicit: str | None, candidates: list[str]) -> Path:
    if explicit:
        path = Path(explicit)
        if not path.is_file():
            raise SystemExit(f"Font not found: {path}")
        return path
    for candidate in candidates:
        if Path(candidate).is_file():
            return Path(candidate)
    raise SystemExit("A monospace font is required; supply --font /path/to/font.ttf")


def render(frame: dict, columns: int, rows: int, mono: Path) -> Image.Image:
    width = columns * CELL_WIDTH + 80
    height = rows * CELL_HEIGHT + 284
    image = Image.new("RGB", (width, height), BACKGROUND)
    draw = ImageDraw.Draw(image)
    bold_path = mono.with_name("consolab.ttf")
    if not bold_path.is_file():
        bold_path = mono.with_name("DejaVuSansMono-Bold.ttf")
    font = ImageFont.truetype(str(mono), 20)
    bold = ImageFont.truetype(str(bold_path if bold_path.is_file() else mono), 20)
    label = ImageFont.truetype(str(mono), 15)
    title = ImageFont.truetype(str(bold_path if bold_path.is_file() else mono), 28)
    detail = ImageFont.truetype(str(mono), 18)

    draw.text((32, 20), "TREE RAIL  /  PARALLEL WORKSPACES", font=label, fill=ACCENT)
    draw.text((32, 49), frame["title"], font=title, fill="#f0f4fa")
    draw.text((32, 89), frame["detail"], font=detail, fill=MUTED)

    left, top = 24, 133
    right, bottom = width - 24, top + rows * CELL_HEIGHT + 64
    draw.rounded_rectangle((left + 3, top + 6, right + 3, bottom + 6), radius=13, fill="#05090f")
    draw.rounded_rectangle((left, top, right, bottom), radius=13, fill=SURFACE, outline="#2b3a4e", width=1)
    draw.line((left, top + 40, right, top + 40), fill="#2b3a4e")
    for index, color in enumerate(["#fda4af", "#fde68a", "#86efac"]):
        x = left + 19 + index * 19
        draw.ellipse((x, top + 16, x + 8, top + 24), fill=color)
    draw.text((left + 92, top + 10), "treerail  ·  demo workspace", font=label, fill=MUTED)
    origin_x, origin_y = 40, top + 51
    draw.rectangle((origin_x, origin_y, origin_x + columns * CELL_WIDTH - 1,
                    origin_y + rows * CELL_HEIGHT - 1), fill=BACKGROUND)
    # All cell backgrounds first, then glyphs: this preserves symbols that
    # occupy more than one terminal cell and font antialiasing at cell edges.
    for index, cell in enumerate(frame["cells"]):
        x = origin_x + index % columns * CELL_WIDTH
        y = origin_y + index // columns * CELL_HEIGHT
        if cell["bg"] is not None:
            draw.rectangle((x, y, x + CELL_WIDTH - 1, y + CELL_HEIGHT - 1), fill=tuple(cell["bg"]))
    for index, cell in enumerate(frame["cells"]):
        if not cell["text"].strip():
            continue
        x = origin_x + index % columns * CELL_WIDTH
        y = origin_y + index // columns * CELL_HEIGHT
        draw.text((x, y), cell["text"], font=bold if cell["bold"] else font,
                  fill=tuple(cell["fg"]) if cell["fg"] is not None else TEXT)

    key_y = bottom + 27
    key_width = int(draw.textlength(frame["keys"], font=detail)) + 30
    draw.rounded_rectangle((32, key_y, 32 + key_width, key_y + 38), radius=7,
                           fill="#172a3c", outline="#30506b")
    draw.text((47, key_y + 7), frame["keys"], font=detail, fill=ACCENT)
    return image


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("capture", nargs="?", default="target/readme-frames.json")
    parser.add_argument("--output", default="docs/media")
    parser.add_argument("--font")
    parser.add_argument("--ffmpeg", default="ffmpeg")
    parser.add_argument("--stills-only", action="store_true")
    args = parser.parse_args()
    mono = find_font(args.font, [r"C:\Windows\Fonts\consola.ttf",
                               "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf",
                               "/System/Library/Fonts/Menlo.ttc"])
    data = json.loads(Path(args.capture).read_text(encoding="utf-8"))
    output = Path(args.output).resolve()
    output.mkdir(parents=True, exist_ok=True)
    working = Path("target/readme-media").resolve()
    working.mkdir(parents=True, exist_ok=True)
    timeline = ["ffconcat version 1.0"]
    for index, frame in enumerate(data["frames"]):
        image = render(frame, data["columns"], data["rows"], mono)
        path = working / f"frame-{index:03d}.png"
        image.save(path, optimize=True)
        if frame["still"]:
            image.save(output / f"{frame['still']}.png", optimize=True)
        escaped = path.as_posix().replace("'", "'\\''")
        timeline.extend([f"file '{escaped}'", f"duration {frame['duration_ms'] / 1000:.3f}"])
    timeline.append(timeline[-2])
    sequence = working / "timeline.ffconcat"
    sequence.write_text("\n".join(timeline) + "\n", encoding="utf-8")
    seconds = sum(frame["duration_ms"] for frame in data["frames"]) / 1000
    if not args.stills_only:
        ffmpeg = shutil.which(args.ffmpeg)
        if not ffmpeg:
            raise SystemExit("FFmpeg not found; use --ffmpeg or --stills-only")
        common = [ffmpeg, "-hide_banner", "-loglevel", "warning", "-y", "-safe", "0", "-i", str(sequence), "-t", str(seconds)]
        subprocess.run(common + ["-vf", "fps=24,format=yuv420p", "-c:v", "libx264", "-preset", "slow",
                                 "-crf", "20", "-movflags", "+faststart", "-an", str(output / "demo.mp4")], check=True)
        subprocess.run(common + ["-vf", "fps=8,scale=1280:-1:flags=lanczos,split[a][b];[a]palettegen=stats_mode=diff[p];[b][p]paletteuse=dither=bayer:bayer_scale=3",
                                 "-loop", "0", str(output / "demo.gif")], check=True)
    print(f"Rendered {len(data['frames'])} real UI frames ({seconds:.1f}s) to {output}")


if __name__ == "__main__":
    main()
