"""Installs a release's server kit over this server (serverctl.start_update runs it, apart from the server).

  python3 -I selfupdate.py --version 0.8.11 --sha256 <hex> --repo owner/name --dir <state>/update --url <kit url>

Each step is written to <dir>/state.json ({step, version, from, started, at, ok, error, got, total}); the installer's
own output goes to <dir>/update.log. It only uses the standard library, and runs from a copy in <dir>, since the
installer replaces the server's folder.
"""
import argparse, hashlib, json, os, shutil, ssl, subprocess, sys, tarfile, time, urllib.request
from pathlib import Path

HOME = Path.home()
BIN = HOME / '.local' / 'bin'


def tls():
    c = ssl.create_default_context()
    p = ssl.get_default_verify_paths()
    if sys.platform == 'darwin' and not p.cafile and not p.capath:
        try:
            pem = subprocess.run(['/usr/bin/security', 'find-certificate', '-a', '-p',
                                  '/System/Library/Keychains/SystemRootCertificates.keychain'],
                                 capture_output=True, text=True, timeout=10).stdout
            if pem.strip():
                c.load_verify_locations(cadata=pem)
        except (OSError, ssl.SSLError, subprocess.SubprocessError):
            pass
    return c


class State:
    def __init__(self, d):
        self.f = d / 'state.json'
        try:
            self.s = json.loads(self.f.read_text())
        except (OSError, ValueError):
            self.s = {}

    def put(self, **kw):
        self.s.update(kw, at=time.time())
        tmp = self.f.with_suffix('.tmp')
        tmp.write_text(json.dumps(self.s))
        os.replace(tmp, self.f)


def download(url, to, st):
    req = urllib.request.Request(url, headers={'User-Agent': 'Switchboard-server'})
    h = hashlib.sha256()
    with urllib.request.urlopen(req, timeout=30, context=tls()) as r, open(to, 'wb') as out:
        total = int(r.headers.get('Content-Length') or 0)
        got, last = 0, 0.0
        while True:
            b = r.read(256 * 1024)
            if not b:
                break
            out.write(b)
            h.update(b)
            got += len(b)
            if time.time() - last > 0.5:
                st.put(got=got, total=total)
                last = time.time()
        st.put(got=got, total=total or got)
    return h.hexdigest()


def unpack(kit, into):
    """The kit's one top folder, unpacked; nothing may land outside `into`."""
    with tarfile.open(kit, 'r:gz') as t:
        root = into.resolve()
        for m in t.getmembers():
            p = (into / m.name).resolve()
            if not (p == root or root in p.parents) or m.issym() or m.islnk() or m.isdev():
                raise ValueError(f'the kit holds an unsafe entry: {m.name}')
        try:
            t.extractall(into, filter='data')
        except TypeError:  # Python before 3.12
            t.extractall(into)
    tops = [p for p in into.iterdir() if p.is_dir()]
    if len(tops) != 1:
        raise ValueError('the kit is not one folder')
    return tops[0]


STEPS = {  # the installer's "== <step>" lines, as this shows them
    'Checking this machine': 'checking',
    'Putting the server in place': 'server',
    'The hub': 'hub',
    'Downloads for Add a machine': 'downloads',
    'Starting': 'starting_up',
}


def install(top, no_hub, st, log):
    argv = ['bash', str(top / 'scripts' / 'install' / 'install-server.sh'), '--yes', '--no-setup', '--plain']
    if no_hub:
        argv.append('--no-hub')
    env = dict(os.environ, PATH=f"{BIN}:{os.environ.get('PATH') or '/usr/bin:/bin'}", NO_COLOR='1')
    p = subprocess.Popen(argv, cwd=str(top), stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                         stderr=subprocess.STDOUT, env=env, text=True, errors='replace')
    tail = []
    for line in p.stdout:
        log.write(line)
        log.flush()
        tail = (tail + [line.rstrip()])[-12:]
        if line.startswith('== '):
            name = line[3:].strip()
            step = next((v for k, v in STEPS.items() if name.startswith(k)), None)
            if name.startswith('Running it'):
                step = 'restarting'
            if step:
                st.put(step=step)
    return p.wait(), tail


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--version', required=True)
    ap.add_argument('--sha256', required=True)
    ap.add_argument('--repo', required=True)
    ap.add_argument('--dir', required=True)
    ap.add_argument('--url', required=True)
    ap.add_argument('--no-hub', action='store_true')
    a = ap.parse_args()
    d = Path(a.dir)
    st = State(d)
    work = d / 'work'
    shutil.rmtree(work, ignore_errors=True)
    work.mkdir(parents=True)
    with open(d / 'update.log', 'a', encoding='utf-8') as log:
        log.write(f'\n== Updating to {a.version} at {time.ctime()}\n')
        try:
            st.put(step='downloading', got=0, total=0)
            kit = work / 'kit.tar.gz'
            got = download(a.url, kit, st)
            if got != a.sha256:
                raise ValueError("the download doesn't match the release's signed list, so it was not installed")
            st.put(step='unpacking')
            top = unpack(kit, work / 'kit')
            v = (top / 'VERSION').read_text().split()[0] if (top / 'VERSION').is_file() else ''
            if v != a.version:
                raise ValueError(f'the kit says {v or "no version"}, not {a.version}')
            st.put(step='checking')
            code, tail = install(top, a.no_hub, st, log)
            if code != 0:
                last = next((t for t in reversed(tail) if t.strip()), '')
                raise RuntimeError(f'the installer stopped ({last or f"exit {code}"})')
            st.put(step='done', ok=True, error=None, finished=time.time())
        except Exception as e:
            log.write(f'failed: {e}\n')
            st.put(ok=False, error=str(e)[:400], finished=time.time())
            # Whatever happened, leave the server running.
            if (BIN / 'switchboard').is_file():
                subprocess.run([str(BIN / 'switchboard'), 'start'], stdout=log, stderr=log, timeout=120, check=False)
        finally:
            shutil.rmtree(work, ignore_errors=True)


if __name__ == '__main__':
    main()
