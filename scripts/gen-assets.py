#!/usr/bin/env python3
"""生成桌面程序的图标与图标字体子集（产物已提交到仓库，只有修改设计时才需要重新运行）。

依赖：pip install pillow fonttools
用法：python3 scripts/gen-assets.py [--emoji-font <NotoEmoji-Regular.ttf>]
  - crates/desktop/assets/icon/icon-{16..1024}.png、icon.ico（Windows）、icon.icns（macOS）
  - crates/desktop/assets/fonts/LaborFeeIcons-Regular.ttf：Noto Emoji（SIL OFL 1.1）子集，只含界面用到的图标字形
"""
import argparse
import glob
import os
import sys

from PIL import Image, ImageDraw, ImageFont

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
ASSETS = os.path.join(ROOT, "crates", "desktop", "assets")
CJK_FONT = os.path.join(ASSETS, "fonts", "LaborFeeSansSC-Regular.otf")

# 界面用到的图标字形（与 crates/desktop/src/icons.rs 保持一致）
ICON_CHARS = "➡🔄💰📊☀🌙📂💾✅✔⚠❌✖ℹ▶⬇📥📤🔍📝🔢🔒❓💡📄📈📋🏢💼💴🔧📐📌🎯🌓🔀↔💲💵📃📉🏦⬅🔃📎"

TOP = (29, 78, 216)  # 品牌蓝 #1D4ED8
BOTTOM = (13, 148, 136)  # 青 #0D9488


def icon(size: int) -> Image.Image:
    s = 1024
    img = Image.new("RGBA", (s, s), (0, 0, 0, 0))
    grad = Image.new("RGBA", (s, s))
    px = grad.load()
    for y in range(s):
        for x in range(s):
            t = (x * 0.35 + y * 0.65) / s
            px[x, y] = tuple(int(TOP[i] + (BOTTOM[i] - TOP[i]) * t) for i in range(3)) + (255,)
    mask = Image.new("L", (s, s), 0)
    margin = 72  # macOS 图标留白
    ImageDraw.Draw(mask).rounded_rectangle((margin, margin, s - margin, s - margin), radius=200, fill=255)
    img.paste(grad, (0, 0), mask)
    d = ImageDraw.Draw(img)
    # 白色票据卡片
    card = (300, 230, 724, 800)
    d.rounded_rectangle(card, radius=56, fill=(255, 255, 255, 245))
    font = ImageFont.truetype(CJK_FONT, 330)
    d.text((512, 440), "¥", font=font, fill=TOP + (255,), anchor="mm")
    for i, w in enumerate((300, 220)):
        y = 640 + i * 70
        d.rounded_rectangle((512 - w // 2, y, 512 + w // 2, y + 34), radius=17, fill=BOTTOM + (255,))
    return img.resize((size, size), Image.LANCZOS)


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--emoji-font")
    a = ap.parse_args()
    out = os.path.join(ASSETS, "icon")
    os.makedirs(out, exist_ok=True)
    big = icon(1024)
    for n in (16, 32, 48, 64, 128, 256):
        big.resize((n, n), Image.LANCZOS).save(os.path.join(out, f"icon-{n}.png"))
    big.save(os.path.join(out, "icon.ico"), sizes=[(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)])
    big.save(os.path.join(out, "icon.icns"))
    print("图标已生成：", out)

    src = a.emoji_font or next(iter(glob.glob(os.path.expanduser(
        "~/.cargo/registry/src/*/epaint_default_fonts-*/fonts/NotoEmoji-Regular.ttf"))), None)
    if not src:
        sys.exit("找不到 NotoEmoji-Regular.ttf，请用 --emoji-font 指定")
    from fontTools import subset
    from fontTools.ttLib import TTFont

    opts = subset.Options()
    opts.name_IDs = ["*"]
    opts.notdef_outline = True
    opts.hinting = False
    f = TTFont(src)
    sub = subset.Subsetter(opts)
    sub.populate(unicodes=[ord(c) for c in ICON_CHARS])
    sub.subset(f)
    # 按 OFL 要求，修改过的字体改名
    for rec in f["name"].names:
        if rec.nameID in (1, 3, 4, 6, 16):
            rec.string = "LaborFeeIcons-Regular" if rec.nameID == 6 else "LaborFee Icons"
    dst = os.path.join(ASSETS, "fonts", "LaborFeeIcons-Regular.ttf")
    f.save(dst)
    print("图标字体已生成：", dst, os.path.getsize(dst), "字节")


if __name__ == "__main__":
    main()
