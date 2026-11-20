#!/usr/bin/env python3
"""Generate the Jarvis Windows icon, installer icon and MSIX tile assets.

Standard library only, so Windows CI and contributors can regenerate the
committed assets without Pillow or ImageMagick.

Usage: python packaging/windows/make_assets.py
"""

from __future__ import annotations

import struct
import zlib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
WINDOWS = ROOT / "packaging" / "windows"
MSIX_ASSETS = ROOT / "packaging" / "msix" / "Assets"

BLUE_TOP = (86, 160, 255)
BLUE_BOTTOM = (37, 99, 235)
WHITE = (255, 255, 255)


def clamp(value: float, low: float = 0.0, high: float = 1.0) -> float:
    return max(low, min(high, value))


def lerp(start: float, end: float, amount: float) -> float:
    return start + (end - start) * amount


def rounded_rect_alpha(
    x: float, y: float, width: float, height: float, inset: float, radius: float
) -> float:
    half_width = width / 2 - inset
    half_height = height / 2 - inset
    center_x = width / 2
    center_y = height / 2
    qx = abs(x + 0.5 - center_x) - (half_width - radius)
    qy = abs(y + 0.5 - center_y) - (half_height - radius)
    distance = (
        (max(qx, 0.0) ** 2 + max(qy, 0.0) ** 2) ** 0.5
        + min(max(qx, qy), 0.0)
        - radius
    )
    return clamp(0.5 - distance)


def circle_alpha(x: float, y: float, center_x: float, center_y: float, radius: float) -> float:
    return clamp(radius + 0.5 - ((x + 0.5 - center_x) ** 2 + (y + 0.5 - center_y) ** 2) ** 0.5)


def gradient(x: float, y: float, width: float, height: float) -> tuple[int, int, int]:
    amount = clamp((x + y) / max(1.0, width + height))
    return tuple(round(lerp(start, end, amount)) for start, end in zip(BLUE_TOP, BLUE_BOTTOM))


def square_master(size: int = 1024) -> bytes:
    pixels = bytearray(size * size * 4)
    inset = size * 0.035
    radius = size * 0.22
    dot_radius = size * 0.115
    for y in range(size):
        for x in range(size):
            tile = rounded_rect_alpha(x, y, size, size, inset, radius)
            if tile == 0.0:
                continue
            red, green, blue = gradient(x, y, size, size)
            dot = circle_alpha(x, y, size / 2, size / 2, dot_radius)
            if dot > 0.0:
                red = round(lerp(red, WHITE[0], dot))
                green = round(lerp(green, WHITE[1], dot))
                blue = round(lerp(blue, WHITE[2], dot))
            offset = (y * size + x) * 4
            pixels[offset] = red
            pixels[offset + 1] = green
            pixels[offset + 2] = blue
            pixels[offset + 3] = round(tile * 255)
    return bytes(pixels)


def wide_asset(width: int = 310, height: int = 150) -> bytes:
    pixels = bytearray(width * height * 4)
    inset = 4.0
    radius = height * 0.22
    dot_radius = height * 0.17
    center_x = width * 0.27
    center_y = height / 2
    bars = [(0.52, 0.28), (0.66, 0.5), (0.80, 0.36)]
    for y in range(height):
        for x in range(width):
            tile = rounded_rect_alpha(x, y, width, height, inset, radius)
            if tile == 0.0:
                continue
            red, green, blue = gradient(x, y, width, height)
            dot = circle_alpha(x, y, center_x, center_y, dot_radius)
            if dot > 0.0:
                red = round(lerp(red, WHITE[0], dot))
                green = round(lerp(green, WHITE[1], dot))
                blue = round(lerp(blue, WHITE[2], dot))
            for bar_x, bar_height in bars:
                bar_width = width * 0.045
                left = width * bar_x
                top = center_y - height * bar_height / 2
                bottom = center_y + height * bar_height / 2
                bar = clamp(
                    min(
                        x + 0.5 - left,
                        left + bar_width - (x + 0.5),
                        y + 0.5 - top,
                        bottom - (y + 0.5),
                    )
                    + 0.5
                )
                if bar > 0.0:
                    red = round(lerp(red, WHITE[0], bar))
                    green = round(lerp(green, WHITE[1], bar))
                    blue = round(lerp(blue, WHITE[2], bar))
            offset = (y * width + x) * 4
            pixels[offset] = red
            pixels[offset + 1] = green
            pixels[offset + 2] = blue
            pixels[offset + 3] = round(tile * 255)
    return bytes(pixels)


