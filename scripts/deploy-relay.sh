#!/usr/bin/env bash
# Install and run your own iroh relay (iroh-relay 1.5.0) on a Linux host, as a systemd service.
# Switchboard's computers and phones meet through a relay when they cannot reach each other directly.
# scripts/relay/README.md has sizing, ports, DNS and cost.
#
#   scripts/deploy-relay.sh [HOST] install --hostname relay.example.com --contact you@example.com
#   scripts/deploy-relay.sh [HOST] install --dev [--port 3340] [--user]
#   scripts/deploy-relay.sh [HOST] status | logs | uninstall
#   scripts/deploy-relay.sh config|unit [install options]     print what install would write, change nothing
#
# HOST is an ssh target; without it the script acts on the machine it runs on, and on nothing else.
#
# install options
#   --hostname NAME    production: TLS from Let's Encrypt for NAME. Needs TCP 80 and 443 and UDP 7842 open, and
#                      NAME pointing at this host. Needs root.
#   --contact EMAIL    production: the address Let's Encrypt keeps for the certificate.
#   --dev              plain HTTP, no certificate, no QUIC address discovery. For a private or Tailscale
#                      address only: anyone who can reach the port can use the relay.
#   --port N           --dev: the HTTP port (3340).
#   --metrics-port N   metrics on 127.0.0.1:N (9090).
#   --bind ADDR        the address to listen on ([::], which is every address, IPv4 included).
#   --user             a systemd user service in your home folder, no root. Only with --dev.
#   --name NAME        the service name (switchboard-relay). A second relay on one host needs its own.
#   --prefix DIR       --user: where the binary, config and state go (~/.local/share/NAME).
#   --rx-bytes N       per client, bytes per second it may send through the relay (4194304, 4 MiB/s)
#   --rx-burst N       per client, bytes above that rate in one burst (8388608)
#   --conn-rate N      new connections accepted per second (50)
#   --conn-burst N     new connections accepted in one burst (100)
#   --no-limits        leave [limits] out: nothing is rate limited
#   --build            build with cargo (needs Rust 1.91 or newer) instead of downloading the release binary
#
# Running install again with the same options changes nothing; with other options it rewrites the config (the
# old one is kept as relay.toml.bak) and restarts the service.
set -euo pipefail

VERSION=1.5.0
RELEASES=https://github.com/n0-computer/iroh/releases/download
# sha256 of iroh-relay-v1.5.0-<arch>-unknown-linux-musl.tar.gz, from the release page.
SHA_x86_64=935892e4ad570d694f7ad2780f800077b99fd72d9a56ac982792fdd04ef631b4
SHA_aarch64=99a14d2ff541711f1b175a15e0c1571a45f46c171d655cda29849d91d90a5c9c

die() { echo "deploy-relay: $*" >&2; exit 1; }
usage() { sed -n '2,33p' "$0" | sed 's/^# \{0,1\}//'; }

is_verb() { case "$1" in install|status|logs|uninstall|config|unit) return 0 ;; *) return 1 ;; esac; }

case "${1:-}" in
  ''|-h|--help|help) usage; exit 0 ;;
esac

# An ssh target first: run this same script there, with the rest of the arguments.
if ! is_verb "$1"; then
  HOST=$1; shift
  case "$HOST" in -*) die "unknown option $HOST (the first argument is a host or one of: install status logs uninstall config unit)" ;; esac
  [ $# -gt 0 ] && is_verb "$1" || die "after the host, say what to do: install, status, logs, uninstall, config or unit"
  case "$1" in
    config|unit) ;;  # these only print: no need to leave this machine
    *) exec ssh "$HOST" "bash -s -- $(printf '%q ' "$@")" < "$0" ;;
  esac
fi

