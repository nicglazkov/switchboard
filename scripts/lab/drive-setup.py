#!/usr/bin/env python3
"""Answer setup.py's questions the way a person would, in a real terminal (a pty), for install-lab.sh.

usage: drive-setup.py <server-container> <user> <machine-container> <machine-user> <label>

The person: takes the suggested address (the one picked before), adds a machine with a code (the pasted curl line runs on
<machine-container>), says Done, then browses that machine's folders and picks Desktop/Claude/site. Prints the
whole session, and exits 1 if a step never came.
"""
import os, pty, re, select, subprocess, sys, time

server, user, machine, muser, label = sys.argv[1:6]
SETUP = '.local/share/switchboard/app/install/setup.py'
pid, fd = pty.fork()
if pid == 0:
    os.execvp('docker', ['docker', 'exec', '-it', server, 'su', '-', user, '-c', f'python3 {SETUP}'])

buf = ''


def wait_for(pattern, timeout=120):
    """Read until pattern shows up in what was printed since the last match; the match."""
    global buf
    end = time.time() + timeout
    while time.time() < end:
        m = re.search(pattern, buf, re.S)
        if m:
            buf = buf[m.end():]
            return m
        r, _, _ = select.select([fd], [], [], 1)
        if r:
            try:
                chunk = os.read(fd, 4096).decode(errors='replace')
            except OSError:
                break
            sys.stdout.write(chunk)
            sys.stdout.flush()
            buf += chunk
    sys.stdout.write(f'\n[drive-setup: never saw {pattern!r}]\n')
    sys.exit(1)


def send(text):
    time.sleep(0.3)
    os.write(fd, (text + '\r').encode())


def open_folder(name):
    """At the folder picker: open the folder with this name, by its number in the list just printed."""
    m = wait_for(r'(.*?)Enter picks this folder[^\n]*?: ')
    n = re.search(r'(\d+)\. ' + re.escape(name) + r'/?\r?\n', m.group(1))
    if not n:
        sys.stdout.write(f'\n[drive-setup: no folder {name} in the list]\n')
        sys.exit(1)
    send(n.group(1))


wait_for(r'Which one should they use\?.*?: ')
send('')                                        # the suggested address
wait_for(r'Pick one \[\d\]: ')
send('1')                                       # Add a machine with a code
m = wait_for(r'linux:\s+(curl [^\r\n]+)')
line = m.group(1).strip()
wait_for(r'Waiting for it to link')
subprocess.run(['docker', 'exec', machine, 'su', '-', muser, '-c', f'{line} --label {label}'], check=True,
               stdout=subprocess.DEVNULL)
wait_for(r'Linked: ' + re.escape(label))
wait_for(r'Pick one \[3\]: ')
send('3')                                       # Done
while True:  # machines with no folder yet come up in the list's order; the others keep their suggestion
    who = wait_for(r'The Claude folder on (\S+?):').group(1)
    if who == label:
        break
    wait_for(r'Enter picks this folder.*?: ')
    send('')
for name in ('Desktop', 'Claude', 'site'):
    open_folder(name)
wait_for(r'~/Desktop/Claude/site\r?\n.*?Enter picks this folder.*?: ')
send('')                                        # pick it
wait_for(r'new sessions start in ~/Desktop/Claude/site')
wait_for(r'All set')
os.waitpid(pid, 0)
print('\n[drive-setup: done]')
