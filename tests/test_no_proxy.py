"""Calls to this computer and to the server on your own network never go through a proxy.

urllib takes http_proxy from the environment. With one set and no no_proxy, the server's calls to the hub on
127.0.0.1 and the helper's calls to the server went to the proxy, which cannot reach either."""
import json, os, sys, tempfile, threading, unittest, urllib.request
from http.server import BaseHTTPRequestHandler, HTTPServer
from pathlib import Path
from tests.helpers import fresh_sb
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'server'))
import features, nodes, serverctl  # noqa: E402


class Here(BaseHTTPRequestHandler):
    seen = []

    def answer(self):
        n = int(self.headers.get('Content-Length') or 0)
        Here.seen.append((self.command, self.path, self.rfile.read(n)))
        body = json.dumps({'ok': True, 'status': {'day': 'x'}, 'open': [{'machine': 'desk', 'bg': 'abcd1234'}]}).encode()
        self.send_response(200)
        self.send_header('Content-Length', str(len(body)))
        self.end_headers()
        self.wfile.write(body)
    do_GET = do_POST = answer

    def log_message(self, *a):
        pass


class NoProxyTest(unittest.TestCase):
    def setUp(self):
        Here.seen = []
        self.httpd = HTTPServer(('127.0.0.1', 0), Here)
        threading.Thread(target=self.httpd.serve_forever, daemon=True).start()
        self.addCleanup(self.httpd.server_close)
        self.addCleanup(self.httpd.shutdown)
        self.url = f'http://127.0.0.1:{self.httpd.server_address[1]}'
        # a proxy that is not there, and nothing that says to go around it
        # Only these names are touched and put back: fresh_sb sets others, which must outlive this test.
        want = {'http_proxy': 'http://127.0.0.1:9', 'HTTP_PROXY': 'http://127.0.0.1:9'}
        for k in [k for k in os.environ if k.lower() in ('no_proxy', 'http_proxy', 'https_proxy', 'all_proxy')] + list(want):
            self.addCleanup(os.environ.__setitem__, k, os.environ[k]) if k in os.environ else self.addCleanup(os.environ.pop, k, None)
            os.environ.pop(k, None)
        os.environ.update(want)
        was = urllib.request._opener
        urllib.request._opener = None  # urlopen keeps the first opener it made: make it look at the proxy again
        self.addCleanup(lambda: setattr(urllib.request, '_opener', was))

    def test_the_proxy_is_really_in_the_way(self):
        with self.assertRaises(OSError):
            urllib.request.urlopen(self.url + '/x', timeout=2)

    def test_the_server_reaches_the_hub(self):
        self.assertEqual(nodes.hub_call('POST', '/internal/x', {'a': 1}, timeout=2, hub=self.url)['ok'], True)
        self.assertEqual(features.hub_open_known(self.url + '/internal/open', timeout=2), {('desk', 'abcd1234')})
        self.assertIsNotNone(serverctl._probe(self.url + '/healthz', timeout=2))
        self.assertEqual([p for _, p, _ in Here.seen], ['/internal/x', '/internal/open', '/healthz'])

    def test_the_helper_reaches_the_server(self):
        sb = fresh_sb(Path(tempfile.mkdtemp()))
        self.assertTrue(sb._post_any({'event': 'x'}, 2, [self.url]))
        self.assertEqual(sb.hub('GET', '/api/x', timeout=2, servers=[self.url])['ok'], True)
        self.assertEqual(sb.budget_fetch(2, servers=[self.url]), {'day': 'x'})
        self.assertEqual([p for _, p, _ in Here.seen], ['/api/event', '/api/x', '/api/budget'])


if __name__ == '__main__':
    unittest.main()
