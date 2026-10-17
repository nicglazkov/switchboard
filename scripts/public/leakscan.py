#!/usr/bin/env python3
"""Look for things that must not be published: personal names, home paths, private addresses,
secrets, em dashes, and stock filler phrases in prose.

    leakscan.py [--deny FILE] [--history] PATH...

PATH is a folder or a file. With --history, PATH must be a git checkout and every commit
message, author, committer and blob in its history is checked too.

--deny FILE adds your own patterns, one regular expression per line (case-insensitive,
# starts a comment). Keep that file outside the repository: it is the list of what is private
to you. SWITCHBOARD_DENYLIST names a default one.

A line that contains "leakscan: allow" is skipped. Exit status is 1 when anything is found.
"""

import argparse
import os
import re
import subprocess
import sys

TEXT_LIMIT = 20 * 1024 * 1024
SKIP_DIRS = {'.git', 'target', 'node_modules', '__pycache__', '.pytest_cache', 'dist', 'build', '.gradle'}

# Addresses and names that are fine to publish.
DOC_IPS = re.compile(r'^(192\.0\.2|198\.51\.100|203\.0\.113)\.')
PLACEHOLDER_USERS = {'you', 'yourname', 'shared', 'public', 'runner', 'example'}
OK_EMAIL_DOMAINS = ('example.com', 'example.org', 'example.net', 'users.noreply.github.com',
                    'anthropic.com', 'example', '.local', '.lan', '.internal', '.ts.net')
OK_TAILNETS = {'example', 'tail0000', 'tail1234', 'your-tailnet'}
FILE_SUFFIXES = ('.png', '.jpg', '.jpeg', '.svg', '.gif', '.webp', '.ico', '.icns', '.pdf')

RULES = [
    ('em dash', re.compile('\\u2014')),
    ('private address', re.compile(
        r'(?<![\d.])(10\.\d{1,3}\.\d{1,3}\.\d{1,3}|172\.(1[6-9]|2\d|3[01])\.\d{1,3}\.\d{1,3}'
        r'|192\.168\.\d{1,3}\.\d{1,3}|100\.(6[4-9]|[7-9]\d|1[01]\d|12[0-7])\.\d{1,3}\.\d{1,3})(?![\d])')),
    ('home folder', re.compile(r'(/Users/|/home/|[A-Za-z]:\\\\?Users\\\\?)([A-Za-z0-9._-]+)')),
    ('email address', re.compile(r'\b[A-Za-z0-9._%+-]+@([A-Za-z0-9-]+\.)+[A-Za-z]{2,}\b')),
    ('tailnet name', re.compile(r'\b([a-z0-9-]+)\.ts\.net\b')),
    ('private key', re.compile(r'-----BEGIN [A-Z ]*PRIVATE KEY-----')),
    ('api key', re.compile(r'\b(sk-ant-[A-Za-z0-9_-]{16,}|gh[pousr]_[A-Za-z0-9]{30,}|github_pat_[A-Za-z0-9_]{20,}'
                           r'|AKIA[0-9A-Z]{16}|xox[abpr]-[A-Za-z0-9-]{10,}|AIza[0-9A-Za-z_-]{30,})')),
    ('signing identity', re.compile(r'(Developer ID (Application|Installer): [^"\n]*\([A-Z0-9]{10}\)'
                                    r'|DEVELOPMENT_TEAM\s*[=:]\s*"?[A-Z0-9]{10}\b|AuthKey_[A-Z0-9]{10}\.p8)')),
]

# Filler that reads as machine-written. Checked in prose only: Markdown, templates, commit messages.
PROSE = re.compile(r'\b(seamless(ly)?|robust|leverag(e|es|ing)|empower(s|ing)?|delv(e|es|ing)|unlock(s|ing)?'
                   r'|effortless(ly)?|cutting[- ]edge|streamlin(e|es|ed|ing)|elevat(e|es|ing)|supercharg(e|ed|es)'
                   r'|game[- ]chang(er|ing)|next[- ]level|world[- ]class|revolutioni[sz](e|es|ing)|tapestry'
                   r'|a testament to|in today\'s|whether you\'re|it\'s worth noting|dive (in|into)|embark'
                   r'|holistic|synerg(y|ies)|unleash(es|ing)?|boasts?|nestled|bustling|meticulous(ly)?)\b', re.I)
PROSE_SUFFIXES = ('.md', '.yml', '.yaml', '.txt')

# Other projects' license texts, copied word for word: their authors' names, emails and punctuation are theirs to keep.
# Addresses, home folders and keys are still checked in them.
THIRD_PARTY = {'THIRD-PARTY-NOTICES.txt'}
THIRD_PARTY_SKIP = {'email address', 'em dash'}


