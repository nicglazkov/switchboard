import json, sys, threading, time, unittest
from pathlib import Path
from tempfile import mkdtemp
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'server'))
import addresses, server  # noqa: E402

IP_ADDR = """1: lo    inet 127.0.0.1/8 scope host lo\\       valid_lft forever preferred_lft forever
2: eth0    inet 192.0.2.10/24 brd 192.0.2.255 scope global dynamic eth0\\       valid_lft 85000sec
3: docker0    inet 10.0.0.1/24 brd 10.0.0.255 scope global docker0\\       valid_lft forever
4: br-1a2b3c    inet 10.0.0.2/24 brd 10.0.0.255 scope global br-1a2b3c
5: tailscale0    inet 100.64.0.10/32 scope global tailscale0
6: wlan0    inet 169.254.10.20/16 brd 169.254.255.255 scope link wlan0
7: eth1    inet 192.0.2.11/24 brd 192.0.2.255 scope global eth1
8: veth9f@if7    inet 10.0.0.9/24 scope global veth9f
"""

IFCONFIG = """lo0: flags=8049<UP,LOOPBACK,RUNNING,MULTICAST> mtu 16384
\tinet 127.0.0.1 netmask 0xff000000
en0: flags=8863<UP,BROADCAST,SMART,RUNNING,SIMPLEX,MULTICAST> mtu 1500
\tether 00:00:5e:00:53:01
\tinet6 fe80::1%en0 prefixlen 64 secured scopeid 0x4
\tinet 192.0.2.10 netmask 0xffffff00 broadcast 192.0.2.255
bridge100: flags=8863<UP,BROADCAST,SMART,RUNNING,SIMPLEX,MULTICAST> mtu 1500
\tinet 192.0.2.99 netmask 0xffffff00 broadcast 192.0.2.255
utun4: flags=8051<UP,POINTOPOINT,RUNNING,MULTICAST> mtu 1280
\tinet 100.64.0.10 --> 100.64.0.10 netmask 0xffffffff
"""

TS_UP = json.dumps({'BackendState': 'Running',
                    'Self': {'DNSName': 'studio.tail0000.ts.net.', 'TailscaleIPs': ['100.64.0.10', 'fd7a:115c:a1e0::1']}})
TS_DOWN = json.dumps({'BackendState': 'Stopped', 'Self': {'DNSName': 'studio.tail0000.ts.net.', 'TailscaleIPs': []}})


class Done:
    def __init__(self, stdout):
        self.stdout = stdout


def fake(tools, ts=TS_UP, ip=IP_ADDR, ifconfig=IFCONFIG, calls=None, delay=0):
    def which(name):
        return f'/usr/bin/{name}' if name in tools else None

    def run(args, **kw):
        if calls is not None:
            calls.append(args[0])
        if args[0] == 'tailscale':
            time.sleep(delay)
            return Done(ts)
        return Done({'ip': ip, 'ifconfig': ifconfig}[args[0]])
    return which, run


def make(tools=('ip', 'tailscale'), allowed=None, bind=None, **kw):
    which, run = fake(tools, **kw)
    a = addresses.Addresses(8130, allowed=allowed, bind=bind, run=run, which=which, hostname=lambda: 'Studio.example')
    a.route_ip = lambda: None
    return a


def hosts(v):
    return [(x['kind'], x['host']) for x in v['list']]


class ParseTest(unittest.TestCase):
    def test_ip_addr(self):
        pairs = addresses.parse_ip_addr(IP_ADDR)
        self.assertIn(('eth0', '192.0.2.10'), pairs)
        self.assertIn(('veth9f', '10.0.0.9'), pairs, 'the @peer part is not the name')

    def test_ifconfig(self):
        pairs = addresses.parse_ifconfig(IFCONFIG)
        self.assertEqual(pairs, [('lo0', '127.0.0.1'), ('en0', '192.0.2.10'), ('bridge100', '192.0.2.99'),
                                 ('utun4', '100.64.0.10')])

    def test_tailscale(self):
        self.assertEqual(addresses.parse_tailscale(TS_UP), (['100.64.0.10'], 'studio.tail0000.ts.net'))
        self.assertEqual(addresses.parse_tailscale(TS_DOWN), ([], None))
        self.assertEqual(addresses.parse_tailscale('not json'), ([], None))
        self.assertEqual(addresses.parse_tailscale('[]'), ([], None))


