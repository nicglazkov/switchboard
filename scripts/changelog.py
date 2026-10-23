#!/usr/bin/env python3
"""Write CHANGELOG.md from the app's own release notes (RELEASES in native/crates/sb-desktop/src/about.rs, the list
About > What's new shows), so the two never disagree. Each version is dated by the first commit that set it in
native/Cargo.toml; a version not committed yet is dated today.

    scripts/changelog.py            # rewrite CHANGELOG.md
    scripts/changelog.py --check    # exit 1 when CHANGELOG.md is out of date
"""
import datetime, json, os, re, subprocess, sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
ABOUT = os.path.join(ROOT, 'native/crates/sb-desktop/src/about.rs')
OUT = os.path.join(ROOT, 'CHANGELOG.md')
REPO = 'https://github.com/nicglazkov/switchboard'


def releases():
    """[(version, [note, ...])], newest first, read from the Rust source."""
    s = open(ABOUT, encoding='utf-8').read()
    body = s[s.index('pub const RELEASES'):]
    body = body[:body.index('\n];')]
    out = []
    for m in re.finditer(r'\(\s*"(\d+\.\d+\.\d+)",\s*&\[(.*?)\],\s*\)', body, re.S):
        notes = [json.loads(n) for n in re.findall(r'"(?:[^"\\]|\\.)*"', m.group(2))]
        out.append((m.group(1), notes))
    return out


def dates():
    """{version: 'YYYY-MM-DD'} from the first commit that put each version in native/Cargo.toml."""
    log = subprocess.run(['git', 'log', '--reverse', '--format=DATE %ad', '--date=short', '-p', '--', 'native/Cargo.toml'],
                         cwd=ROOT, capture_output=True, text=True, check=True).stdout
    seen, day = {}, None
    for line in log.splitlines():
        if line.startswith('DATE '):
            day = line[5:]
        else:
            m = re.match(r'\+version = "(\d+\.\d+\.\d+)"', line)
            if m and m.group(1) not in seen:
                seen[m.group(1)] = day
    return seen


def noted(ver):
    """The day the release notes first named `ver`, for a version that never sat in Cargo.toml on its own."""
    out = subprocess.run(['git', 'log', '--reverse', '--format=%ad', '--date=short', '-S', f'"{ver}"', '--',
                          'native/crates/sb-desktop/src/about.rs'], cwd=ROOT, capture_output=True, text=True).stdout.split()
    return out[0] if out else None


def render():
    d, today = dates(), datetime.date.today().isoformat()
    for ver, _ in releases():
        d.setdefault(ver, noted(ver) or today)
    lines = ['# Changelog', '',
             'What changed in each version of Switchboard, newest first. The app shows the same notes in',
             'About > What\'s new. Downloads are on the',
             f'[releases page]({REPO}/releases).', '']
    for ver, notes in releases():
        lines += [f'## {ver} ({d[ver]})', '']
        lines += [f'- {n}' for n in notes]
        lines.append('')
    return '\n'.join(lines)


if __name__ == '__main__':
    text = render()
    if '--check' in sys.argv:
        cur = open(OUT, encoding='utf-8').read() if os.path.exists(OUT) else ''
        sys.exit(0 if cur == text else 1)
    open(OUT, 'w', encoding='utf-8').write(text)
    print(f'wrote {OUT} ({len(releases())} versions)')