def resize(source: bytes, source_size: int, size: int) -> bytes:
    if size == source_size:
        return source
    output = bytearray(size * size * 4)
    scale = source_size / size
    for y in range(size):
        y0 = int(y * scale)
        y1 = max(y0 + 1, int((y + 1) * scale))
        for x in range(size):
            x0 = int(x * scale)
            x1 = max(x0 + 1, int((x + 1) * scale))
            alpha_sum = 0
            red_sum = green_sum = blue_sum = 0
            count = (y1 - y0) * (x1 - x0)
            for sy in range(y0, y1):
                for sx in range(x0, x1):
                    offset = (sy * source_size + sx) * 4
                    alpha = source[offset + 3]
                    alpha_sum += alpha
                    red_sum += source[offset] * alpha
                    green_sum += source[offset + 1] * alpha
                    blue_sum += source[offset + 2] * alpha
            offset = (y * size + x) * 4
            if alpha_sum:
                output[offset] = round(red_sum / alpha_sum)
                output[offset + 1] = round(green_sum / alpha_sum)
                output[offset + 2] = round(blue_sum / alpha_sum)
            output[offset + 3] = round(alpha_sum / count)
    return bytes(output)


def png_chunk(kind: bytes, data: bytes) -> bytes:
    return (
        struct.pack(">I", len(data))
        + kind
        + data
        + struct.pack(">I", zlib.crc32(kind + data) & 0xFFFFFFFF)
    )


def write_png(path: Path, width: int, height: int, pixels: bytes) -> None:
    raw = b"".join(
        b"\x00" + pixels[y * width * 4 : (y + 1) * width * 4] for y in range(height)
    )
    data = (
        b"\x89PNG\r\n\x1a\n"
        + png_chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0))
        + png_chunk(b"IDAT", zlib.compress(raw, 9))
        + png_chunk(b"IEND", b"")
    )
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(data)


def icon_image(pixels: bytes, size: int) -> bytes:
    header = struct.pack("<IiiHHIIiiII", 40, size, size * 2, 1, 32, 0, size * size * 4, 0, 0, 0, 0)
    body = bytearray()
    for y in range(size - 1, -1, -1):
        for x in range(size):
            offset = (y * size + x) * 4
            red, green, blue, alpha = pixels[offset : offset + 4]
            body.extend((blue, green, red, alpha))
    mask_stride = ((size + 31) // 32) * 4
    return header + bytes(body) + bytes(mask_stride * size)


def write_ico(path: Path, master: bytes, sizes: tuple[int, ...]) -> None:
    images = []
    for size in sizes:
        resized = resize(master, 1024, size)
        images.append((size, icon_image(resized, size)))
    header = struct.pack("<HHH", 0, 1, len(images))
    directory = bytearray()
    offset = len(header) + len(images) * 16
    for size, image in images:
        directory.extend(
            struct.pack(
                "<BBBBHHII",
                0 if size >= 256 else size,
                0 if size >= 256 else size,
                0,
                0,
                1,
                32,
                len(image),
                offset,
            )
        )
        offset += len(image)
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(header + bytes(directory) + b"".join(image for _, image in images))


def main() -> None:
    master = square_master()
    write_png(WINDOWS / "jarvis.png", 1024, 1024, master)
    write_ico(WINDOWS / "jarvis.ico", master, (16, 24, 32, 48, 64, 128, 256))
    for name, size in (
        ("StoreLogo.png", 50),
        ("Square44x44Logo.png", 44),
        ("Square150x150Logo.png", 150),
        ("Square310x310Logo.png", 310),
    ):
        write_png(MSIX_ASSETS / name, size, size, resize(master, 1024, size))
    write_png(MSIX_ASSETS / "Wide310x150Logo.png", 310, 150, wide_asset())
    print("Wrote packaging/windows/jarvis.{png,ico} and MSIX assets")


if __name__ == "__main__":
    main()
