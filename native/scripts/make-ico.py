#!/usr/bin/env python3
"""Pack per-size PNGs into one Windows .ico (PNG-compressed entries).

usage: make-ico.py <out.ico> <png> [<png> ...]
Each PNG must be square; its width becomes that entry's size. Runs on the Linux host (Pillow).
"""
import sys

from PIL import Image

out, paths = sys.argv[1], sys.argv[2:]
imgs = sorted((Image.open(p).convert('RGBA') for p in paths), key=lambda im: im.width)
for im in imgs:
    if im.width != im.height:
        sys.exit(f'not square: {im.width}x{im.height}')
big = imgs[-1]
big.save(out, format='ICO', sizes=[(im.width, im.height) for im in imgs], append_images=imgs[:-1])
check = Image.open(out)
print('ico sizes:', sorted(check.info.get('sizes', set())))
