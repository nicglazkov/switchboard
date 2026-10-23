#!/usr/bin/env bash
# Make the server kit: what get-server.sh downloads to install the Switchboard server on a machine with no copy of
# the repository. Run it in a copy of the repository.
#
#   scripts/install/pack-server-kit.sh                                   # into dist/server-kit/
#   scripts/install/pack-server-kit.sh --base https://example.org/sb     # get-server.sh downloads from there
#
# It writes three files:
#   switchboard-server-kit.tar.gz          server/, agent/sb.py, scripts/install/, LICENSE and VERSION, the
#                                          ready-made switchboard-hub and switchboard-node builds in native/dist,
#                                          and the Rust sources, so a machine with Rust can build the hub itself
#   switchboard-server-kit.tar.gz.sha256   its SHA-256, which get-server.sh checks
#   get-server.sh                          the one-line installer; with --base it knows where the kit is
# The kit also carries python-zeroconf and what it needs (pure Python wheels, licenses included) in server/vendor, so
# a Linux server without avahi-utils still announces itself; the server uses it only when the machine has none.
# Put all three in one folder on a web server. Then, on the new server machine:
#   curl -fsSL <base>/get-server.sh | bash
#
# Options:
#   --base URL        the folder the three files are served from (bakes <base>/switchboard-server-kit.tar.gz into
#                     get-server.sh; without it, get-server.sh needs --kit or SWITCHBOARD_KIT_URL)
#   --out DIR         where to write them (default dist/server-kit)
#   --no-binaries     leave out the ready-made hub and node builds
#   --no-sources      leave out the Rust sources (native/, as git has it)
#   --no-vendor       leave out server/vendor (python-zeroconf); pip downloads it otherwise
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"
OUT="$REPO/dist/server-kit"
BASE="" BINARIES=1 SOURCES=1 VENDOR=1
while [ $# -gt 0 ]; do
  case "$1" in
    --base) BASE="${2:?--base needs a URL}"; shift ;;
    --out) OUT="${2:?--out needs a folder}"; shift ;;
    --no-binaries) BINARIES=0 ;;
    --no-sources) SOURCES=0 ;;
    --no-vendor) VENDOR=0 ;;
    -h|--help) sed -n '2,/^set -e/p' "$0" | sed '$d; s/^# \{0,1\}//'; exit 0 ;;
    *) echo "pack-server-kit.sh: unknown option $1 (see --help)" >&2; exit 2 ;;
  esac
  shift
done
case "$BASE" in ""|http://*|https://*) ;; *) echo "pack-server-kit.sh: --base must start with http:// or https://" >&2; exit 2 ;; esac
BASE="${BASE%/}"

[ -f "$REPO/server/server.py" ] && [ -f "$REPO/agent/sb.py" ] || { echo "pack-server-kit.sh: run this in a copy of the repository" >&2; exit 1; }
VERSION=$(sed -n 's/^version = "\([^"]*\)".*/\1/p' "$REPO/native/Cargo.toml" 2>/dev/null | head -n 1)
COMMIT=$(git -C "$REPO" rev-parse --short HEAD 2>/dev/null || true)
STAMP=$(git -C "$REPO" log -1 --format=%ct 2>/dev/null || date +%s)
mkdir -p "$OUT"
KIT="$OUT/switchboard-server-kit.tar.gz"

# python-zeroconf 0.39.4 is the last release with a pure Python wheel; it runs on Python 3.7 and later.
WHEELS=""
if [ "$VENDOR" = 1 ]; then
  WHEELS=$(mktemp -d)
  trap 'rm -rf "$WHEELS"' EXIT
  if ! python3 -m pip download -q --no-deps --only-binary=:all: --platform any --python-version 3.9 -d "$WHEELS" \
      zeroconf==0.39.4 ifaddr==0.2.0 async-timeout==5.0.1 >/dev/null 2>&1; then
    echo "pack-server-kit.sh: could not download python-zeroconf (offline?); the kit is made without server/vendor" >&2
    WHEELS=""
  fi
fi

# Python's tarfile makes the same archive on macOS and Linux: sorted names, one time stamp, no owners.
python3 - "$REPO" "$KIT" "${VERSION:-unknown}${COMMIT:+ ($COMMIT)}" "$STAMP" "$BINARIES" "$SOURCES" "$WHEELS" <<'PY'
import glob, gzip, io, os, subprocess, sys, tarfile, zipfile
repo, out, version, stamp = sys.argv[1], sys.argv[2], sys.argv[3], int(sys.argv[4])
binaries, sources, wheels = sys.argv[5] == '1', sys.argv[6] == '1', sys.argv[7]
top = 'switchboard-server-kit'
skip_dirs = {'__pycache__', '.pytest_cache'}
files = []
for base in ('server', 'scripts/install'):
    for d, dirs, names in os.walk(os.path.join(repo, base)):
        dirs[:] = sorted(x for x in dirs if x not in skip_dirs)
        files += [os.path.join(d, n) for n in names if not n.endswith(('.pyc', '.pyo')) and n != '.DS_Store']
