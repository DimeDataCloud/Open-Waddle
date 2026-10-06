#!/usr/bin/env python3
"""Draws the Chrome Web Store promo tiles from Waddle's own sprite.

    python3 scripts/make-store-art.py        # writes release/store-small-440x280.png and release/store-marquee-1400x560.png

Needs Pillow. Output is 24-bit PNG without alpha, as the store requires.
"""
import json
import os
from PIL import Image, ImageDraw, ImageFont

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
PAPER, INK, MUTED, YELLOW, SKY = (255, 253, 246), (42, 30, 20), (122, 104, 86), (255, 210, 63), (214, 236, 255)
PALETTE = {"o": (42, 30, 20), "b": (255, 210, 63), "d": (158, 130, 39), "k": (92, 76, 23), "w": (255, 255, 255),
           "e": (20, 20, 28), "a": (255, 140, 26), "A": (158, 87, 16)}
FONTS = "/usr/share/fonts/truetype/liberation/LiberationSans-{}.ttf"


def font(size, bold=False):
    return ImageFont.truetype(FONTS.format("Bold" if bold else "Regular"), size)


def duck(img, x, y, scale, frame="idle", flip=False):
    rows = json.load(open(os.path.join(ROOT, "src/body/waddle.sprites.json")))["frames"][frame]
    for ry, row in enumerate(rows):
        for rx, ch in enumerate(row[::-1] if flip else row):
            if ch in PALETTE:
                px, py = x + rx * scale, y + ry * scale
                img.paste(PALETTE[ch], (px, py, px + scale, py + scale))


def bubble(d, box, text, size, tail="right"):
    x0, y0, x1, y1 = box
    d.rounded_rectangle(box, radius=14, fill=PAPER, outline=INK, width=3)
    cx, cy = (x1, (y0 + y1) // 2) if tail == "right" else ((x0 + x1) // 2, y1)
    pts = [(x1 - 2, cy - 10), (x1 + 16, cy + 4), (x1 - 2, cy + 12)] if tail == "right" else [(cx - 10, y1 - 2), (cx + 4, y1 + 16), (cx + 12, y1 - 2)]
    d.polygon(pts, fill=PAPER)
    d.line([pts[0], pts[1], pts[2]], fill=INK, width=3)
    d.line([pts[0], pts[2]], fill=PAPER, width=5)
    f = font(size, True)
    w = d.textlength(text, font=f)
    d.text(((x0 + x1 - w) / 2, (y0 + y1) / 2 - size * 0.62), text, font=f, fill=INK)


def window(d, box, title_bar=44):
    x0, y0, x1, y1 = box
    d.rounded_rectangle(box, radius=16, fill=(255, 255, 255), outline=INK, width=4)
    d.rounded_rectangle((x0, y0, x1, y0 + title_bar), radius=16, fill=SKY, outline=INK, width=4)
    d.rectangle((x0 + 4, y0 + title_bar - 14, x1 - 4, y0 + title_bar - 2), fill=SKY)
    d.line((x0 + 4, y0 + title_bar, x1 - 4, y0 + title_bar), fill=INK, width=3)
    for i, c in enumerate([(255, 95, 86), (255, 189, 46), (39, 201, 63)]):
        d.ellipse((x0 + 18 + i * 26, y0 + 14, x0 + 34 + i * 26, y0 + 30), fill=c, outline=INK, width=2)


def field(d, box, label, value=None):
    d.text((box[0], box[1] - 24), label, font=font(17, True), fill=MUTED)
    d.rounded_rectangle(box, radius=8, fill=(250, 248, 240), outline=MUTED, width=2)
    if value:
        d.text((box[0] + 14, (box[1] + box[3]) / 2 - 12), value, font=font(21), fill=INK)


def button(d, box, text):
    d.rounded_rectangle(box, radius=10, fill=YELLOW, outline=INK, width=3)
    f = font(22, True)
    w = d.textlength(text, font=f)
    d.text(((box[0] + box[2] - w) / 2, (box[1] + box[3]) / 2 - 14), text, font=f, fill=INK)


def small():
    img = Image.new("RGB", (440, 280), PAPER)
    d = ImageDraw.Draw(img)
    d.rectangle((0, 262, 440, 280), fill=YELLOW)
    d.text((24, 20), "Waddle for Chrome", font=font(34, True), fill=INK)
    d.text((24, 64), "Reads and uses web pages for you", font=font(18), fill=MUTED)
    window(d, (24, 160, 416, 262), title_bar=30)
    field(d, (44, 214, 250, 246), "Address", "12 Duck Lane")
    button(d, (276, 214, 398, 246), "Order")
    duck(img, 292, 160 - 98, 7, "idle")
    bubble(d, (92, 96, 268, 142), "Filling that in!", 20)
    return img


def marquee():
    img = Image.new("RGB", (1400, 560), PAPER)
    d = ImageDraw.Draw(img)
    d.rectangle((0, 530, 1400, 560), fill=YELLOW)
    d.text((80, 100), "Waddle for Chrome", font=font(72, True), fill=INK)
    d.text((84, 206), "Your desktop duck reads and uses web pages", font=font(34), fill=MUTED)
    d.text((84, 252), "for you, and asks before anything risky.", font=font(34), fill=MUTED)
    x = 84
    for label in ["Open source", "Private", "Asks first"]:
        w = d.textlength(label, font=font(26, True)) + 40
        d.rounded_rectangle((x, 350, x + w, 400), radius=25, fill=(255, 244, 200), outline=INK, width=3)
        d.text((x + 20, 360), label, font=font(26, True), fill=INK)
        x += w + 18
    window(d, (800, 210, 1330, 490), title_bar=48)
    d.rounded_rectangle((900, 222, 1290, 246), radius=12, fill=(255, 255, 255), outline=INK, width=2)
    d.text((912, 223), "shop.example/checkout", font=font(16), fill=MUTED)
    field(d, (832, 310, 1298, 352), "Full name", "Dana Duck")
    field(d, (832, 392, 1060, 434), "Address", "12 Duck Lane")
    button(d, (1100, 392, 1298, 434), "Place order")
    duck(img, 1150, 210 - 140, 10, "peck")
    bubble(d, (850, 70, 1100, 130), "Filling that in!", 26)
    return img


if __name__ == "__main__":
    out = os.path.join(ROOT, "release")
    os.makedirs(out, exist_ok=True)
    for name, img in [("store-small-440x280.png", small()), ("store-marquee-1400x560.png", marquee())]:
        img.convert("RGB").save(os.path.join(out, name), optimize=True)
        print(os.path.join(out, name), img.size, img.mode)