class ListTest(unittest.TestCase):
    def test_every_kind_and_nothing_else(self):
        v = make().refresh()
        self.assertEqual(v['port'], 8130)
        self.assertEqual(hosts(v), [('lan', '192.0.2.10'), ('lan', '192.0.2.11'), ('bonjour', 'studio.local'),
                                    ('tailscale', '100.64.0.10'), ('magicdns', 'studio.tail0000.ts.net')])
        self.assertEqual(v['list'][0]['url'], 'http://192.0.2.10:8130')
        self.assertEqual(v['list'][-1]['url'], 'http://studio.tail0000.ts.net:8130')

    def test_mac_ifconfig_skips_bridges_and_tunnels(self):
        v = make(tools=('ifconfig',)).refresh()
        self.assertEqual(hosts(v), [('lan', '192.0.2.10'), ('bonjour', 'studio.local')])

    def test_no_tailscale_or_tailscale_down(self):
        self.assertNotIn('tailscale', [k for k, _ in hosts(make(tools=('ip',)).refresh())])
        self.assertEqual([k for k, _ in hosts(make(ts=TS_DOWN).refresh())], ['lan', 'lan', 'bonjour'])

    def test_the_allow_list_has_the_last_word(self):
        v = make(allowed=lambda h: not h.endswith('.ts.net') and h != '192.0.2.11').refresh()
        self.assertEqual(hosts(v), [('lan', '192.0.2.10'), ('bonjour', 'studio.local'), ('tailscale', '100.64.0.10')])

    def test_with_the_real_server_config(self):
        d = Path(mkdtemp())
        (d / 'server.json').write_text(json.dumps({'bind': ['0.0.0.0'], 'hosts': ['192.0.2.10'], 'origins': [],
                                                    'allow': ['tailscale']}))
        cfg = server.ServerConfig(d, d)
        v = make(allowed=cfg.host_ok, bind=cfg.bind).refresh()
        self.assertEqual(hosts(v), [('lan', '192.0.2.10'), ('tailscale', '100.64.0.10'),
                                    ('magicdns', 'studio.tail0000.ts.net')],
                         'a LAN address it does not list and .local names (not allowed) are left out')

    def test_bound_to_some_addresses_only(self):
        v = make(bind=['192.0.2.11']).refresh()
        self.assertEqual(hosts(v), [('lan', '192.0.2.11'), ('bonjour', 'studio.local')])

    def test_no_tool_falls_back_to_the_route(self):
        a = make(tools=())
        a.route_ip = lambda: '192.0.2.12'
        self.assertEqual(hosts(a.refresh()), [('lan', '192.0.2.12'), ('bonjour', 'studio.local')])
        a.route_ip = lambda: '127.0.0.1'
        self.assertEqual(hosts(a.refresh()), [], 'no LAN address: no .local name either')

    def test_a_tool_that_fails_is_no_answer(self):
        def run(args, **kw):
            raise OSError('gone')
        a = addresses.Addresses(8130, run=run, which=lambda n: '/x', hostname=lambda: 'studio')
        a.route_ip = lambda: None
        self.assertEqual(a.refresh(), {'port': 8130, 'list': []})


class CacheTest(unittest.TestCase):
    def test_get_never_waits_and_keeps_the_answer(self):
        calls = []
        a = make(calls=calls, delay=0.3)
        t = time.time()
        first = a.get()
        self.assertLess(time.time() - t, 0.1, 'a slow tailscale must not hold up the request')
        self.assertEqual(first, {'port': 8130, 'list': []})
        for _ in range(50):
            if a.get()['list']:
                break
            time.sleep(0.05)
        self.assertEqual(len(a.get()['list']), 5)
        n = len(calls)
        a.get()
        a.get()
        time.sleep(0.05)
        self.assertEqual(len(calls), n, 'kept for the ttl: no new look')

    def test_a_stale_answer_is_served_while_it_looks_again(self):
        now = [1000.0]
        which, run = fake(('ip',))
        a = addresses.Addresses(8130, run=run, which=which, hostname=lambda: 'studio', clock=lambda: now[0], ttl=60)
        a.refresh()
        old = a.get()
        now[0] += 61
        self.assertEqual(a.get(), old)
        for t in threading.enumerate():
            if t.name == 'sb-addresses':
                t.join(2)
        self.assertFalse(a._busy)


class ServerTest(unittest.TestCase):
    def test_summary_carries_the_addresses_on_a_real_server_only(self):
        import fleet
        d = Path(mkdtemp())
        f = fleet.Fleet([], lambda *a: {'ok': True, 'sessions': []}, clock=lambda: 1000.0)
        app = server.App(f, d / 'state', d / 'web', cfg_dir=d / 'cfg')
        self.assertNotIn('addresses', app.summary())
        app.addresses = make()
        app.addresses.refresh()
        self.assertEqual(app.summary()['addresses']['list'][0]['host'], '192.0.2.10')
        self.assertEqual(app.sessions()['summary']['addresses']['port'], 8130)


if __name__ == '__main__':
    unittest.main()