files += [os.path.join(repo, 'agent', 'sb.py'), os.path.join(repo, 'LICENSE'),
          os.path.join(repo, 'native', 'crates', 'sb-core', 'src', 'term', 'schemes.json')]
kept = []
if binaries:
    for n in ('switchboard-hub-linux', 'switchboard-hub-mac', 'switchboard-node-linux', 'switchboard-node-mac', 'switchboard-node.exe'):
        p = os.path.join(repo, 'native', 'dist', n)
        if os.path.isfile(p) and os.path.getsize(p):
            files.append(p); kept.append(n)
if sources:
    try:
        tracked = subprocess.run(['git', '-C', repo, 'ls-files', '-z', 'native'], capture_output=True, check=True).stdout
    except (OSError, subprocess.CalledProcessError):
        sys.exit('pack-server-kit.sh: the Rust sources are read from git; this is not a git checkout (try --no-sources)')
    files += [os.path.join(repo, n) for n in tracked.decode().split('\0')
              if n and not n.startswith(('native/dist/', 'native/target/')) and '/screenshots/' not in n]
files = sorted(set(f for f in files if os.path.isfile(f)))

def add(tar, name, data, mode):
    info = tarfile.TarInfo(f'{top}/{name}')
    info.size, info.mtime, info.mode = len(data), stamp, mode
    info.uid = info.gid = 0
    info.uname = info.gname = ''
    tar.addfile(info, io.BytesIO(data))

raw = io.BytesIO()
with tarfile.open(fileobj=raw, mode='w', format=tarfile.PAX_FORMAT) as tar:
    add(tar, 'VERSION', (version + '\n').encode(), 0o644)
    for f in files:
        rel = os.path.relpath(f, repo)
        with open(f, 'rb') as fh:
            data = fh.read()
        mode = 0o755 if os.access(f, os.X_OK) or rel.startswith('native/dist/') and not rel.endswith('.exe') else 0o644
        add(tar, rel, data, mode)
    vendored = []
    for whl in sorted(glob.glob(os.path.join(wheels, '*.whl'))) if wheels else []:
        with zipfile.ZipFile(whl) as z:
            for n in sorted(z.namelist()):
                if not n.endswith('/') and not n.endswith(('.pyc', '.pyo', '/RECORD')):
                    add(tar, f'server/vendor/{n}', z.read(n), 0o644)
        vendored.append(os.path.basename(whl).split('-')[0])
with open(out, 'wb') as fh, gzip.GzipFile(filename='', mode='wb', fileobj=fh, mtime=stamp) as gz:
    gz.write(raw.getvalue())
print(f'{len(files) + 1} files; ready-made builds: {", ".join(kept) or "none"}; Rust sources: {"yes" if sources else "no"}; '
      f'vendored: {", ".join(vendored) or "none"}')
PY

( cd "$OUT" && if command -v sha256sum >/dev/null; then sha256sum switchboard-server-kit.tar.gz; else shasum -a 256 switchboard-server-kit.tar.gz; fi ) >"$KIT.sha256"
if [ -n "$BASE" ]; then
  sed "s|^KIT_DEFAULT=\"\"|KIT_DEFAULT=\"$BASE/switchboard-server-kit.tar.gz\"|" "$HERE/get-server.sh" >"$OUT/get-server.sh"
else
  cp "$HERE/get-server.sh" "$OUT/get-server.sh"
fi
chmod 755 "$OUT/get-server.sh"

echo "Server kit ${VERSION:-} in $OUT"
echo "  $(cut -d ' ' -f 1 <"$KIT.sha256")  switchboard-server-kit.tar.gz ($(($(wc -c <"$KIT") / 1024)) KB)"
if [ -n "$BASE" ]; then
  echo "Serve that folder at $BASE, then on the new server machine:"
  echo "  curl -fsSL $BASE/get-server.sh | bash"
else
  echo "No --base given, so get-server.sh needs to be told where the kit is:"
  echo "  curl -fsSL <base>/get-server.sh | SWITCHBOARD_KIT_URL=<base>/switchboard-server-kit.tar.gz bash"
fi