VERB=$1; shift
MODE='' HOSTNAME_='' CONTACT='' PORT=3340 METRICS_PORT=9090 BIND='[::]' USER_UNIT=0 NAME=switchboard-relay PREFIX=''
RX_BYTES=4194304 RX_BURST=8388608 CONN_RATE=50 CONN_BURST=100 LIMITS=1 BUILD=0

need() { [ $# -ge 2 ] || die "$1 needs a value"; }
number() { case "$2" in ''|*[!0-9]*) die "$1 takes a whole number, not '$2'" ;; esac; }
port() { number "$1" "$2"; [ "$2" -ge 1 ] && [ "$2" -le 65535 ] || die "$1 takes a port from 1 to 65535"; }

while [ $# -gt 0 ]; do
  case "$1" in
    --dev) MODE=dev ;;
    --hostname) need "$@"; HOSTNAME_=$2; shift ;;
    --contact) need "$@"; CONTACT=$2; shift ;;
    --port) need "$@"; port "$1" "$2"; PORT=$2; shift ;;
    --metrics-port) need "$@"; port "$1" "$2"; METRICS_PORT=$2; shift ;;
    --bind) need "$@"; BIND=$2; shift ;;
    --user) USER_UNIT=1 ;;
    --name) need "$@"; NAME=$2; shift ;;
    --prefix) need "$@"; PREFIX=$2; shift ;;
    --rx-bytes) need "$@"; number "$1" "$2"; RX_BYTES=$2; shift ;;
    --rx-burst) need "$@"; number "$1" "$2"; RX_BURST=$2; shift ;;
    --conn-rate) need "$@"; number "$1" "$2"; CONN_RATE=$2; shift ;;
    --conn-burst) need "$@"; number "$1" "$2"; CONN_BURST=$2; shift ;;
    --no-limits) LIMITS=0 ;;
    --build) BUILD=1 ;;
    -h|--help) usage; exit 0 ;;
    *) die "unknown option $1" ;;
  esac
  shift
done

case "$NAME" in ''|*[!a-z0-9-]*|-*) die "--name takes lowercase letters, digits and dashes" ;; esac
case "$BIND" in *[!0-9a-fA-F:.\[\]]*|'') die "--bind takes an IP address, for example 203.0.113.7 or [::]" ;; esac
case "$BIND" in *:*) case "$BIND" in \[*\]) ;; *) BIND="[$BIND]" ;; esac ;; esac
case "$PREFIX" in *[\"\\\$\`]*|*'
'*) die "--prefix has a character a config file cannot hold" ;; esac

check_install_args() {
  if [ -n "$HOSTNAME_" ]; then
    [ "$MODE" != dev ] || die "--dev and --hostname do not go together: --dev serves plain HTTP"
    MODE=prod
    case "$HOSTNAME_" in *[!A-Za-z0-9.-]*|.*|*.|-*) die "--hostname takes a DNS name, for example relay.example.com" ;; esac
    case "$HOSTNAME_" in *.*) ;; *) die "--hostname takes a full DNS name, for example relay.example.com" ;; esac
    [ -n "$CONTACT" ] || die "--contact is needed with --hostname: Let's Encrypt wants an email address"
    case "$CONTACT" in *[!A-Za-z0-9.@_+-]*) die "--contact takes an email address" ;; *?@?*.?*) ;; *) die "--contact takes an email address" ;; esac
    [ "$USER_UNIT" = 0 ] || die "--user only works with --dev: ports 80 and 443 need root"
  fi
  [ -n "$MODE" ] || die "say --dev, or --hostname NAME --contact EMAIL"
  if [ "$LIMITS" = 1 ]; then
    [ "$RX_BYTES" -ge 1 ] && [ "$RX_BURST" -ge 1 ] && [ "$CONN_RATE" -ge 1 ] && [ "$CONN_BURST" -ge 1 ] \
      || die "limits are 1 or more; use --no-limits for none"
  fi
}

