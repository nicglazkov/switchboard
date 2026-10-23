"""The fonts the apps bundle. GPUI on macOS refuses a font with no 'm' (it measures with it) and falls back to the
system font without saying so, which happened to a Manrope cut down to the letters of the product name."""
import struct, unittest
from pathlib import Path

FONTS = Path(__file__).resolve().parents[1] / 'native' / 'crates' / 'sb-desktop' / 'assets' / 'fonts'


def chars(path):
    """The code points a TrueType font maps (its format 4 cmap subtables)."""
    d = path.read_bytes()
    tables = {}
    for i in range(struct.unpack('>H', d[4:6])[0]):
        tag, _, off, _ = struct.unpack('>4sIII', d[12 + 16 * i:28 + 16 * i])
        tables[tag] = off
    cmap, out = tables[b'cmap'], set()
    for i in range(struct.unpack('>H', d[cmap + 2:cmap + 4])[0]):
        _, _, sub = struct.unpack('>HHI', d[cmap + 4 + 8 * i:cmap + 12 + 8 * i])
        st = cmap + sub
        if struct.unpack('>H', d[st:st + 2])[0] != 4:
            continue
        n2 = struct.unpack('>H', d[st + 6:st + 8])[0]
        ends = struct.unpack(f'>{n2 // 2}H', d[st + 14:st + 14 + n2])
        starts = struct.unpack(f'>{n2 // 2}H', d[st + 16 + n2:st + 16 + 2 * n2])
        for a, b in zip(starts, ends):
            if b != 0xFFFF:
                out.update(range(a, b + 1))
    return out


class FontsTest(unittest.TestCase):
    def test_every_bundled_font_has_the_whole_latin_alphabet(self):
        ttfs = sorted(FONTS.glob('*.ttf'))
        self.assertTrue(ttfs)
        need = {ord(c) for c in 'abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789'}
        for f in ttfs:
            with self.subTest(font=f.name):
                self.assertFalse(need - chars(f), f'{f.name} is missing characters')


if __name__ == '__main__':
    unittest.main()
