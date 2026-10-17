"""Announcing the server on the local network as _switchboard._tcp, so the iPhone app finds it without an address
typed in. The TXT record carries url=<the address to use>. Best effort, with what the machine has: python-zeroconf
when installed, else the system's own tool (dns-sd on a Mac, avahi-publish on Linux), else nothing (logged)."""
import shutil, socket, subprocess, threading

TYPE = '_switchboard._tcp'


def _url(port, cfg):
    """The address the phone should use: the first configured origin, else this machine's .local name."""
    for o in (getattr(cfg, 'origins', None) or []):
        if isinstance(o, str) and o.startswith(('http://', 'https://')):
            return o.rstrip('/')
    return f'http://{socket.gethostname().split(".")[0].lower()}.local:{port}'


def _name():
    return f'Switchboard on {socket.gethostname().split(".")[0]}'[:63]


def announce(port, cfg, log, which=shutil.which, popen=subprocess.Popen):
    """Starts announcing and returns how ('zeroconf', 'dns-sd', 'avahi'), or None when nothing on this machine can."""
    url = _url(port, cfg)
    try:
        from zeroconf import ServiceInfo, Zeroconf  # optional: pip install zeroconf
    except ImportError:
        Zeroconf = None
    if Zeroconf:
        try:
            zc = Zeroconf()
            info = ServiceInfo(f'{TYPE}.local.', f'{_name()}.{TYPE}.local.', port=port, properties={'url': url},
                               server=f'{socket.gethostname().split(".")[0]}.local.')
            threading.Thread(target=zc.register_service, args=(info,), daemon=True).start()
            log(f'bonjour: announcing {url} (zeroconf)')
            return 'zeroconf'
        except Exception as e:
            log(f'bonjour: zeroconf failed ({e})')
    for tool, args in (('dns-sd', ['-R', _name(), TYPE, 'local', str(port), f'url={url}']),
                       ('avahi-publish-service', [_name(), TYPE, str(port), f'url={url}'])):
        exe = which(tool)
        if not exe:
            continue
        try:
            # it announces for as long as it runs; it goes when the server does
            popen([exe] + args, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, stdin=subprocess.DEVNULL)
            log(f'bonjour: announcing {url} ({tool})')
            return 'dns-sd' if tool == 'dns-sd' else 'avahi'
        except OSError as e:
            log(f'bonjour: {tool} failed ({e})')
    log('bonjour: not announced (install python3-zeroconf, or avahi-utils on Linux, for the iPhone app to find this server)')
    return None
