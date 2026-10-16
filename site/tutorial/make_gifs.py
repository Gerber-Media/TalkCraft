#!/usr/bin/env python3
"""Turns the keyframes of `crates/ui-egui/tests/site_media.rs` into the tutorial GIFs.

    python3 site/tutorial/make_gifs.py MEDIA OUT_DIR

Reads MEDIA/frames/<lang>/<scene>/timeline.json and writes OUT_DIR/<scene>-<lang>.gif plus a still
of the last keyframe (<scene>-<lang>.png, for the page before the GIF has loaded). The pointer glides
from one keyframe to the next, a click shows a ring, and each keyframe's caption runs along the
bottom. Needs Pillow.
"""
import json
import math
import os
import sys

from PIL import Image, ImageDraw, ImageFont

HERE = os.path.dirname(os.path.abspath(__file__))
FONT = os.path.join(HERE, "..", "..", "assets", "fonts", "Inter-SemiBold.ttf")
GREEN = (47, 158, 99)
STEP_MS = 40     # one frame of pointer motion
MOVE_MS = 520    # the way from one keyframe's pointer to the next
CLICK_MS = 240   # the ring of a click
CAPTION_H = 46


def arrow(scale=1.0):
    """A pointer arrow (tip at 0,0) as a polygon."""
    pts = [(0, 0), (0, 17), (4.2, 13.2), (7, 19.5), (9.6, 18.4), (6.9, 12.3), (12.2, 12.2)]
    return [(x * scale, y * scale) for x, y in pts]


def draw_pointer(img, at, scale=1.25):
    d = ImageDraw.Draw(img)
    x, y = at
    shadow = [(px + x + 1.5, py + y + 1.5) for px, py in arrow(scale)]
    d.polygon(shadow, fill=(0, 0, 0, 90))
    d.polygon([(px + x, py + y) for px, py in arrow(scale)], fill=(255, 255, 255), outline=(0, 0, 0), width=1)


def draw_ring(img, at, t):
    """The click: a green ring growing and fading, t in 0..1."""
    d = ImageDraw.Draw(img, "RGBA")
    r = 8 + 18 * t
    a = int(220 * (1 - t))
    x, y = at
    d.ellipse([x - r, y - r, x + r, y + r], outline=GREEN + (a,), width=4)


def caption_bar(img, text, font):
    if not text:
        return
    w, h = img.size
    band = Image.new("RGBA", (w, CAPTION_H), (16, 18, 17, 215))
    img.paste(band, (0, h - CAPTION_H), band)
    d = ImageDraw.Draw(img)
    box = d.textbbox((0, 0), text, font=font)
    tw, th = box[2] - box[0], box[3] - box[1]
    d.rectangle([0, h - CAPTION_H, 6, h], fill=GREEN)
    d.text(((w - tw) / 2, h - CAPTION_H + (CAPTION_H - th) / 2 - box[1]), text, font=font, fill=(245, 245, 240))


def ease(t):
    return 0.5 - 0.5 * math.cos(math.pi * t)


def compose(scene_dir, font):
    with open(os.path.join(scene_dir, "timeline.json"), encoding="utf-8") as f:
        tl = json.load(f)
    frames, durations = [], []

    def emit(base, at, caption, ms, ring=None):
        img = base.copy()
        if ring is not None:
            draw_ring(img, at, ring)
        draw_pointer(img, at)
        caption_bar(img, caption, font)
        frames.append(img)
        durations.append(max(20, int(ms)))

    prev_img, prev_at = None, None
    for k in tl["frames"]:
        img = Image.open(os.path.join(scene_dir, k["file"])).convert("RGB")
        at = tuple(k["cursor"])
        if prev_img is not None and prev_at != at:
            # The pointer travels over the screen as it was; the change shows when it arrives.
            steps = MOVE_MS // STEP_MS
            for i in range(1, steps):
                t = ease(i / steps)
                pos = (prev_at[0] + (at[0] - prev_at[0]) * t, prev_at[1] + (at[1] - prev_at[1]) * t)
                emit(prev_img, pos, k["caption"], STEP_MS)
        if k["click"]:
            for i in range(CLICK_MS // STEP_MS):
                emit(img, at, k["caption"], STEP_MS, ring=i / (CLICK_MS // STEP_MS))
        emit(img, at, k["caption"], k["ms"])
        prev_img, prev_at = img, at
    return frames, durations


def save_gif(frames, durations, path):
    # One palette for the whole scene keeps colours steady from frame to frame.
    sample = Image.new("RGB", (frames[0].width, frames[0].height * 2))
    sample.paste(frames[0], (0, 0))
    sample.paste(frames[-1], (0, frames[0].height))
    palette = sample.quantize(colors=255, method=Image.Quantize.MEDIANCUT)
    q = [f.quantize(palette=palette, dither=Image.Dither.NONE) for f in frames]
    q[0].save(path, save_all=True, append_images=q[1:], duration=durations, loop=0, optimize=True, disposal=1)


def main(media, out):
    os.makedirs(out, exist_ok=True)
    font = ImageFont.truetype(FONT, 19)
    root = os.path.join(media, "frames")
    for lang in sorted(os.listdir(root)):
        for scene in sorted(os.listdir(os.path.join(root, lang))):
            d = os.path.join(root, lang, scene)
            if not os.path.exists(os.path.join(d, "timeline.json")):
                continue
            frames, durations = compose(d, font)
            gif = os.path.join(out, f"{scene}-{lang}.gif")
            save_gif(frames, durations, gif)
            frames[-1].save(os.path.join(out, f"{scene}-{lang}.png"), optimize=True)
            print(f"{scene}-{lang}.gif: {len(frames)} frames, {sum(durations) / 1000:.1f} s, {os.path.getsize(gif) // 1024} KB")


if __name__ == "__main__":
    if len(sys.argv) != 3:
        sys.exit(__doc__)
    main(sys.argv[1], sys.argv[2])
