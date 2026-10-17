#!/usr/bin/env bash
# Prove the install path on fresh Linux containers: one server and three fake machines, set up only with the
# scripts in scripts/install, the way someone who isn't the owner would.
#
#   scripts/lab/install-lab.sh          # needs Docker; builds switchboard-hub and switchboard-node with cargo if needed
#   scripts/lab/install-lab.sh --keep   # leave the containers running afterwards (docker rm -f sblab-server ... to clean)
#
# The containers (network sblab, all Ubuntu 24.04 with python3, curl, ps and an ssh server, nothing of Switchboard):
#   sblab-server   the server, installed by install-server.sh and set up by setup.py
#   sblab-studio   links with a code, using the pasted curl line
#   sblab-laptop   links with a code, using the pasted line for a machine with no curl
#   sblab-tower    is added over ssh (install-agent.sh --ssh-key, then setup.py --ssh)
#   sblab-desk     is added by a person running setup.py in a terminal (drive-setup.py types the answers)
#   sblab-bare     Ubuntu with no Python: install-agent.sh must say what to install and stop
#   sblab-sd       Ubuntu running systemd (privileged): the server as systemd user services, linked to itself
# The machines are fake in one way: `claude` is not installed. Each gets one made-up chat in ~/.claude/projects, so
# the server has a session to list. Nothing reaches a real Claude session.
#
# The base image is made once from ubuntu:24.04 (apt over https; set https_proxy if the network needs it).
set -uo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"
KEEP=0; [ "${1:-}" = --keep ] && KEEP=1
NET=sblab
IMG=switchboard-lab-base:ubuntu24
WORK=$(mktemp -d)
chmod 755 "$WORK"  # the containers' own users read it
PASS=0 FAIL=0
ok() { PASS=$((PASS + 1)); printf '  ok    %s\n' "$*"; }
bad() { FAIL=$((FAIL + 1)); printf '  FAIL  %s\n' "$*"; }
check() { local what="$1"; shift; if "$@" >/dev/null 2>&1; then ok "$what"; else bad "$what"; fi; }
as() { local c="$1" u="$2"; shift 2; docker exec "$c" su - "$u" -c "$*"; }
step() { printf '\n== %s\n' "$*"; }

cleanup() {
  if [ "$KEEP" = 1 ]; then echo "Kept the containers, and $WORK (mounted at /lab in them)."; return; fi
  docker rm -f sblab-server sblab-studio sblab-laptop sblab-tower sblab-desk sblab-bare sblab-sd >/dev/null 2>&1
  rm -rf "$WORK"
}
trap cleanup EXIT

command -v docker >/dev/null && docker info >/dev/null 2>&1 || { echo "install-lab: Docker is not running" >&2; exit 1; }