# Where things live. A system service: root owns the binary and config, the relay runs as its own user.
paths() {
  if [ "$USER_UNIT" = 1 ]; then
    ROOT=${PREFIX:-$HOME/.local/share/$NAME}
    BIN=$ROOT/iroh-relay CONF=$ROOT/relay.toml STATE=$ROOT/state
    UNIT=${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user/$NAME.service
    SYSTEMCTL="systemctl --user" JOURNAL="journalctl --user"
  else
    [ -z "$PREFIX" ] || die "--prefix only works with --user"
    BIN=/opt/$NAME/iroh-relay CONF=/etc/$NAME/relay.toml STATE=/var/lib/$NAME
    UNIT=/etc/systemd/system/$NAME.service
    SYSTEMCTL=systemctl JOURNAL=journalctl
  fi
  RUN_USER=sb-relay
}

config_text() {
  echo "# Written by scripts/deploy-relay.sh for iroh-relay $VERSION. Edits are overwritten by the next install."
  if [ "$MODE" = dev ]; then
    echo "http_bind_addr = \"$BIND:$PORT\""
  else
    echo "http_bind_addr = \"$BIND:80\""
    echo "enable_quic_addr_discovery = true"
  fi
  echo "metrics_bind_addr = \"127.0.0.1:$METRICS_PORT\""
  echo 'access = "everyone"'
  if [ "$MODE" = prod ]; then
    echo
    echo "[tls]"
    echo "https_bind_addr = \"$BIND:443\""
    echo "quic_bind_addr = \"$BIND:7842\""
    echo "hostname = \"$HOSTNAME_\""
    echo 'cert_mode = "LetsEncrypt"'
    echo "contact = \"$CONTACT\""
    echo "cert_dir = \"$STATE/certs\""
  fi
  if [ "$LIMITS" = 1 ]; then
    echo
    echo "[limits]"
    echo "accept_conn_limit = $CONN_RATE.0"
    echo "accept_conn_burst = $CONN_BURST"
    echo
    echo "[limits.client.rx]"
    echo "bytes_per_second = $RX_BYTES"
    echo "max_burst_bytes = $RX_BURST"
  fi
}

unit_text() {
  local dev=''
  [ "$MODE" = dev ] && dev=' --dev'
  echo "[Unit]"
  echo "Description=Switchboard relay (iroh-relay $VERSION)"
  echo "After=network-online.target"
  echo "Wants=network-online.target"
  echo
  echo "[Service]"
  echo "ExecStart=\"$BIN\" --config-path \"$CONF\"$dev"
  echo "WorkingDirectory=$STATE"
  echo "Restart=on-failure"
  echo "RestartSec=5"
  echo "KillSignal=SIGINT"  # the relay shuts down cleanly on Ctrl-C, not on SIGTERM
  echo "LimitNOFILE=65536"
  if [ "$USER_UNIT" = 0 ]; then
    echo "User=$RUN_USER"
    echo "Group=$RUN_USER"
    echo "AmbientCapabilities=CAP_NET_BIND_SERVICE"
    echo "CapabilityBoundingSet=CAP_NET_BIND_SERVICE"
    echo "NoNewPrivileges=true"
    echo "ProtectSystem=strict"
    echo "ProtectHome=true"
    echo "PrivateTmp=true"
    echo "PrivateDevices=true"
    echo "ReadWritePaths=$STATE"
  fi
  echo
  echo "[Install]"
  if [ "$USER_UNIT" = 1 ]; then echo "WantedBy=default.target"; else echo "WantedBy=multi-user.target"; fi
}

need_linux() {
  [ "$(uname -s)" = Linux ] || die "this installs on Linux; give an ssh host as the first argument to install there"
  command -v systemctl >/dev/null 2>&1 || die "systemd (systemctl) was not found on this host"
}
need_root() {
  [ "$USER_UNIT" = 1 ] || [ "$(id -u)" = 0 ] || die "run this as root (sudo), or add --user with --dev for a service in your own account"
}

sha256_of() {
  if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | cut -d' ' -f1; else shasum -a 256 "$1" | cut -d' ' -f1; fi
}

# The release binary, checked against the hash above; or a cargo build. Skipped when the right binary is there.
fetch_binary() {
  if [ -x "$BIN" ] && [ "$("$BIN" --version 2>/dev/null)" = "iroh-relay $VERSION" ]; then return 1; fi
  local tmp arch want got
  tmp=$(mktemp -d)
  # shellcheck disable=SC2064
  trap "rm -rf '$tmp'" EXIT
  if [ "$BUILD" = 1 ]; then
    command -v cargo >/dev/null 2>&1 || die "--build needs cargo (Rust 1.91 or newer) on this host"
    cargo install --locked iroh-relay --version "$VERSION" --features server --root "$tmp/root"
    cp "$tmp/root/bin/iroh-relay" "$tmp/iroh-relay"
  else
    arch=$(uname -m)
    case "$arch" in
      x86_64|amd64) arch=x86_64; want=$SHA_x86_64 ;;
      aarch64|arm64) arch=aarch64; want=$SHA_aarch64 ;;
      *) die "no release binary for $arch: use --build" ;;
    esac
    curl -fsSL -o "$tmp/relay.tgz" "$RELEASES/v$VERSION/iroh-relay-v$VERSION-$arch-unknown-linux-musl.tar.gz" \
      || die "could not download iroh-relay $VERSION"
    got=$(sha256_of "$tmp/relay.tgz")
    [ "$got" = "$want" ] || die "the download does not match the expected checksum (got $got)"
    tar -xzf "$tmp/relay.tgz" -C "$tmp" ./iroh-relay
  fi
  mkdir -p "$(dirname "$BIN")"
  install -m 0755 "$tmp/iroh-relay" "$BIN.new"
  mv -f "$BIN.new" "$BIN"
  rm -rf "$tmp"; trap - EXIT
  return 0
}

