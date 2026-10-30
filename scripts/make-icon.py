"""Draws assets/favicon.ico: a white heart with a pulse line on a red rounded square.

    python3 scripts/make-icon.py      (needs Pillow)
"""

from pathlib import Path

from PIL import Image, ImageDraw

CANVAS = 1024
RED = (200, 52, 60, 255)
WHITE = (255, 255, 255, 255)
SIZES = [(16, 16), (32, 32), (48, 48), (64, 64), (128, 128)]


def draw() -> Image.Image:
    image = Image.new("RGBA", (CANVAS, CANVAS), (0, 0, 0, 0))
    pen = ImageDraw.Draw(image)
    pen.rounded_rectangle((0, 0, CANVAS - 1, CANVAS - 1), radius=220, fill=RED)

    # Heart: two circles joined to the tip along their tangent lines, so the sides have no notch.
    radius = 190
    pen.ellipse((330 - radius, 360 - radius, 330 + radius, 360 + radius), fill=WHITE)
    pen.ellipse((694 - radius, 360 - radius, 694 + radius, 360 + radius), fill=WHITE)
    pen.polygon([(188, 486), (330, 360), (694, 360), (836, 486), (512, 850)], fill=WHITE)

    # Pulse line across the heart.
    points = [(150, 520), (400, 520), (460, 400), (540, 680), (610, 520), (874, 520)]
    pen.line(points, fill=RED, width=44, joint="curve")

    return image


def main() -> None:
    assets = Path(__file__).resolve().parent.parent / "assets"
    image = draw()

    favicon = assets / "favicon.ico"
    image.resize((128, 128), Image.LANCZOS).save(favicon, format="ICO", sizes=SIZES)
    print(f"{favicon}: {favicon.stat().st_size} bytes")

    # For connector settings that take an uploaded image, such as ChatGPT's.
    icon = assets / "icon-512.png"
    image.resize((512, 512), Image.LANCZOS).save(icon, optimize=True)
    print(f"{icon}: {icon.stat().st_size} bytes")


if __name__ == "__main__":
    main()
