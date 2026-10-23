"""Updates from GitHub Releases (docs/architecture.md, Updates).

Each release carries latest.json (every build's file, size, SHA-256, version and build id) and latest.json.sig, an
Ed25519 signature of its exact bytes. The server reads both from the newest release and hands them to the apps as
they are; the apps check the signature with the key compiled into them, so neither this server nor GitHub can
change what is offered. The apps have no TLS, so they download through this server (GET /api/app/files/<file>),
which passes the release's file on, Range requests included, and only for a file the newest list names.

SWITCHBOARD_RELEASES names another repository ("owner/name"); "off" turns the GitHub look off.
"""
import json, os, re, ssl, subprocess, sys, urllib.request

REPO = 'nicglazkov/switchboard'
REPO_RE = re.compile(r'[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+')
FILE_RE = re.compile(r'[A-Za-z0-9][A-Za-z0-9._-]{0,127}')
CHUNK = 64 * 1024


def repo():
    """The repository whose releases the apps update from, or None when that is turned off."""
    r = (os.environ.get('SWITCHBOARD_RELEASES') or REPO).strip()
    return r if REPO_RE.fullmatch(r) else None


def page(r, version=None):
    """The release page people read: one version's notes, or the newest."""
    return f'https://github.com/{r}/releases/tag/v{version}' if version else f'https://github.com/{r}/releases/latest'


def asset(r, version, name):
    return f'https://github.com/{r}/releases/download/v{version}/{name}'


_CTX = None


def context():
    """TLS that checks certificates. Python from python.org on a Mac has no certificates of its own until someone
    runs its Install Certificates step, so there the Mac's own root certificates are used."""
    global _CTX
    if _CTX is None:
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
        _CTX = c
    return _CTX


def _open(req, timeout, opener):
    return opener(req, timeout=timeout) if opener else urllib.request.urlopen(req, timeout=timeout, context=context())


def _get(url, limit, opener):
    req = urllib.request.Request(url, headers={'User-Agent': 'Switchboard-server'})
    with _open(req, 10, opener) as resp:
        return resp.read(limit)


def latest(r, opener=None):
    """{manifest, sig, version, files}: the newest release's list as text, byte for byte, and its signature (None when
    the release has none). OSError or ValueError when GitHub can't be reached or the list is not one."""
    text = _get(f'https://github.com/{r}/releases/latest/download/latest.json', 200_000, opener).decode('utf-8')
    d = json.loads(text)
    if not isinstance(d, dict) or not isinstance(d.get('version'), str) or not isinstance(d.get('files'), dict):
        raise ValueError('the newest release has no list of builds')
    try:
        sig = _get(f'https://github.com/{r}/releases/latest/download/latest.json.sig', 1000, opener).decode('ascii').strip()
    except OSError:
        sig = None
    names = {f.get('file') for f in d['files'].values() if isinstance(f, dict)}
    return {'manifest': text, 'sig': sig or None, 'version': d['version'],
            'files': sorted(n for n in names if isinstance(n, str) and FILE_RE.fullmatch(n))}


def base_for(host):
    """Where an app downloads the files from: this server, at the address the app used. None for an odd Host."""
    h = (host or '').strip()
    return f'http://{h}/api/app/files' if re.fullmatch(r'[A-Za-z0-9.\-]+(?::\d{1,5})?|\[[0-9A-Fa-f:.]+\](?::\d{1,5})?', h) else None


def open_file(r, version, name, rng=None, opener=None):
    """The release file, ready to read: (status, headers to send, response). `rng` is the app's Range header."""
    req = urllib.request.Request(asset(r, version, name), headers={'User-Agent': 'Switchboard-server'})
    if rng and re.fullmatch(r'bytes=\d+-\d*', rng):
        req.add_header('Range', rng)
    resp = _open(req, 30, opener)
    status = getattr(resp, 'status', 200)
    out = {'Content-Type': 'application/octet-stream'}
    for k in ('Content-Length', 'Content-Range', 'Accept-Ranges'):
        if resp.headers.get(k):
            out[k] = resp.headers[k]
    return status, out, resp