# Write $2 to file $1 when it differs. Returns 0 when the file changed.
write_if_changed() {
  local file=$1 text=$2 mode=$3
  if [ -f "$file" ] && [ "$(cat "$file")" = "$text" ]; then return 1; fi
  mkdir -p "$(dirname "$file")"
  [ -f "$file" ] && cp -p "$file" "$file.bak"
  printf '%s\n' "$text" > "$file.new"
  chmod "$mode" "$file.new"
  mv -f "$file.new" "$file"
  return 0
}

health_url() {
  if [ "$MODE" = dev ]; then
    case "$BIND" in '[::]'|0.0.0.0) echo "http://127.0.0.1:$PORT/healthz" ;; *) echo "http://$BIND:$PORT/healthz" ;; esac
  else
    echo "https://$HOSTNAME_/healthz"
  fi
}

do_install() {
  check_install_args; need_linux; need_root; paths
  command -v curl >/dev/null 2>&1 || die "curl was not found on this host"
  local changed=0
  if [ "$USER_UNIT" = 0 ] && ! id "$RUN_USER" >/dev/null 2>&1; then
    useradd --system --no-create-home --home-dir /nonexistent --shell /usr/sbin/nologin "$RUN_USER"
  fi
  if fetch_binary; then changed=1; fi
  mkdir -p "$STATE"
  if [ "$USER_UNIT" = 0 ]; then
    chown "$RUN_USER:$RUN_USER" "$STATE"; chmod 0750 "$STATE"
    if [ "$MODE" = prod ]; then mkdir -p "$STATE/certs"; chown "$RUN_USER:$RUN_USER" "$STATE/certs"; chmod 0700 "$STATE/certs"; fi
  fi
  if write_if_changed "$CONF" "$(config_text)" 0644; then changed=1; fi
  if write_if_changed "$UNIT" "$(unit_text)" 0644; then changed=1; rm -f "$UNIT.bak"; $SYSTEMCTL daemon-reload; fi
  $SYSTEMCTL enable "$NAME" >/dev/null 2>&1 || true
  if [ "$changed" = 1 ]; then $SYSTEMCTL restart "$NAME"; else $SYSTEMCTL start "$NAME"; fi

  local url i=0 ok=0 tries=10
  url=$(health_url)
  [ "$MODE" = prod ] && tries=45  # the first certificate takes a moment
  while [ $i -lt $tries ]; do
    if curl -fsS --max-time 5 "$url" >/dev/null 2>&1; then ok=1; break; fi
    $SYSTEMCTL is-active --quiet "$NAME" || break
    sleep 2; i=$((i + 1))
  done
  if [ "$ok" = 1 ]; then
    echo "iroh-relay $VERSION is running as $NAME ($( [ "$changed" = 1 ] && echo updated || echo "no change" ))."
    if [ "$MODE" = dev ]; then
      echo "Relay address: http://<this host's private or Tailscale address>:$PORT"
    else
      echo "Relay address: https://$HOSTNAME_"
    fi
    return 0
  fi
  echo "The relay did not answer at $url." >&2
  if [ "$MODE" = prod ]; then
    echo "Check that $HOSTNAME_ points at this host and that TCP 80 and 443 are open, then look at the logs:" >&2
  else
    echo "Look at the logs:" >&2
  fi
  echo "  scripts/deploy-relay.sh logs$( [ "$USER_UNIT" = 1 ] && echo ' --user' )$( [ "$NAME" = switchboard-relay ] || echo " --name $NAME" )" >&2
  exit 1
}

