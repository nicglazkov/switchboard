"""The downloads on port 8131: the files Add a machine hands to a new computer, served over plain HTTP.

    python downloads.py [--directory DIR] [--port 8131] [--config DIR]

In place of `python -m http.server 8131 --directory DIR`, which the installed services still run today (pointing
them here is the installers' step). It serves the same folder the same way, with one difference: when server.json
says "keys": "only" (keys.py), nothing listens. A new computer then pairs with a code or a link and gets what it needs
through that connection, and a port open to the network would be the one thing left that answers by address.

In "only" the program stays up and idle instead of ending, so a service manager that restarts whatever stops does not
start it again and again. The mode is read when it starts: after a change to server.json, restart it with the
server. Standard library only.
"""
import argparse
import functools
import sys
import threading
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import keys as keys_mod  # noqa: E402

PORT = 8131


def default_config():
    return Path.home() / '.config' / 'switchboard'


def default_directory():
    return Path.home() / '.local' / 'share' / 'switchboard' / 'downloads'


def serves(cfg_dir):
    """Whether the downloads are served at all: not with keys only, and not when server.json cannot be read."""
    return keys_mod.read_mode(Path(cfg_dir) / 'server.json') != keys_mod.ONLY


def make_server(directory, port=PORT, host=''):
    handler = functools.partial(SimpleHTTPRequestHandler, directory=str(directory))
    return ThreadingHTTPServer((host, port), handler)


def main(argv=None, wait=None):
    ap = argparse.ArgumentParser(description='Serves the Switchboard downloads, unless keys are set to only.')
    ap.add_argument('--directory', default=str(default_directory()))
    ap.add_argument('--port', type=int, default=PORT)
    ap.add_argument('--config', default=str(default_config()))
    a = ap.parse_args(argv)
    if not serves(a.config):
        print(f'downloads: not served on port {a.port}, because this home computer answers itself only (keys only)',
              flush=True)
        (wait or threading.Event().wait)()
        return 0
    httpd = make_server(a.directory, a.port)
    print(f'downloads: {a.directory} on port {a.port}', flush=True)
    try:
        httpd.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        httpd.server_close()
    return 0


if __name__ == '__main__':
    sys.exit(main())
