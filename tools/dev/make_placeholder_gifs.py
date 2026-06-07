#!/usr/bin/env python3

from __future__ import annotations

import base64
import shutil
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
MEDIA = ROOT / "media"
SIZE = (900, 480)
GIFS = [
    "hero",
    "agent-task-planner",
    "chat-modes",
    "code-completion",
    "auto-apply",
    "browser-tool",
    "memory",
    "mcp-skills",
    "any-device",
]
FALLBACK_GIF = base64.b64decode(
    "R0lGODlhAQABAIAAAAAAAP///ywAAAAAAQABAAACAUwAOw=="
)


def write_with_pillow(path: Path, stem: str) -> bool:
    try:
        from PIL import Image, ImageDraw, ImageFont
    except ImportError:
        return False

    image = Image.new("RGB", SIZE, "#111827")
    draw = ImageDraw.Draw(image)
    font = ImageFont.load_default()
    lines = [stem + ".gif", "PLACEHOLDER — replace with real recording"]
    y = 190
    for line in lines:
        box = draw.textbbox((0, 0), line, font=font)
        x = (SIZE[0] - (box[2] - box[0])) // 2
        draw.text((x, y), line, fill="#F9FAFB", font=font)
        y += 48
    image.save(path, format="GIF", save_all=True, loop=0, duration=1200)
    return True


def write_with_imagemagick(path: Path, stem: str) -> bool:
    binary = shutil.which("magick") or shutil.which("convert")
    if not binary:
        return False

    label = f"{stem}.gif\nPLACEHOLDER — replace with real recording"
    if Path(binary).name == "magick":
        command = [
            binary,
            "-size",
            f"{SIZE[0]}x{SIZE[1]}",
            "xc:#111827",
            "-gravity",
            "center",
            "-fill",
            "#F9FAFB",
            "-pointsize",
            "32",
            "-annotate",
            "0",
            label,
            str(path),
        ]
    else:
        command = [
            binary,
            "-size",
            f"{SIZE[0]}x{SIZE[1]}",
            "xc:#111827",
            "-gravity",
            "center",
            "-fill",
            "#F9FAFB",
            "-pointsize",
            "32",
            "-annotate",
            "0",
            label,
            str(path),
        ]
    try:
        subprocess.run(command, check=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        return True
    except (OSError, subprocess.CalledProcessError):
        return False


def write_fallback(path: Path) -> None:
    path.write_bytes(FALLBACK_GIF)


def main() -> None:
    MEDIA.mkdir(parents=True, exist_ok=True)
    for stem in GIFS:
        path = MEDIA / f"{stem}.gif"
        if write_with_pillow(path, stem):
            continue
        if write_with_imagemagick(path, stem):
            continue
        write_fallback(path)
        print(f"{path.relative_to(ROOT)}: wrote minimal fallback GIF")
    print(f"wrote {len(GIFS)} placeholder GIFs to {MEDIA.relative_to(ROOT)}")


if __name__ == "__main__":
    main()