do_status() {
  need_linux; paths
  [ -f "$UNIT" ] || die "$NAME is not installed here$( [ "$USER_UNIT" = 1 ] || echo ' as a system service (add --user for one in your account)' )"
  $SYSTEMCTL --no-pager status "$NAME" || true
  local http metrics
  http=$(sed -n 's/^http_bind_addr = "\(.*\)"$/\1/p' "$CONF" 2>/dev/null | head -1)
  metrics=$(sed -n 's/^metrics_bind_addr = "\(.*\)"$/\1/p' "$CONF" 2>/dev/null | head -1)
  if [ -n "$metrics" ] && command -v curl >/dev/null 2>&1; then
    echo
    echo "Listening on $http. From $metrics/metrics:"
    curl -fsS --max-time 5 "http://$metrics/metrics" 2>/dev/null \
      | grep -E '^relayserver_(accepts|bytes_sent|bytes_recv|bytes_rx_ratelimited|unique_client_keys)(_total)? ' \
      || echo "  (metrics did not answer)"
  fi
}

do_logs() {
  need_linux; paths
  exec $JOURNAL --no-pager -n 200 -u "$NAME"
}

do_uninstall() {
  need_linux; need_root; paths
  $SYSTEMCTL disable --now "$NAME" >/dev/null 2>&1 || true
  rm -f "$UNIT" "$UNIT.bak"
  $SYSTEMCTL daemon-reload || true
  if [ "$USER_UNIT" = 1 ]; then
    rm -f "$BIN" "$CONF" "$CONF.bak"
    rm -rf "$STATE"
    rmdir "$ROOT" 2>/dev/null || true
  else
    rm -rf "/opt/$NAME" "/etc/$NAME" "$STATE"
    # The user goes only when no other relay service here still runs as it.
    if ! grep -ls "^User=$RUN_USER\$" /etc/systemd/system/*.service >/dev/null 2>&1; then
      userdel "$RUN_USER" >/dev/null 2>&1 || true
    fi
  fi
  echo "$NAME is removed."
}

case "$VERB" in
  install) do_install ;;
  status) do_status ;;
  logs) do_logs ;;
  uninstall) do_uninstall ;;
  config) check_install_args; paths; config_text ;;
  unit) check_install_args; paths; unit_text ;;
esac
