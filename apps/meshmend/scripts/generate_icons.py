from __future__ import annotations

import shutil
import subprocess
from pathlib import Path

from PIL import Image, ImageDraw, ImageFilter


ROOT = Path(__file__).resolve().parents[1]
ICON_DIR = ROOT / "src-tauri" / "icons"
SOURCE_SIZE = 1024

PNG_TARGETS = {
    "32x32.png": 32,
    "128x128.png": 128,
    "128x128@2x.png": 256,
    "Square30x30Logo.png": 30,
    "Square44x44Logo.png": 44,
    "Square71x71Logo.png": 71,
    "Square89x89Logo.png": 89,
    "Square107x107Logo.png": 107,
    "Square142x142Logo.png": 142,
    "Square150x150Logo.png": 150,
    "Square284x284Logo.png": 284,
    "Square310x310Logo.png": 310,
    "StoreLogo.png": 50,
    "icon.png": 512,
}


def main() -> None:
    ICON_DIR.mkdir(parents=True, exist_ok=True)
    source = draw_source_icon()

    source.save(ICON_DIR / "icon-source.png")

    for filename, size in PNG_TARGETS.items():
        write_png(source, ICON_DIR / filename, size)

    write_ico(source, ICON_DIR / "icon.ico")
    write_icns(source, ICON_DIR / "icon.icns")


def draw_source_icon() -> Image.Image:
    size = SOURCE_SIZE
    scale = size / 1024
    image = Image.new("RGBA", (size, size), (0, 0, 0, 0))

    shadow = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    shadow_draw = ImageDraw.Draw(shadow)
    shadow_draw.rounded_rectangle(
        box(92, 112, 932, 940, scale),
        radius=int(188 * scale),
        fill=(0, 0, 0, 120),
    )
    shadow = shadow.filter(ImageFilter.GaussianBlur(int(28 * scale)))
    image.alpha_composite(shadow)

    background = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    bg_draw = ImageDraw.Draw(background)
    for y in range(size):
        t = y / max(size - 1, 1)
        r = int(17 + 14 * (1 - t))
        g = int(23 + 20 * (1 - t))
        b = int(34 + 30 * (1 - t))
        bg_draw.line((0, y, size, y), fill=(r, g, b, 255))

    mask = Image.new("L", (size, size), 0)
    mask_draw = ImageDraw.Draw(mask)
    mask_draw.rounded_rectangle(
        box(88, 82, 936, 930, scale),
        radius=int(184 * scale),
        fill=255,
    )
    background.putalpha(mask)
    image.alpha_composite(background)

    draw = ImageDraw.Draw(image)
    draw_mesh(draw, scale)
    draw_mending(draw, scale)

    return image


def draw_mesh(draw: ImageDraw.ImageDraw, scale: float) -> None:
    points = {
        "a": point(226, 590, scale),
        "b": point(288, 356, scale),
        "c": point(452, 242, scale),
        "d": point(660, 268, scale),
        "e": point(794, 440, scale),
        "f": point(770, 646, scale),
        "g": point(604, 770, scale),
        "h": point(386, 742, scale),
        "i": point(500, 496, scale),
        "j": point(372, 500, scale),
        "k": point(628, 512, scale),
    }

    triangles = [
        ("a", "b", "j", (42, 111, 132, 255)),
        ("b", "c", "j", (48, 132, 154, 255)),
        ("c", "i", "j", (65, 155, 172, 255)),
        ("c", "d", "i", (82, 177, 186, 255)),
        ("d", "k", "i", (68, 149, 170, 255)),
        ("d", "e", "k", (45, 123, 153, 255)),
        ("e", "f", "k", (39, 103, 135, 255)),
        ("f", "g", "k", (51, 127, 148, 255)),
        ("g", "i", "k", (70, 158, 159, 255)),
        ("g", "h", "i", (54, 135, 148, 255)),
        ("h", "a", "j", (38, 100, 129, 255)),
        ("h", "j", "i", (53, 132, 151, 255)),
    ]

    for p1, p2, p3, fill in triangles:
        draw.polygon([points[p1], points[p2], points[p3]], fill=fill)

    line = (148, 239, 232, 190)
    for p1, p2, p3, _ in triangles:
        draw.line([points[p1], points[p2], points[p3], points[p1]], fill=line, width=int(7 * scale))

    outline = [points[key] for key in ("a", "b", "c", "d", "e", "f", "g", "h", "a")]
    draw.line(outline, fill=(204, 255, 246, 210), width=int(11 * scale), joint="curve")

    for key in ("b", "c", "d", "e", "g", "h"):
        x, y = points[key]
        r = int(11 * scale)
        draw.ellipse((x - r, y - r, x + r, y + r), fill=(209, 255, 247, 235))


def draw_mending(draw: ImageDraw.ImageDraw, scale: float) -> None:
    crack = [
        point(494, 268, scale),
        point(468, 382, scale),
        point(526, 492, scale),
        point(486, 622, scale),
        point(544, 742, scale),
    ]

    draw.line(crack, fill=(14, 18, 25, 245), width=int(34 * scale), joint="curve")
    draw.line(crack, fill=(28, 34, 42, 245), width=int(19 * scale), joint="curve")

    stitches = [
        ((438, 342), (535, 318)),
        ((426, 432), (546, 398)),
        ((457, 530), (574, 560)),
        ((438, 636), (552, 620)),
        ((493, 716), (595, 682)),
    ]

    gold = (255, 190, 83, 255)
    gold_dark = (184, 108, 44, 255)

    for start, end in stitches:
        start_point = point(*start, scale)
        end_point = point(*end, scale)
        draw.line([start_point, end_point], fill=gold_dark, width=int(22 * scale))
        draw.line([start_point, end_point], fill=gold, width=int(13 * scale))
        for x, y in (start_point, end_point):
            r = int(14 * scale)
            draw.ellipse((x - r, y - r, x + r, y + r), fill=gold)


def write_png(source: Image.Image, path: Path, size: int) -> None:
    source.resize((size, size), Image.Resampling.LANCZOS).save(path)


def write_ico(source: Image.Image, path: Path) -> None:
    sizes = [(16, 16), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)]
    source.save(path, sizes=sizes)


def write_icns(source: Image.Image, path: Path) -> None:
    iconset = ICON_DIR / "MeshMend.iconset"
    if iconset.exists():
        shutil.rmtree(iconset)

    iconset.mkdir()
    icon_specs = {
        "icon_16x16.png": 16,
        "icon_16x16@2x.png": 32,
        "icon_32x32.png": 32,
        "icon_32x32@2x.png": 64,
        "icon_128x128.png": 128,
        "icon_128x128@2x.png": 256,
        "icon_256x256.png": 256,
        "icon_256x256@2x.png": 512,
        "icon_512x512.png": 512,
        "icon_512x512@2x.png": 1024,
    }

    for filename, size in icon_specs.items():
        write_png(source, iconset / filename, size)

    subprocess.run(["iconutil", "-c", "icns", str(iconset), "-o", str(path)], check=True)
    shutil.rmtree(iconset)


def point(x: int, y: int, scale: float) -> tuple[int, int]:
    return int(x * scale), int(y * scale)


def box(x1: int, y1: int, x2: int, y2: int, scale: float) -> tuple[int, int, int, int]:
    return int(x1 * scale), int(y1 * scale), int(x2 * scale), int(y2 * scale)


if __name__ == "__main__":
    main()
