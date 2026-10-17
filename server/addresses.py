"""The addresses this server can be reached at, for the apps to remember. GET /api/summary (and the summary inside
/api/sessions) carries them as "addresses": {"port": 8130, "list": [{"kind", "host", "url"}, ...]}, so an app that
reached the server once at home also knows its Tailscale address for when it is away.

Kinds: "lan" (an IPv4 address on a local network), "bonjour" (<host>.local), "tailscale" (the tailnet IPv4) and
"magicdns" (the tailnet name). Left out: loopback, link-local, container and bridge interfaces, and anything the
server's own allow list would refuse. Standard library only. The answer is kept for a few minutes and worked out on a
thread, so a slow `tailscale` never holds up a request."""
import ipaddress, json, re, shutil, socket, subprocess, threading, time

TTL = 300           # seconds an answer is kept
TOOL_WAIT = 4       # seconds `ip`, `ifconfig` or `tailscale` may take
# Interfaces that are not a way in from another computer: containers, bridges, VMs and VPN tunnels (Tailscale's own
# address comes from the tailscale command instead).
SKIP_IF = re.compile(r'^(lo|docker|br-|veth|virbr|vnet|vmnet|vboxnet|cni|flannel|cali|weave|kube|podman|lxc|lxd|'
                     r'bridge|tailscale|utun|tun|tap|wg|zt|awdl|llw|anpi|ap\d|gif|stf)', re.I)
CGNAT = ipaddress.ip_network('100.64.0.0/10')
LABEL_RE = re.compile(r'[a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?')


def _lan_ip(s):
    """The address when it is a usable LAN IPv4 one, else None."""
    try:
        ip = ipaddress.IPv4Address(s)
    except ValueError:
        return None
    if ip.is_loopback or ip.is_link_local or ip.is_multicast or ip.is_unspecified or ip in CGNAT or not ip.is_private:
        return None
    return str(ip)


def parse_ip_addr(text):
    """(interface, address) pairs from `ip -4 -o addr show`."""
    out = []
    for line in text.splitlines():
        parts = line.split()
        if len(parts) >= 4 and parts[2] == 'inet':
            out.append((parts[1].split('@')[0], parts[3].split('/')[0]))
    return out


def parse_ifconfig(text):
    """(interface, address) pairs from `ifconfig` on a Mac, BSD or older Linux."""
    out, name = [], None
    for line in text.splitlines():
        if line and not line[0].isspace():
            name = line.split(':')[0].split()[0]
            continue
        parts = line.split()
        if name and len(parts) >= 2 and parts[0] == 'inet':
            out.append((name, parts[1].replace('addr:', '')))
    return out


def parse_tailscale(text):
    """([IPv4 addresses], MagicDNS name or None) from `tailscale status --json`, when Tailscale is up."""
    try:
        d = json.loads(text)
    except ValueError:
        return [], None
    if not isinstance(d, dict) or d.get('BackendState') != 'Running':
        return [], None
    me = d.get('Self') if isinstance(d.get('Self'), dict) else {}
    ips = []
    for x in me.get('TailscaleIPs') or []:
        try:
            if isinstance(x, str) and ipaddress.ip_address(x).version == 4:
                ips.append(x)
        except ValueError:
            pass
    name = me.get('DNSName')
    name = name.rstrip('.').lower() if isinstance(name, str) and name.rstrip('.').endswith('.ts.net') else None
    return ips, name


class Addresses:
    """`get()` answers at once from what was last worked out, and starts working it out again when that is older than
    `ttl` (or there is nothing yet). `allowed(host)` is the server's own Host check; `bind` its listen addresses."""

    def __init__(self, port, allowed=None, bind=None, run=subprocess.run, which=shutil.which,
                 hostname=socket.gethostname, clock=time.time, ttl=TTL):
        self.port, self.allowed, self.run, self.which = port, allowed, run, which
        self.hostname, self.clock, self.ttl = hostname, clock, ttl
        self.bind = [b for b in (bind or []) if b not in ('0.0.0.0', '::', '')]
        self._lock = threading.Lock()
        self._value, self._at, self._busy = None, None, False

    def get(self):
        with self._lock:
            stale = self._at is None or self.clock() - self._at >= self.ttl
            if stale and not self._busy:
                self._busy = True
                threading.Thread(target=self._refresh_bg, name='sb-addresses', daemon=True).start()
            return self._value if self._value is not None else {'port': self.port, 'list': []}

    def _refresh_bg(self):
        try:
            self.refresh()
        finally:
            with self._lock:
                self._busy = False

    def refresh(self):
        """Works the list out now (it may wait on the tools) and keeps it."""
        try:
            v = self.compute()
        except Exception:  # never let a surprise from a tool take the summary down
            v = {'port': self.port, 'list': []}
        with self._lock:
            self._value, self._at = v, self.clock()
        return v

    def _out(self, args):
        try:
            r = self.run(args, capture_output=True, text=True, timeout=TOOL_WAIT)
            return r.stdout or ''
        except (OSError, subprocess.SubprocessError):
            return ''

    def interfaces(self):
        """(interface, address) pairs, from whichever tool this system has."""
        if self.which('ip'):
            pairs = parse_ip_addr(self._out(['ip', '-4', '-o', 'addr', 'show']))
            if pairs:
                return pairs
        if self.which('ifconfig'):
            return parse_ifconfig(self._out(['ifconfig']))
        return []

    def route_ip(self):
        """The address this machine would use to reach other networks, when no tool listed any."""
        try:
            with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as u:  # no packet is sent: it only picks a route
                u.connect(('10.255.255.255', 1))  # leakscan: allow
                return u.getsockname()[0]
        except OSError:
            return None

    def tailscale(self):
        if not self.which('tailscale'):
            return [], None
        return parse_tailscale(self._out(['tailscale', 'status', '--json']))

    def compute(self):
        lan = []
        for name, ip in self.interfaces():
            ip = _lan_ip(ip)
            if ip and not SKIP_IF.match(name) and ip not in lan:
                lan.append(ip)
        if not lan:
            ip = _lan_ip(self.route_ip() or '')
            if ip:
                lan.append(ip)
        ts_ips, ts_name = self.tailscale()
        if self.bind:
            lan = [ip for ip in lan if ip in self.bind]
            ts_ips = [ip for ip in ts_ips if ip in self.bind]
            if not ts_ips:
                ts_name = None
        out = [('lan', ip) for ip in lan]
        short = (self.hostname() or '').split('.')[0].lower()
        if lan and LABEL_RE.fullmatch(short):
            out.append(('bonjour', f'{short}.local'))
        out += [('tailscale', ip) for ip in ts_ips]
        if ts_name:
            out.append(('magicdns', ts_name))
        seen, items = set(), []
        for kind, host in out:
            if host in seen or (self.allowed and not self.allowed(host)):
                continue
            seen.add(host)
            items.append({'kind': kind, 'host': host, 'url': f'http://{host}:{self.port}'})
        return {'port': self.port, 'list': items}
