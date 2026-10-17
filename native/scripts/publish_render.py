#!/usr/bin/env python3
"""Render the Switchboard download page and swap it into place. Runs on the publish host.

publish.sh uploads the page assets and the built apps into <dest>.incoming, then pipes this
script over ssh. It hashes every artifact, fills the page template, writes SHA256SUMS, and
replaces <dest> with the new folder in two renames, so the lab server never serves a half
copied page.

usage: python3 - --dest DIR --base URL --version X.Y.Z [--allow-missing]
"""
import argparse
import datetime
import hashlib
import html
import json
import os
import re
import shutil
import sys
from zoneinfo import ZoneInfo

ARTIFACTS = {  # placeholder prefix -> file name
    'MAC': 'Switchboard-mac.zip',
    'WIN': 'Switchboard-windows.zip',
    'LINUX': 'Switchboard-linux-x86_64',
}
OPTIONAL = {'ANDROID': 'Switchboard-android.apk'}  # built by android/scripts/build.sh
LINUX_TGZ = 'Switchboard-linux-x86_64.tar.gz'
PAGE_FILES = ('index.html', 'favicon.svg', 'icon-macos.svg', 'manrope.woff2', 'Manrope-OFL.txt')
TZ = ZoneInfo(os.environ.get('SWITCHBOARD_TZ') or 'UTC')  # dates on the download page


def die(msg):
    print('publish: ' + msg, file=sys.stderr)
    sys.exit(1)


def safe_dir(p):
    """A remote folder this script may replace: under the home dir, not the home dir itself."""
    home = os.path.realpath(os.path.expanduser('~'))
    full = os.path.realpath(os.path.join(home, os.path.expanduser(p)))
    if not full.startswith(home + os.sep) or full == home or len(full) - len(home) < 4:
        die(f'refusing to publish into {full!r}: pick a folder under {home}')
    return full


def sha256(path):
    h = hashlib.sha256()
    with open(path, 'rb') as f:
        for chunk in iter(lambda: f.read(1 << 20), b''):
            h.update(chunk)
    return h.hexdigest()


def size_text(n):
    if n >= 1_000_000:
        return f'{n / 1_000_000:.1f} MB'
    if n >= 1_000:
        return f'{n / 1_000:.0f} KB'
    return f'{n} bytes'


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--dest', required=True)
    ap.add_argument('--base', required=True)
    ap.add_argument('--version', required=True)
    ap.add_argument('--allow-missing', action='store_true')
    a = ap.parse_args()

    dest = safe_dir(a.dest)
    stage = dest + '.incoming'
    old = dest + '.previous'
    if not os.path.isdir(stage):
        die(f'nothing uploaded at {stage}')
    for f in PAGE_FILES:
        if not os.path.isfile(os.path.join(stage, f)):
            die(f'page file missing from upload: {f}')
    if not re.fullmatch(r'\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?', a.version):
        die(f'odd version {a.version!r}')
    if not re.fullmatch(r'https?://[A-Za-z0-9.:\-]+(?:/[A-Za-z0-9._\-/]*)?', a.base):
        die(f'odd base url {a.base!r}')
    base = a.base.rstrip('/')

    values = {'VERSION': a.version, 'BASE': base}
    sums = []
    mtimes = []
    missing = []
    for key, name in {**ARTIFACTS, **OPTIONAL}.items():
        path = os.path.join(stage, name)
        values[key + '_FILE'] = name
        if os.path.isfile(path):
            st = os.stat(path)
            digest = sha256(path)
            values[key + '_STATE'] = 'ready'
            values[key + '_SIZE'] = size_text(st.st_size)
            values[key + '_SHA'] = digest
            sums.append(f'{digest}  {name}')
            mtimes.append(st.st_mtime)
        else:
            if key not in OPTIONAL:
                missing.append(name)
            values[key + '_STATE'] = 'missing'
            values[key + '_SIZE'] = ''
            values[key + '_SHA'] = ''
    if missing and not a.allow_missing:
        die('missing: ' + ', '.join(missing) + ' (pass --allow-missing to publish the rest)')
    if not mtimes:
        die('no artifacts at all')

    tgz = os.path.join(stage, LINUX_TGZ)
    if os.path.isfile(tgz):
        st = os.stat(tgz)
        digest = sha256(tgz)
        sums.append(f'{digest}  {LINUX_TGZ}')
        values['LINUX_TGZ_LINK'] = f'<a href="{LINUX_TGZ}" download>{LINUX_TGZ}</a> <span>{size_text(st.st_size)}</span>'
    else:
        values['LINUX_TGZ_LINK'] = ''

    built = datetime.datetime.fromtimestamp(max(mtimes), TZ)
    values['BUILT'] = f'{built:%b} {built.day}, {built.year}'

    page_path = os.path.join(stage, 'index.html')
    with open(page_path, encoding='utf-8') as f:
        page = f.read()

    def fill(m):
        k = m.group(1)
        if k not in values:
            die(f'template asks for unknown value {k}')
        v = values[k]
        return v if k == 'LINUX_TGZ_LINK' else html.escape(v, quote=True)

    page = re.sub(r'\{\{([A-Z_]+)\}\}', fill, page)
    if '{{' in page or '\u2014' in page:
        die('rendered page still has a placeholder or an em dash')
    with open(page_path, 'w', encoding='utf-8') as f:
        f.write(page)
    with open(os.path.join(stage, 'SHA256SUMS'), 'w', encoding='utf-8') as f:
        f.write('\n'.join(sums) + '\n')
    # latest.json: what the desktop apps compare their own build with ("Restart to update", never automatic).
    latest = {'version': a.version, 'built': max(mtimes), 'files': {}}
    for plat, key in (('mac', 'MAC'), ('windows', 'WIN'), ('linux', 'LINUX')):
        name = ARTIFACTS[key]
        if values[key + '_STATE'] != 'ready':
            continue
        bpath = os.path.join(stage, name + '.build')
        build = open(bpath, encoding='utf-8').read().strip() if os.path.isfile(bpath) else ''
        latest['files'][plat] = {'file': name, 'sha256': values[key + '_SHA'], 'build': build}
        if os.path.isfile(bpath):
            os.remove(bpath)
    with open(os.path.join(stage, 'latest.json'), 'w', encoding='utf-8') as f:
        json.dump(latest, f, indent=1)

    # Swap: dest -> dest.previous, stage -> dest, then drop dest.previous.
    if os.path.isdir(old):
        shutil.rmtree(old)
    if os.path.isdir(dest):
        os.rename(dest, old)
    os.rename(stage, dest)
    if os.path.isdir(old):
        shutil.rmtree(old)

    print(f'published {a.version} ({values["BUILT"]}) to {dest}')
    for line in sums:
        digest, name = line.split('  ', 1)
        size = os.path.getsize(os.path.join(dest, name))
        print(f'  {name:36} {size_text(size):>9}  {digest[:16]}')
    if missing:
        print('  not published: ' + ', '.join(missing))


if __name__ == '__main__':
    main()