def allowed(rule, m):
    s = m.group(0)
    if rule == 'private address':
        # Stand-ins that tests use for "some LAN", "some tailnet address" and the Android emulator's host.
        return s.startswith(('10.0.0.', '100.64.0.')) or s == '10.0.2.2'
    if rule == 'home folder':
        return m.group(2).lower() in PLACEHOLDER_USERS or m.group(2).startswith(('$', '%', '<'))
    if rule == 'email address':
        d = s.split('@', 1)[1].lower()
        return d.endswith(OK_EMAIL_DOMAINS) or d.endswith(FILE_SUFFIXES) or s.lower().startswith('git@')
    if rule == 'tailnet name':
        return m.group(1) in OK_TAILNETS
    return False


def load_deny(path):
    out = []
    if not path:
        return out
    with open(os.path.expanduser(path), encoding='utf-8') as f:
        for line in f:
            line = line.strip()
            if line and not line.startswith('#'):
                out.append(re.compile(line, re.I))
    return out


def check_text(where, text, deny, prose, found, binary=False, third_party=False):
    if third_party:
        deny, prose = [], False
    for n, line in enumerate(text.splitlines(), 1):
        if 'leakscan: allow' in line:
            continue
        for rule, rx in RULES:
            if (binary or third_party) and rule in THIRD_PARTY_SKIP:
                continue
            for m in rx.finditer(line):
                if not allowed(rule, m):
                    found.append(f'{where}:{n}: {rule}: {m.group(0)!r}')
        for rx in deny:
            for m in rx.finditer(line):
                found.append(f'{where}:{n}: private term: {m.group(0)!r}')
        if prose:
            for m in PROSE.finditer(line):
                found.append(f'{where}:{n}: filler word: {m.group(0)!r}')


def decode(data):
    if b'\0' in data[:8192]:
        # A binary file: check the printable runs in it, so a path baked into a build is still caught.
        return '\n'.join(s.decode('ascii') for s in re.findall(rb'[\x20-\x7e]{6,}', data)), False
    return data.decode('utf-8', 'replace'), True


def scan_file(path, shown, deny, found):
    if os.path.getsize(path) > TEXT_LIMIT:
        return
    with open(path, 'rb') as f:
        text, is_text = decode(f.read())
    check_text(shown, text, deny, is_text and shown.endswith(PROSE_SUFFIXES), found, not is_text,
               os.path.basename(shown) in THIRD_PARTY)


def scan_tree(root, deny, found):
    if os.path.isfile(root):
        scan_file(root, root, deny, found)
        return
    for d, dirs, files in os.walk(root):
        dirs[:] = sorted(x for x in dirs if x not in SKIP_DIRS)
        for name in sorted(files):
            p = os.path.join(d, name)
            if os.path.islink(p):
                continue
            scan_file(p, os.path.relpath(p, root), deny, found)


def git(root, *args):
    return subprocess.run(['git', '-C', root, *args], check=True, capture_output=True).stdout


def scan_history(root, deny, found):
    log = git(root, 'log', '--all', '--format=%H%x00%an <%ae>%x00%cn <%ce>%x00%B%x01').decode('utf-8', 'replace')
    for entry in log.split('\x01'):
        parts = entry.strip('\n').split('\x00')
        if len(parts) < 4:
            continue
        sha, author, committer, body = parts
        check_text(f'commit {sha[:10]} author', author + '\n' + committer, deny, False, found)
        check_text(f'commit {sha[:10]} message', body, deny, True, found)
    blobs = git(root, 'rev-list', '--all', '--objects').decode().split('\n')
    seen = set()
    for line in blobs:
        if ' ' not in line:
            continue
        sha, path = line.split(' ', 1)
        if sha in seen:
            continue
        seen.add(sha)
        kind = git(root, 'cat-file', '-t', sha).strip()
        if kind != b'blob':
            continue
        data = git(root, 'cat-file', 'blob', sha)
        if len(data) > TEXT_LIMIT:
            continue
        text, is_text = decode(data)
        check_text(f'history {sha[:10]} {path}', text, deny, is_text and path.endswith(PROSE_SUFFIXES), found,
                   not is_text, os.path.basename(path) in THIRD_PARTY)


def main():
    ap = argparse.ArgumentParser(description=__doc__.split('\n')[0])
    ap.add_argument('paths', nargs='+')
    ap.add_argument('--deny', default=os.environ.get('SWITCHBOARD_DENYLIST'))
    ap.add_argument('--history', action='store_true')
    a = ap.parse_args()
    deny = load_deny(a.deny)
    found = []
    for p in a.paths:
        scan_tree(p, deny, found)
        if a.history:
            scan_history(p, deny, found)
    for line in found:
        print(line)
    if found:
        print(f'leakscan: {len(found)} problem(s)', file=sys.stderr)
        return 1
    print('leakscan: clean', file=sys.stderr)
    return 0


if __name__ == '__main__':
    sys.exit(main())
