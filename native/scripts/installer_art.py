#!/usr/bin/env python3
"""Draws the installer's art from branding.toml: the disk image's Finder background and the installer window's header.
Nothing here is hand-drawn, so renaming or recoloring the app is an edit to branding.toml and a rerun.

    python3 native/scripts/installer_art.py <out-dir>

Writes SVGs and renders them with rsvg-convert (librsvg), using the app's own fonts from sb-desktop/assets/fonts:
    dmg-background.png, dmg-background@2x.png   [dmg] window size, light (Finder draws its labels dark on it)
    header-light.png, header-dark.png           1600x300, the installer's header band at 2x
native/scripts/make-installer-art.sh runs this on the Linux host and copies the PNGs to native/packaging/installer/.
"""
import os
import re
import subprocess
import sys
import tempfile
from pathlib import Path

NATIVE = Path(__file__).resolve().parents[1]
BRANDING = NATIVE.parent / 'branding.toml'
if not BRANDING.exists():
    BRANDING = NATIVE / 'branding.toml'  # a build host's copy, synced next to native/'s contents
FONTS = NATIVE / 'crates' / 'sb-desktop' / 'assets' / 'fonts'
ICONS = NATIVE / 'crates' / 'sb-desktop' / 'assets' / 'icons'


def branding():
    """branding.toml as {section: {key: value}}: strings and integers, which is all it holds."""
    out, cur = {}, None
    for line in BRANDING.read_text().splitlines():
        line = line.strip()
        if not line or line.startswith('#'):
            continue
        m = re.match(r'\[(\w+)\]$', line)
        if m:
            cur = out.setdefault(m.group(1), {})
            continue
        m = re.match(r'(\w+)\s*=\s*(?:"([^"]*)"|(-?\d+))', line)
        if m and cur is not None:
            cur[m.group(1)] = m.group(2) if m.group(2) is not None else int(m.group(3))
    return out


def rgb(hex_):
    h = hex_.lstrip('#')
    return tuple(int(h[i:i + 2], 16) for i in (0, 2, 4))


def mix(a, b, t):
    """Hex color `a` moved toward `b` by t."""
    x, y = rgb(a), rgb(b)
    return '#%02X%02X%02X' % tuple(round(p + (q - p) * t) for p, q in zip(x, y))


def field(w, h, x0, cy, accent, base, n=9, spread=None, reach=None):
    """Many links converging on one point (x0+reach, cy), fanning out to the left: the header's motif."""
    spread = spread or h * 0.9
    reach = reach or w * 0.42
    tip = (x0 + reach, cy)
    out = []
    for i in range(n):
        f = i / (n - 1) - 0.5
        sy = cy + f * spread
        a = 0.10 + 0.22 * (1 - abs(f) * 2)
        col = mix(base, accent, 0.55)
        out.append(f'<path d="M{x0:.1f},{sy:.1f} C{x0 + reach * 0.55:.1f},{sy:.1f} {x0 + reach * 0.45:.1f},{cy:.1f} '
                   f'{tip[0]:.1f},{cy:.1f}" fill="none" stroke="{col}" stroke-opacity="{a:.2f}" stroke-width="2.2" '
                   f'stroke-linecap="round"/>')
        out.append(f'<circle cx="{x0:.1f}" cy="{sy:.1f}" r="4.2" fill="{col}" fill-opacity="{a + 0.12:.2f}"/>')
    out.append(f'<circle cx="{tip[0]:.1f}" cy="{cy:.1f}" r="46" fill="url(#halo)"/>')
    out.append(f'<circle cx="{tip[0]:.1f}" cy="{cy:.1f}" r="11" fill="{accent}"/>')
    return '\n'.join(out)


def dots(w, h, color, opacity, step=24, r=1.3):
    """A quiet dot grid, the board's texture."""
    return (f'<pattern id="grid" width="{step}" height="{step}" patternUnits="userSpaceOnUse">'
            f'<circle cx="{step / 2}" cy="{step / 2}" r="{r}" fill="{color}" fill-opacity="{opacity}"/></pattern>'
            f'<rect width="{w}" height="{h}" fill="url(#grid)"/>')


