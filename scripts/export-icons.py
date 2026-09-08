#!/usr/bin/env python3
"""Regenerate preview PNGs and the multi-resolution Windows ICO from SVG on macOS."""
from pathlib import Path
import struct
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]


def render(source, output, size):
    subprocess.run([
        "swift", "-module-cache-path", str(ROOT / "target/swift-module-cache"),
        str(ROOT / "scripts/render-logo.swift"), str(source), str(output), str(size),
    ], check=True)


def main():
    assets = ROOT / "assets"
    render(assets / "logo.svg", assets / "logo.png", 1024)
    render(assets / "logo-mark.svg", assets / "logo-mark.png", 128)
    sizes = (16, 24, 32, 48, 64, 128, 256)
    images = []
    with tempfile.TemporaryDirectory() as temp:
        for size in sizes:
            png = Path(temp) / f"{size}.png"
            render(assets / "logo.svg", png, size)
            images.append(png.read_bytes())
    offset = 6 + 16 * len(sizes)
    directory = bytearray(struct.pack("<HHH", 0, 1, len(sizes)))
    for size, image in zip(sizes, images):
        directory.extend(struct.pack("<BBBBHHII", size % 256, size % 256, 0, 0, 1, 32, len(image), offset))
        offset += len(image)
    (assets / "logo.ico").write_bytes(directory + b"".join(images))


if __name__ == "__main__":
    main()
