"""Render BetterExplorer artwork in the sizes used by Windows."""

from pathlib import Path
from PIL import Image, ImageDraw, ImageFilter

ASSETS = Path(__file__).resolve().parents[1] / "assets"
SIZES = [16, 20, 24, 32, 40, 48, 64, 128, 256]


def draw(size, symbol):
    n = size * 6
    result = Image.new("RGBA", (n, n))

    def shape(box, radius):
        mask = Image.new("L", (n, n))
        ImageDraw.Draw(mask).rounded_rectangle(tuple(round(v * n) for v in box), round(radius * n), 255)
        return mask

    def polygon(points):
        mask = Image.new("L", (n, n))
        ImageDraw.Draw(mask).polygon([(round(x * n), round(y * n)) for x, y in points], 255)
        return mask

    def paint(mask, a, b=None):
        b = b or a
        colors = Image.new("RGB", (1, n))
        for y in range(n):
            colors.putpixel((0, y), tuple(round(x + (z - x) * y / (n - 1)) for x, z in zip(a, b)))
        layer = colors.resize((n, n)).convert("RGBA")
        layer.putalpha(mask)
        result.alpha_composite(layer)

    if symbol == "bex":
        paint(shape((.035, .035, .965, .965), .205), (86, 58, 47), (30, 21, 19))
    if symbol in ("bex", "folder"):
        paint(shape((.13, .22, .52, .55), .075), (255, 183, 87), (217, 91, 30))
        paint(shape((.13, .33, .87, .76), .078), (255, 179, 84), (183, 59, 18))
        paint(shape((.17, .42, .87, .81), .072), (255, 187, 88), (221, 90, 27))
    if symbol == "bex":
        bolt = polygon([(.61, .21), (.34, .57), (.51, .57), (.43, .90), (.74, .48), (.55, .48)])
        paint(bolt.filter(ImageFilter.MaxFilter(max(3, (n // 36) | 1))), (124, 43, 16))
        paint(bolt, (255, 255, 249), (255, 216, 122))
    if symbol == "file":
        paint(polygon([(.21, .07), (.58, .07), (.80, .30), (.80, .92), (.21, .92)]),
              (243, 247, 252), (147, 162, 183))
        paint(polygon([(.58, .07), (.58, .30), (.80, .30)]), (105, 125, 150))
        for y in (.46, .60, .74):
            paint(shape((.34, y, .66, y + .045), .02), (111, 130, 155))
    if symbol == "drive":
        paint(shape((.08, .26, .92, .76), .09), (209, 219, 233), (93, 108, 129))
        paint(shape((.17, .56, .83, .68), .04), (56, 65, 80))
        paint(shape((.68, .58, .78, .66), .04), (146, 255, 175))
    return result.resize((size, size), Image.Resampling.LANCZOS)


def main():
    ASSETS.mkdir(exist_ok=True)
    for symbol in ("bex", "folder", "file", "drive"):
        frames = [draw(size, symbol) for size in SIZES]
        frames[-1].save(ASSETS / f"{symbol}.ico", sizes=[(size, size) for size in SIZES], append_images=frames[:-1])
        if symbol == "bex":
            frames[-1].save(ASSETS / "bex.png")


if __name__ == "__main__":
    main()
