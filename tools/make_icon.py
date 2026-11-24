"""Render BetterExplorer artwork in the sizes used by Windows."""

from pathlib import Path
from PIL import Image, ImageDraw, ImageFilter

ASSETS = Path(__file__).resolve().parents[1] / "assets"
SIZES = [16, 20, 24, 32, 40, 48, 64, 128, 256]
NAVIGATION_SIZES = [16, 20, 24, 32, 36, 40, 48, 64, 128, 256]
APP_SIZES = [16, 20, 24, 28, 30, 32, 36, 40, 48, 60, 64, 72, 80, 96, 128, 256]
SYMBOLS = (
    "bex", "folder", "file", "drive", "home", "desktop", "downloads",
    "documents", "pictures", "music", "videos", "network", "recycle", "libraries",
)


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

    def ellipse(box):
        mask = Image.new("L", (n, n))
        ImageDraw.Draw(mask).ellipse(tuple(round(v * n) for v in box), 255)
        return mask

    def paint(mask, a, b=None, *, fit=False):
        b = b or a
        top, bottom = 0, n
        if fit:
            _, top, _, bottom = mask.getbbox()
        colors = Image.new("RGB", (1, n))
        for y in range(top, bottom):
            fraction = (y - top) / (bottom - top - 1)
            colors.putpixel((0, y), tuple(round(start + (end - start) * fraction) for start, end in zip(a, b)))
        layer = colors.resize((n, n)).convert("RGBA")
        layer.putalpha(mask)
        result.alpha_composite(layer)

    if symbol == "bex":
        paint(shape((.035, .035, .965, .965), .205), (86, 58, 47), (30, 21, 19))
        paint(shape((.075, .165, .485, .48), .084), (255, 179, 104), (211, 82, 25), fit=True)
        paint(shape((.075, .298, .925, .853), .09), (249, 129, 48), (142, 39, 11), fit=True)
        paint(shape((.094, .365, .925, .853), .085), (249, 133, 56), (160, 48, 12), fit=True)
        bolt = polygon([(.61, .215), (.34, .57), (.49, .57), (.42, .89), (.72, .47), (.55, .47)])
        paint(bolt.filter(ImageFilter.MaxFilter(max(3, (n // 36) | 1))), (122, 34, 10))
        paint(bolt, (255, 255, 250), (255, 218, 128))
    elif symbol == "folder":
        paint(shape((.035, .13, .48, .53), .087), (255, 211, 112), (210, 125, 31))
        paint(shape((.035, .28, .965, .85), .089), (247, 189, 75), (190, 106, 23))
        paint(shape((.066, .39, .965, .884), .087), (255, 217, 122), (223, 150, 43))
    elif symbol == "file":
        paint(polygon([(.21, .07), (.58, .07), (.80, .30), (.80, .92), (.21, .92)]),
              (243, 247, 252), (147, 162, 183))
        paint(polygon([(.58, .07), (.58, .30), (.80, .30)]), (105, 125, 150))
        for y in (.46, .60, .74):
            paint(shape((.34, y, .66, y + .045), .02), (111, 130, 155))
    elif symbol == "drive":
        paint(shape((.08, .26, .92, .76), .09), (209, 219, 233), (93, 108, 129))
        paint(shape((.17, .56, .83, .68), .04), (56, 65, 80))
        paint(shape((.68, .58, .78, .66), .04), (146, 255, 175))
    elif symbol == "home":
        paint(shape((.23, .41, .77, .88), .09), (247, 166, 107), (213, 103, 49))
        paint(polygon([(.08, .47), (.50, .10), (.92, .47), (.83, .57), (.50, .29), (.17, .57)]),
              (202, 84, 50), (153, 50, 36))
        paint(shape((.41, .61, .59, .88), .045), (255, 241, 213), (255, 217, 164))
    elif symbol == "desktop":
        paint(shape((.42, .64, .58, .85), .045), (60, 121, 169), (44, 87, 127))
        paint(shape((.27, .80, .73, .90), .05), (85, 139, 179), (54, 92, 135))
        paint(shape((.07, .12, .93, .71), .085), (63, 140, 193), (36, 82, 130))
        paint(shape((.17, .22, .83, .60), .04), (188, 244, 251), (91, 173, 221))
    elif symbol == "downloads":
        paint(polygon([(.40, .10), (.60, .10), (.60, .43), (.78, .43), (.50, .71),
                       (.22, .43), (.40, .43)]), (54, 179, 158), (17, 112, 103))
        paint(shape((.13, .69, .27, .89), .055), (38, 145, 131), (23, 103, 101))
        paint(shape((.73, .69, .87, .89), .055), (38, 145, 131), (23, 103, 101))
        paint(shape((.13, .77, .87, .89), .055), (38, 145, 131), (23, 103, 101))
    elif symbol == "documents":
        paint(shape((.19, .09, .81, .91), .075), (113, 117, 216), (73, 77, 162))
        paint(shape((.30, .17, .73, .82), .045), (245, 248, 255), (207, 215, 255))
        for y, width in ((.32, .24), (.49, .24), (.66, .16)):
            paint(shape((.39, y, .39 + width, y + .065), .03), (94, 100, 179))
    elif symbol == "pictures":
        paint(shape((.08, .17, .92, .83), .105), (102, 178, 120), (39, 119, 84))
        paint(ellipse((.23, .30, .39, .46)), (255, 244, 179))
        paint(polygon([(.17, .73), (.39, .51), (.50, .62), (.67, .41), (.84, .73)]),
              (222, 250, 198), (180, 229, 171))
    elif symbol == "music":
        paint(polygon([(.36, .25), (.80, .12), (.80, .31), (.36, .44)]),
              (178, 104, 223), (109, 55, 178))
        paint(shape((.36, .27, .49, .74), .04), (161, 87, 218), (110, 54, 169))
        paint(shape((.67, .20, .80, .66), .04), (161, 87, 218), (110, 54, 169))
        paint(ellipse((.14, .67, .49, .88)), (184, 107, 229), (115, 53, 177))
        paint(ellipse((.46, .56, .80, .78)), (184, 107, 229), (115, 53, 177))
    elif symbol == "videos":
        paint(shape((.07, .20, .93, .80), .115), (227, 112, 135), (175, 59, 91))
        paint(polygon([(.40, .32), (.70, .50), (.40, .68)]), (255, 247, 244), (251, 219, 229))
    elif symbol == "network":
        paint(shape((.22, .65, .78, .76), .035), (77, 153, 184), (42, 106, 144))
        paint(polygon([(.44, .32), (.54, .37), (.27, .73), (.17, .67)]),
              (77, 153, 184), (42, 106, 144))
        paint(polygon([(.56, .32), (.46, .37), (.73, .73), (.83, .67)]),
              (77, 153, 184), (42, 106, 144))
        for box in ((.34, .10, .66, .42), (.08, .59, .40, .91), (.60, .59, .92, .91)):
            paint(ellipse(box), (114, 205, 224), (44, 139, 176))
    elif symbol == "recycle":
        paint(polygon([(.19, .35), (.81, .35), (.74, .84), (.68, .91), (.32, .91), (.26, .84)]),
              (115, 202, 206), (47, 131, 150))
        for x in (.35, .56):
            paint(shape((x, .46, x + .09, .77), .04), (203, 247, 242), (157, 218, 218))
        paint(shape((.38, .11, .62, .29), .06), (74, 153, 170), (43, 95, 115))
        paint(shape((.12, .24, .88, .38), .06), (74, 153, 170), (43, 95, 115))
    elif symbol == "libraries":
        paint(shape((.08, .22, .31, .86), .044), (241, 156, 91), (189, 84, 50))
        paint(shape((.35, .14, .58, .86), .045), (105, 159, 206), (54, 94, 155))
        paint(polygon([(.60, .30), (.76, .25), (.93, .81), (.77, .86)]),
              (181, 128, 210), (114, 67, 158))
        paint(shape((.13, .33, .26, .43), .02), (255, 235, 177), (251, 207, 145))
        paint(shape((.40, .25, .53, .40), .02), (217, 240, 255), (157, 208, 244))
        paint(polygon([(.66, .40), (.77, .36), (.80, .45), (.69, .49)]), (230, 202, 246))
    return result.resize((size, size), Image.Resampling.LANCZOS)


def main():
    ASSETS.mkdir(exist_ok=True)
    for symbol in SYMBOLS:
        if symbol == "bex":
            sizes = APP_SIZES
        elif symbol in ("file", "drive"):
            sizes = SIZES
        else:
            sizes = NAVIGATION_SIZES
        frames = [draw(size, symbol) for size in sizes]
        frames[-1].save(ASSETS / f"{symbol}.ico", sizes=[(size, size) for size in sizes], append_images=frames[:-1])
        if symbol == "bex":
            frames[-1].save(ASSETS / "bex.png")


if __name__ == "__main__":
    main()
