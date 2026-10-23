"""Announcing the server on the local network as _switchboard._tcp, so the iPhone app finds it without an address
typed in. The TXT record carries url=<the address to use>. Best effort, with what the machine has: python-zeroconf
when installed (the server kit carries a copy in server/vendor, used only when the machine has none), else the system's
own tool (dns-sd on a Mac, avahi-publish on Linux), else nothing (logged)."""
import ipaddress, os, shutil, socket, subprocess, sys, threading
from urllib.parse import urlsplit

TYPE = '_switchboard._tcp'


def _lan(origin):
    """True for an address on the local network itself: a private IPv4 that is not Tailscale's 100.64.0.0/10."""
    try:
        ip = ipaddress.ip_address(urlsplit(origin).hostname or '')
    except ValueError:
        return False
    return ip.version == 4 and ip.is_private and ip not in ipaddress.ip_network('100.64.0.0/10')


def _url(port, cfg):
    """The address the phone should use: the first configured origin on the local network (this is only heard
    there), else the first configured origin, else this machine's .local name."""
    origins = [o.rstrip('/') for o in (getattr(cfg, 'origins', None) or [])
               if isinstance(o, str) and o.startswith(('http://', 'https://'))]
    if origins:
        return next((o for o in origins if _lan(o)), origins[0])
    return f'http://{socket.gethostname().split(".")[0].lower()}.local:{port}'


def _name():
    return f'Switchboard on {socket.gethostname().split(".")[0]}'[:63]


VENDOR = os.path.join(os.path.dirname(os.path.abspath(__file__)), 'vendor')


def _zeroconf(vendor=VENDOR):
    """python-zeroconf's ServiceInfo and Zeroconf: the machine's own, else the kit's copy, else (None, None)."""
    for extra in (None, vendor):
        if extra:
            if not os.path.isdir(extra):
                break
            if extra not in sys.path:
                sys.path.append(extra)  # appended, so an installed zeroconf always wins
        try:
            from zeroconf import ServiceInfo, Zeroconf
            return ServiceInfo, Zeroconf
        except ImportError:
            continue
    return None, None


def _lan_ip():
    """This machine's address on the local network (the one its default route uses), or None."""
    s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    try:
        s.connect(('192.0.2.1', 9))  # no packet is sent; it only picks the outgoing interface
        ip = s.getsockname()[0]
        return None if ip.startswith('127.') else ip
    except OSError:
        return None
    finally:
        s.close()


def announce(port, cfg, log, which=shutil.which, popen=subprocess.Popen, zeroconf=_zeroconf):
    """Starts announcing and returns how ('zeroconf', 'dns-sd', 'avahi'), or None when nothing on this machine can."""
    url = _url(port, cfg)
    ServiceInfo, Zeroconf = zeroconf()
    if Zeroconf:
        try:
            zc = Zeroconf()
            ip = _lan_ip()
            info = ServiceInfo(f'{TYPE}.local.', f'{_name()}.{TYPE}.local.', port=port, properties={'url': url},
                               server=f'{socket.gethostname().split(".")[0]}.local.',
                               addresses=[socket.inet_aton(ip)] if ip else None)
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
