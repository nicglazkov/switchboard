#!/usr/bin/env python3
"""Render the Switchboard download page and swap it into place, and make the signed update list.

On the computer that publishes (publish.sh runs these locally, where the builds and the key are):
  publish_render.py manifest --dist DIR --version X.Y.Z --out latest.json
      latest.json: each build's file, size, SHA-256 and build id, for the apps' updates.
  publish_render.py sign --key FILE --in latest.json --out latest.json.sig [--expect PUBKEY]
      latest.json.sig: the base64 Ed25519 signature of latest.json's exact bytes, made with OpenSSL 3
      (SB_OPENSSL names another openssl). --expect refuses a key whose public half is not PUBKEY.

On the publish host (piped over ssh, after publish.sh uploads the page, the apps and latest.json
into <dest>.incoming):
  python3 - --dest DIR --base URL --version X.Y.Z [--allow-missing]
      hashes every artifact, checks latest.json agrees (it is kept byte for byte, so its
      signature still holds), fills the page template, writes SHA256SUMS, and replaces <dest>
      with the new folder in two renames, so the server never serves a half copied page.
"""
import argparse
import base64
import datetime
import hashlib
import html
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
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


PLATFORMS = (('mac', 'MAC'), ('windows', 'WIN'), ('linux', 'LINUX'))  # latest.json key, ARTIFACTS key


def kit_version(kit):
    """The version in a server kit's VERSION file ("0.8.11 (abc1234)" gives "0.8.11"), or ''."""
    import tarfile
    try:
        with tarfile.open(kit, 'r:gz') as t:
            m = next((m for m in t.getmembers() if m.name.count('/') == 1 and m.name.endswith('/VERSION')), None)
            f = m and t.extractfile(m)
            return f.read().decode().split()[0] if f else ''
    except (OSError, tarfile.TarError, IndexError, UnicodeDecodeError):
        return ''


def make_manifest(dist, version, kit=None):
    """latest.json's bytes for the builds in `dist`: what the apps compare their own version and build with.
    A build's id comes from <file>.build, which the build scripts write next to it."""
    files, mtimes = {}, []
    for plat, key in PLATFORMS:
        name = ARTIFACTS[key]
        path = os.path.join(dist, name)
        if not os.path.isfile(path):
            continue
        bpath = path + '.build'
        build = open(bpath, encoding='utf-8').read().strip() if os.path.isfile(bpath) else ''
        if not re.fullmatch(r'\d{14}', build):
            print(f'publish: {name} has no build id ({name}.build), so apps will not be offered it', file=sys.stderr)
        files[plat] = {'file': name, 'sha256': sha256(path), 'build': build, 'size': os.path.getsize(path)}
        mtimes.append(os.path.getmtime(path))
    if not files:
        die(f'no builds in {dist}')
    if kit and os.path.isfile(kit):
        # The server kit, for Settings > Server's update: the server installs it only if it matches this SHA-256.
        kv = kit_version(kit)
        if kv == version:
            files['server'] = {'file': os.path.basename(kit), 'sha256': sha256(kit), 'build': '', 'size': os.path.getsize(kit)}
        else:
            print(f'publish: the server kit is {kv or "of no version"}, not {version}, so it is left out', file=sys.stderr)
    return (json.dumps({'version': version, 'built': max(mtimes), 'files': files}, indent=1) + '\n').encode()


def openssl():
    return os.environ.get('SB_OPENSSL') or 'openssl'


def public_key(key_file):
    """The base64 raw public key of an Ed25519 private key file, as SB_UPDATE_PUBKEY wants it."""
    der = subprocess.run([openssl(), 'pkey', '-in', key_file, '-pubout', '-outform', 'DER'],
                         capture_output=True, check=False)
    if der.returncode != 0 or len(der.stdout) != 44:
        die(f'could not read the Ed25519 key in {key_file} (it needs OpenSSL 3; SB_OPENSSL names another openssl)')
    return base64.b64encode(der.stdout[-32:]).decode()


def sign(key_file, data):
    """The base64 Ed25519 signature of `data`."""
    with tempfile.TemporaryDirectory() as d:
        src, out = os.path.join(d, 'm'), os.path.join(d, 's')
        with open(src, 'wb') as f:
            f.write(data)
        r = subprocess.run([openssl(), 'pkeyutl', '-sign', '-rawin', '-inkey', key_file, '-in', src, '-out', out],
                           capture_output=True, check=False)
        if r.returncode != 0 or not os.path.isfile(out) or os.path.getsize(out) != 64:
            die('could not sign latest.json: ' + (r.stderr.decode(errors='replace').strip() or 'no signature made'))
        with open(out, 'rb') as f:
            return base64.b64encode(f.read()).decode()


def check_manifest(text, stage, values):
    """latest.json as uploaded must list exactly the builds in `stage`, with their sizes and hashes."""
    try:
        m = json.loads(text)
    except ValueError as e:
        die(f'latest.json is unreadable: {e}')
    listed = m.get('files') if isinstance(m, dict) else None
    if not isinstance(listed, dict):
        die('latest.json lists no files')
    for plat, key in PLATFORMS:
        name = ARTIFACTS[key]
        f = listed.get(plat)
        if values[key + '_STATE'] != 'ready':
            if f:
                die(f'latest.json lists {name}, which was not uploaded')
            continue
        if not f or f.get('file') != name or f.get('sha256') != values[key + '_SHA'] \
                or f.get('size') != os.path.getsize(os.path.join(stage, name)):
            die(f'latest.json does not match the uploaded {name}')
    return m


def manifest_main(argv):
    ap = argparse.ArgumentParser(prog='publish_render.py manifest')
    ap.add_argument('--dist', required=True)
    ap.add_argument('--version', required=True)
    ap.add_argument('--out', required=True)
    ap.add_argument('--kit', help='the server kit (pack-server-kit.sh), listed as "server" when it is this version')
    a = ap.parse_args(argv)
    with open(a.out, 'wb') as f:
        f.write(make_manifest(a.dist, a.version, a.kit))


def sign_main(argv):
    ap = argparse.ArgumentParser(prog='publish_render.py sign')
    ap.add_argument('--key', required=True)
    ap.add_argument('--in', dest='src', required=True)
    ap.add_argument('--out', required=True)
    ap.add_argument('--expect', help='the public key the apps were built with (SB_UPDATE_PUBKEY)')
    a = ap.parse_args(argv)
    if a.expect and public_key(a.key) != a.expect.strip():
        die('SB_UPDATE_KEY_FILE is not the key SB_UPDATE_PUBKEY belongs to, so the apps would refuse this list')
    with open(a.src, 'rb') as f:
        data = f.read()
    with open(a.out, 'w', encoding='ascii') as f:
        f.write(sign(a.key, data) + '\n')


def main():
    if len(sys.argv) > 1 and sys.argv[1] in ('manifest', 'sign'):
        return (manifest_main if sys.argv[1] == 'manifest' else sign_main)(sys.argv[2:])
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
    # latest.json (and latest.json.sig) came from publish.sh, made and signed where the key is. It stays exactly
    # as uploaded, so the signature holds; here it only has to agree with the files.
    latest_path = os.path.join(stage, 'latest.json')
    if not os.path.isfile(latest_path):
        die('latest.json missing from upload')
    with open(latest_path, encoding='utf-8') as f:
        latest = check_manifest(f.read(), stage, values)
    if latest.get('version') != a.version:
        die(f'latest.json is for {latest.get("version")!r}, not {a.version}')
    if not os.path.isfile(latest_path + '.sig'):
        print('publish: latest.json is not signed, so no app will offer this version as an update')

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