def header(b, dark):
    c = b['colors']
    bg = c['background_dark'] if dark else c['background_light']
    accent = c['accent_dark'] if dark else c['accent_light']
    ink = c['text_dark'] if dark else c['text_light']
    w, h = 1600, 300
    top = mix(bg, accent, 0.10 if dark else 0.07)
    return f'''<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="0 0 {w} {h}">
<defs>
<linearGradient id="bg" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="{bg}"/><stop offset="1" stop-color="{top}"/></linearGradient>
<radialGradient id="halo"><stop offset="0" stop-color="{accent}" stop-opacity="0.45"/><stop offset="1" stop-color="{accent}" stop-opacity="0"/></radialGradient>
<linearGradient id="fade" x1="0" y1="0" x2="1" y2="0"><stop offset="0" stop-color="{bg}" stop-opacity="1"/><stop offset="0.45" stop-color="{bg}" stop-opacity="0"/></linearGradient>
</defs>
<rect width="{w}" height="{h}" fill="url(#bg)"/>
{dots(w, h, ink, 0.07 if dark else 0.06)}
{field(w, h, w * 0.50, h / 2, accent, bg, n=11, spread=h * 1.25, reach=w * 0.36)}
<rect width="{w}" height="{h}" fill="url(#fade)"/>
<rect y="{h - 2}" width="{w}" height="2" fill="{ink}" fill-opacity="{0.10 if dark else 0.08}"/>
</svg>'''


def dmg(b, scale):
    c, d, a = b['colors'], b['dmg'], b['app']
    W, H = d['window_width'], d['window_height']
    bg, accent, ink = c['background_light'], c['accent_light'], c['text_light']
    s = scale
    icon = d['icon_size']
    ax, ay, px, py = d['app_x'], d['app_y'], d['applications_x'], d['applications_y']
    # The arrow from the app to Applications, between the icons, a little below their centers.
    x1, x2, y = ax + icon * 0.62, px - icon * 0.62, ay - 4
    name = a['name']
    return f'''<svg xmlns="http://www.w3.org/2000/svg" width="{W * s}" height="{H * s}" viewBox="0 0 {W} {H}">
<defs>
<linearGradient id="bg" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="{mix(bg, '#FFFFFF', 0.6)}"/><stop offset="1" stop-color="{mix(bg, accent, 0.06)}"/></linearGradient>
<radialGradient id="halo"><stop offset="0" stop-color="{accent}" stop-opacity="0.35"/><stop offset="1" stop-color="{accent}" stop-opacity="0"/></radialGradient>
</defs>
<rect width="{W}" height="{H}" fill="url(#bg)"/>
{dots(W, H, ink, 0.05, step=18, r=0.9)}
<g font-family="Manrope" font-weight="700" text-anchor="middle" fill="{ink}">
<text x="{W / 2}" y="56" font-size="19">Drag {name} to Applications</text>
</g>
<g font-family="Instrument Sans" font-weight="400" text-anchor="middle" fill="{mix(ink, bg, 0.35)}">
<text x="{W / 2}" y="80" font-size="12.5">Or open Install {name} for a guided setup.</text>
</g>
<path d="M{x1:.1f},{y} C{x1 + 50:.1f},{y - 26} {x2 - 50:.1f},{y - 26} {x2 - 6:.1f},{y}" fill="none" stroke="{accent}"
 stroke-width="2.4" stroke-linecap="round" stroke-dasharray="1 7"/>
<path d="M{x2 - 16:.1f},{y - 9} L{x2 - 4:.1f},{y + 1} L{x2 - 18:.1f},{y + 5}" fill="none" stroke="{accent}" stroke-width="2.4"
 stroke-linecap="round" stroke-linejoin="round"/>
<line x1="60" x2="{W - 60}" y1="{(ay + d['installer_y']) / 2 + 30}" y2="{(ay + d['installer_y']) / 2 + 30}" stroke="{ink}" stroke-opacity="0.07"/>
</svg>'''


def render(svg, out, tmp):
    src = Path(tmp) / (out.stem + '.svg')
    src.write_text(svg)
    subprocess.run(['rsvg-convert', '-o', str(out), str(src)], check=True)


def main():
    out = Path(sys.argv[1] if len(sys.argv) > 1 else '.')
    out.mkdir(parents=True, exist_ok=True)
    b = branding()
    with tempfile.TemporaryDirectory() as tmp:
        # rsvg-convert finds fonts through fontconfig: point it at the app's own fonts first.
        conf = Path(tmp) / 'fonts.conf'
        conf.write_text(f'<?xml version="1.0"?><fontconfig><dir>{FONTS}</dir><include ignore_missing="yes">'
                        f'/etc/fonts/fonts.conf</include><cachedir>{tmp}/fc</cachedir></fontconfig>')
        os.environ['FONTCONFIG_FILE'] = str(conf)
        render(dmg(b, 1), out / 'dmg-background.png', tmp)
        render(dmg(b, 2), out / 'dmg-background@2x.png', tmp)
        render(header(b, False), out / 'header-light.png', tmp)
        render(header(b, True), out / 'header-dark.png', tmp)
    for f in sorted(out.glob('*.png')):
        print(f'{f.name} {f.stat().st_size} bytes')


if __name__ == '__main__':
    main()