# ---------------------------------------------------------------- base image and binaries
CA=""; [ -f /root/.ccr/ca-bundle.crt ] && CA=/root/.ccr/ca-bundle.crt  # a proxy's CA, when apt goes through one
if ! docker image inspect "$IMG" >/dev/null 2>&1; then
  step "Making the base image $IMG"
  docker rm -f sblab-base >/dev/null 2>&1
  docker run -d --name sblab-base --network host ${CA:+-v $CA:/ca.crt:ro} ubuntu:24.04 sleep infinity >/dev/null
  docker exec -e https_proxy="${https_proxy:-${HTTPS_PROXY:-}}" sblab-base sh -c '
    set -e; sed -i "s#http://#https://#g" /etc/apt/sources.list.d/ubuntu.sources
    O=""; [ -f /ca.crt ] && O="-o Acquire::https::CAInfo=/ca.crt"
    apt-get $O update -qq >/dev/null
    DEBIAN_FRONTEND=noninteractive apt-get $O install -y -qq --no-install-recommends python3 procps openssh-server openssh-client curl ca-certificates >/dev/null
    apt-get clean; rm -rf /var/lib/apt/lists/*' || { echo "install-lab: could not make the base image" >&2; exit 1; }
  docker commit sblab-base "$IMG" >/dev/null && docker rm -f sblab-base >/dev/null
fi
step "Building switchboard-hub and switchboard-node (as a release would ship them)"
( cd "$REPO/native" && cargo build --release --locked -q -p sb-hub -p sb-node ) || { echo "install-lab: cargo build failed" >&2; exit 1; }
mkdir -p "$WORK/dist"
cp "$REPO/native/target/release/switchboard-hub" "$WORK/dist/switchboard-hub-linux"
cp "$REPO/native/target/release/switchboard-node" "$WORK/dist/switchboard-node-linux"
# The repository as a new user would download it: what git tracks, plus this working tree's scripts/install.
( cd "$REPO" && git ls-files -co --exclude-standard -- server agent scripts/install | tar -cf "$WORK/repo.tar" -T - )
chmod -R a+rX "$WORK"

docker network inspect "$NET" >/dev/null 2>&1 || docker network create "$NET" >/dev/null
docker rm -f sblab-server sblab-studio sblab-laptop sblab-tower sblab-desk sblab-bare sblab-sd >/dev/null 2>&1
run() { docker run -d --init --name "sblab-$1" --hostname "$1" --network "$NET" -v "$WORK:/lab:ro" "$2" sleep infinity >/dev/null; }

# A made-up chat, so the machine has a session to list (Claude Code writes these; it is not installed here).
seed_chat() {  # $1 container, $2 user, $3 title
  docker exec -i "$1" su - "$2" -c 'python3 - "$0"' "$3" <<'PY'
import json, sys, uuid, datetime, pathlib
home = pathlib.Path.home(); cwd = home / 'Code' / 'app'; cwd.mkdir(parents=True, exist_ok=True)
proj = home / '.claude' / 'projects' / str(cwd).replace('/', '-'); proj.mkdir(parents=True, exist_ok=True)
sid = str(uuid.uuid4()); now = datetime.datetime.now(datetime.timezone.utc)
ts = lambda s: (now - datetime.timedelta(seconds=s)).strftime('%Y-%m-%dT%H:%M:%S.000Z')
rows = [{'type': 'user', 'timestamp': ts(120), 'sessionId': sid, 'cwd': str(cwd), 'origin': {'kind': 'human'},
         'message': {'role': 'user', 'content': sys.argv[1]}},
        {'type': 'assistant', 'timestamp': ts(60), 'sessionId': sid, 'cwd': str(cwd), 'message': {'role': 'assistant',
         'model': 'claude-opus-5-5', 'content': [{'type': 'text', 'text': 'Done.'}],
         'usage': {'input_tokens': 2, 'cache_creation_input_tokens': 100, 'cache_read_input_tokens': 1000, 'output_tokens': 50}}}]
(proj / f'{sid}.jsonl').write_text(''.join(json.dumps(r) + '\n' for r in rows))
PY
}

api() {  # GET a server route, from inside the server (JSON on stdout)
  as sblab-server alex "python3 -c 'import sys, urllib.request; sys.stdout.write(urllib.request.urlopen(\"http://127.0.0.1:8130$1\", timeout=20).read().decode())'"
}
jq_py() { python3 -c "import json, sys; d = json.load(sys.stdin); print($1)"; }

# ---------------------------------------------------------------- the server
step "Server: install-server.sh on a fresh machine"
run server "$IMG"
docker exec sblab-server useradd -m -s /bin/bash alex
as sblab-server alex 'mkdir -p src && tar -xf /lab/repo.tar -C src && bash src/scripts/install/install-server.sh --hub-binary /lab/dist/switchboard-hub-linux --node-dir /lab/dist --yes' | sed 's/^/    /'
check "server answers on :8130" api /api/summary
api /api/summary >/dev/null 2>&1 || { echo "install-lab: the server did not install; stopping" >&2; exit 1; }
check "hub answers on :8132" as sblab-server alex 'curl -fsS http://127.0.0.1:8132/healthz'
check "downloads serve install-agent.sh, sb.py and switchboard-node-linux" as sblab-server alex \
  'cd /tmp && curl -fsS http://server:8131/install-agent.sh -o a && curl -fsS http://server:8131/node/sb.py -o b && curl -fsS http://server:8131/node/switchboard-node-linux -o c'

step "Server: first-run setup (address)"
as sblab-server alex 'python3 .local/share/switchboard/app/install/setup.py --address server --yes' | sed 's/^/    /'
check "server.json puts the chosen address first" as sblab-server alex \
  "python3 -c 'import json; d = json.load(open(\".config/switchboard/server.json\")); assert d[\"origins\"][0] == \"http://server:8130\"'"
check "the server answers at its address" as sblab-server alex 'curl -fsS http://server:8130/api/summary'

code_line() {  # $1 = which line (linux, "linux (no curl)")
  as sblab-server alex 'python3 .local/share/switchboard/app/install/setup.py --code --json' | jq_py "d['lines']['$1']"
}

# ---------------------------------------------------------------- machines linked with a code
step "Studio: links with a code (the pasted curl line)"
run studio "$IMG"; docker exec sblab-studio useradd -m -s /bin/bash sam
seed_chat sblab-studio sam "Fix the login page"
LINE=$(code_line linux)
printf '    pasted: %s\n' "$LINE"
as sblab-studio sam "$LINE --label Studio" | sed 's/^/    /'

step "Laptop: links with a code (the line for a machine with no curl)"
run laptop "$IMG"; docker exec sblab-laptop useradd -m -s /bin/bash robin
docker exec sblab-laptop sh -c 'mv /usr/bin/curl /usr/bin/curl.off'
seed_chat sblab-laptop robin "Write the release notes"
LINE=$(code_line "linux (no curl)")
as sblab-laptop robin "$LINE --label Laptop" | sed 's/^/    /'

step "A wrong code is refused with a clear message"
out=$(as sblab-studio sam 'curl -fsSL http://server:8131/install-agent.sh | bash -s -- --server server --code ZZZZZZ' 2>&1)
printf '%s\n' "$out" | tail -2 | sed 's/^/    /'
case "$out" in *"wrong or has expired"*) ok "wrong code: the hub's message reaches the person" ;; *) bad "wrong code message" ;; esac

# ---------------------------------------------------------------- a machine over ssh
step "Tower: added over ssh"
run tower "$IMG"; docker exec sblab-tower useradd -m -s /bin/bash kai
docker exec sblab-tower sh -c 'mkdir -p /run/sshd && chmod 755 /run/sshd && ssh-keygen -A >/dev/null && /usr/sbin/sshd'
seed_chat sblab-tower kai "Speed up the build"
KEY=$(as sblab-server alex 'python3 .local/share/switchboard/app/install/setup.py --ssh-key')
as sblab-tower kai "curl -fsSL http://server:8131/install-agent.sh | bash -s -- --server server --ssh-key \"$KEY\"" | sed 's/^/    /'
as sblab-server alex 'python3 .local/share/switchboard/app/install/setup.py --ssh tower=kai@tower' | sed 's/^/    /'
check "the key only runs the agent (a shell command is not run)" bash -c \
  "! docker exec sblab-server su - alex -c 'ssh -i .ssh/switchboard_ed25519 -o BatchMode=yes kai@tower touch /tmp/x' >/dev/null 2>&1 || ! docker exec sblab-tower test -e /tmp/x"

# ---------------------------------------------------------------- the interactive setup, in a terminal
# The server picks up linked machines on its next poll; wait for them, so Desk is the only new one below.
for _ in $(seq 1 30); do
  n=$(api /api/machines | jq_py "len(d['machines'])" 2>/dev/null || echo 0); [ "$n" = 3 ] && break; sleep 2
done
step "Desk: a person runs setup in a terminal (address, Add a machine with a code, folder picker)"
run desk "$IMG"; docker exec sblab-desk useradd -m -s /bin/bash lee
docker exec sblab-desk su - lee -c 'mkdir -p Desktop/Claude/site Desktop/Notes'
seed_chat sblab-desk lee "Plan the launch"
python3 "$HERE/drive-setup.py" sblab-server alex sblab-desk lee Desk > "$WORK/drive.log" 2>&1; rc=$?
sed 's/^/    /; s/\r$//' "$WORK/drive.log" | grep -v '^    $'
[ $rc = 0 ] && ok "interactive setup: every step came and answered" || bad "interactive setup (see above)"

# ---------------------------------------------------------------- what the server sees
step "Waiting for the server's next poll"
for _ in $(seq 1 30); do
  n=$(api /api/machines | jq_py "sum(1 for m in d['machines'] if m['online'])" 2>/dev/null || echo 0)
  [ "$n" = 4 ] && break; sleep 2
done
api /api/machines | jq_py "'\n'.join('    %-8s %-6s via %-4s online=%s os=%s' % (m['name'], m['label'], m['via'], m['online'], m['os']) for m in d['machines'])"
for m in studio laptop tower desk; do
  check "$m is online" sh -c "docker exec sblab-server su - alex -c 'python3 -c \"import json, urllib.request; d = json.load(urllib.request.urlopen(\\\"http://127.0.0.1:8130/api/machines\\\")); assert [x for x in d[\\\"machines\\\"] if x[\\\"name\\\"] == \\\"$m\\\" and x[\\\"online\\\"]]\"'"
done
for t in "Fix the login page" "Write the release notes" "Speed up the build" "Plan the launch"; do
  check "session listed: $t" sh -c "docker exec sblab-server su - alex -c 'curl -fsS http://127.0.0.1:8130/api/sessions' | grep -q \"$t\""
done

step "Claude folders"
as sblab-server alex 'python3 .local/share/switchboard/app/install/setup.py --root studio=Code --root laptop --root tower=~/Code/app' | sed 's/^/    /'
roots=$(api /api/machines | jq_py "' '.join('%s=%s' % (m['name'], m['root']) for m in d['machines'])")
printf '    %s\n' "$roots"
case " $roots " in *" studio=~/Code "*) ok "studio's folder: a bare name is read as ~/Code" ;; *) bad "studio's folder" ;; esac
case " $roots " in *" laptop=~ "*) ok "laptop's folder: home, picked by default (no Desktop/Claude there)" ;; *) bad "laptop's folder" ;; esac
case " $roots " in *" tower=~/Code/app "*) ok "tower's folder: as typed (an unquoted ~ the server's shell expanded is undone)" ;; *) bad "tower's folder" ;; esac
case " $roots " in *" desk=~/Desktop/Claude/site "*) ok "desk's folder: picked by browsing" ;; *) bad "desk's folder" ;; esac

# ---------------------------------------------------------------- restart, update, uninstall
step "Restart and update keep everything"
as sblab-server alex '.local/bin/switchboard restart' | sed 's/^/    /'
as sblab-server alex 'bash src/scripts/install/install-server.sh --hub-binary /lab/dist/switchboard-hub-linux --node-dir /lab/dist --yes' >/dev/null
for _ in $(seq 1 30); do
  n=$(api /api/machines 2>/dev/null | jq_py "sum(1 for m in d['machines'] if m['online'])" 2>/dev/null || echo 0)
  [ "$n" = 4 ] && break; sleep 2
done
check "after a restart and a second install, all four machines are back online" test "$n" = 4

step "No Python on the machine"
docker run -d --init --name sblab-bare --network "$NET" -v "$WORK:/lab:ro" ubuntu:24.04 sleep infinity >/dev/null
out=$(docker exec sblab-bare bash -c 'tar -xf /lab/repo.tar -C /tmp && bash /tmp/scripts/install/install-agent.sh --server server --code ABCDEF' 2>&1); rc=$?
printf '%s\n' "$out" | sed 's/^/    /'
[ $rc = 1 ] && case "$out" in *"sudo apt-get install -y python3"*) true ;; *) false ;; esac && ok "no Python: says how to install it and stops" || bad "no Python message"

step "Uninstall"
as sblab-server alex 'bash src/scripts/install/install-server.sh --uninstall' | sed 's/^/    /'
check "the server is stopped" bash -c "! docker exec sblab-server su - alex -c 'curl -fsS http://127.0.0.1:8130/api/summary'"
check "settings are kept" as sblab-server alex 'test -f .config/switchboard/fleet.json'

# ---------------------------------------------------------------- the same, with systemd
step "Systemd: install-server.sh as systemd user services, and the server machine linked to itself"
SD=switchboard-lab-systemd:ubuntu24
if ! docker image inspect "$SD" >/dev/null 2>&1; then
  docker rm -f sblab-sdbase >/dev/null 2>&1
  docker run -d --name sblab-sdbase --network host ${CA:+-v $CA:/ca.crt:ro} "$IMG" sleep infinity >/dev/null
  docker exec -e https_proxy="${https_proxy:-${HTTPS_PROXY:-}}" sblab-sdbase sh -c '
    O=""; [ -f /ca.crt ] && O="-o Acquire::https::CAInfo=/ca.crt"
    apt-get $O update -qq >/dev/null && DEBIAN_FRONTEND=noninteractive apt-get $O install -y -qq --no-install-recommends \
      systemd systemd-sysv dbus dbus-user-session libpam-systemd >/dev/null; apt-get clean; rm -rf /var/lib/apt/lists/*' &&
    docker commit sblab-sdbase "$SD" >/dev/null
  docker rm -f sblab-sdbase >/dev/null
fi
docker run -d --name sblab-sd --hostname sdbox --network "$NET" --privileged --cgroupns=host -v /sys/fs/cgroup:/sys/fs/cgroup:rw \
  --tmpfs /run --tmpfs /run/lock -v "$WORK:/lab:ro" "$SD" /sbin/init >/dev/null
for _ in $(seq 1 30); do docker exec sblab-sd systemctl is-system-running 2>/dev/null | grep -Eq 'running|degraded' && break; sleep 1; done
docker exec sblab-sd sh -c 'useradd -m -s /bin/bash alex && loginctl enable-linger alex'
for _ in $(seq 1 20); do docker exec sblab-sd test -S /run/user/1001/bus && break; sleep 1; done
sd() { docker exec sblab-sd su - alex -c "export XDG_RUNTIME_DIR=/run/user/\$(id -u); $*"; }
sd 'mkdir -p src && tar -xf /lab/repo.tar -C src && bash src/scripts/install/install-server.sh --hub-binary /lab/dist/switchboard-hub-linux --node-dir /lab/dist --yes' | tail -5 | sed 's/^/    /'
check "systemd: the three user services are active and enabled" sd 'systemctl --user is-active switchboard switchboard-hub switchboard-downloads && systemctl --user is-enabled switchboard switchboard-hub switchboard-downloads'
sd 'python3 .local/share/switchboard/app/install/setup.py --address sdbox --yes' >/dev/null
C=$(sd 'python3 .local/share/switchboard/app/install/setup.py --code --json' | jq_py "d['code']")
sd "curl -fsSL http://sdbox:8131/install-agent.sh | bash -s -- --server sdbox --code $C --label Server" | tail -3 | sed 's/^/    /'
check "systemd: switchboard-node runs as a user service" sd 'systemctl --user is-active switchboard-node'
for _ in $(seq 1 30); do
  sd 'curl -fsS 127.0.0.1:8130/api/machines' 2>/dev/null | grep -q '"online": true' && break; sleep 2
done
check "systemd: the server machine is one of its own machines, online" sh -c "docker exec sblab-sd su - alex -c 'curl -fsS 127.0.0.1:8130/api/machines' | grep -q '\"online\": true'"
sd 'bash src/scripts/install/install-server.sh --uninstall' >/dev/null
check "systemd: uninstall removes the units" sd '! systemctl --user is-active switchboard && test ! -e .config/systemd/user/switchboard.service'

printf '\n%d passed, %d failed\n' "$PASS" "$FAIL"
[ "$FAIL" = 0 ]
