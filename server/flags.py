"""Flags for a new session, checked here before they travel to a machine (whose agent checks them again, against
its own `claude --help`). The list is the agent's START_FLAGS; tests keep the two the same."""
import re
import shlex

FLAG_TEXT_RE = r"[A-Za-z0-9 .,:;'?()\[\]/_+=*#@$-]{1,2000}"
FLAG_TOOLS_RE = r"[A-Za-z0-9_*:().,/ @-]{1,400}"
START_FLAGS = {
    '--chrome': None, '--no-chrome': None,
    '--permission-mode': r'acceptEdits|auto|bypassPermissions|manual|dontAsk|plan',
    '--add-dir': 'folder',
    '--agent': r'[A-Za-z0-9][A-Za-z0-9_.-]{0,63}',
    '--worktree': r'[A-Za-z0-9][A-Za-z0-9_.-]{0,63}',
    '--fallback-model': r'(?:opus|sonnet|haiku|fable)(?:,(?:opus|sonnet|haiku|fable)){0,3}',
    '--append-system-prompt': FLAG_TEXT_RE,
    '--allowed-tools': FLAG_TOOLS_RE, '--disallowed-tools': FLAG_TOOLS_RE, '--tools': FLAG_TOOLS_RE,
    '--autocompact': r'auto|[1-9][0-9]{2,3}k|1M',
    '--setting-sources': r'(?:user|project|local)(?:,(?:user|project|local)){0,2}',
    '--brief': None, '--verbose': None, '--safe-mode': None, '--restricted': None, '--disable-slash-commands': None,
    '--strict-mcp-config': None, '--exclude-dynamic-system-prompt-sections': None,
}
FLAG_REPEAT = {'--add-dir': 8}
FLAGS_MAX = 24
# Typed in "Other flags" but shown as chips: they go to the session's own fields.
CHIP_FLAGS = {'--model': r'opus|sonnet|haiku|fable', '--effort': r'low|medium|high|xhigh|max'}
ALIASES = {'-w': '--worktree', '--allowedTools': '--allowed-tools', '--disallowedTools': '--disallowed-tools'}
NAME_PART_RE = re.compile(r'[A-Za-z0-9._ -]{1,64}')
OTHER_MAX = 4000


# How a change reaches a session that is already running. Checked on 2026-10-12 against Claude Code 2.1.296
# (`claude --help` on a Mac) and code.claude.com/docs/en/commands and /docs/en/permission-modes:
# - "live": typed into the session as a slash command (cmd; {v} is the value), applied without a restart;
# - "key": a key in the session (Shift+Tab cycles the permission modes that are in its cycle);
# - anything not listed here only takes effect at launch, so the session is relaunched with it (the chat carries on).
# "unverified" marks what the docs say but nobody has watched happen in a Switchboard terminal yet.
LIVE = {
    '--model': {'how': 'live', 'cmd': '/model {v}', 'note': 'Claude Code also keeps it as your default model.'},
    '--effort': {'how': 'live', 'cmd': '/effort {v}', 'values': {'ultracode': '/effort ultracode on'}},
    '--add-dir': {'how': 'live', 'cmd': '/add-dir {v}', 'remove': 'relaunch'},
    '--permission-mode': {'how': 'key', 'values': {'plan': '/plan'}, 'cycle': ['default', 'acceptEdits', 'plan', 'auto'],
                          'relaunch': ['bypassPermissions', 'dontAsk'],
                          'note': 'Shift+Tab cycles Asks first, Accept edits, Plan and Auto. Bypass and Never asks need a relaunch.'},
}
LIVE_UNVERIFIED = ('--effort ultracode needs Claude Code 2.1.284 or later', '/model may ask to confirm when the cache is warm')
# The names the apps show, the same words as the New sheet.
LABELS = {
    '--chrome': 'Claude in Chrome', '--no-chrome': 'No Chrome', '--permission-mode': 'Permissions', '--add-dir': 'Also open',
    '--agent': 'Agent', '--worktree': 'Worktree', '--fallback-model': 'Fallback model', '--append-system-prompt': 'Extra instructions',
    '--allowed-tools': 'Allowed tools', '--disallowed-tools': 'Blocked tools', '--tools': 'Tools', '--autocompact': 'Compacts at',
    '--setting-sources': 'Settings from', '--brief': 'Brief', '--verbose': 'Verbose', '--safe-mode': 'Safe mode',
    '--restricted': 'Restricted', '--disable-slash-commands': 'No skills', '--strict-mcp-config': 'Only listed MCP',
    '--exclude-dynamic-system-prompt-sections': 'Static prompt', '--model': 'Model', '--effort': 'Effort',
    '--remote-control': 'Remote Control', '--dangerously-skip-permissions': 'Skips permissions',
}


class FlagError(ValueError):
    pass


def check_folder(v):
    """A folder inside the machine's Claude folder, written the way the folder picker writes it."""
    parts = [p for p in v.replace('\\', '/').split('/') if p not in ('', '.')]
    if v.startswith(('/', '~', '\\')) or re.match(r'^[A-Za-z]:', v) or not parts or len(parts) > 12 or \
            not all(NAME_PART_RE.fullmatch(p) and p == p.strip() and p != '..' for p in parts):
        raise FlagError('--add-dir takes a folder inside the machine\'s Claude folder (pick it in the folder browser)')
    return '/'.join(parts)


def tokens(text):
    """'Other flags' as [[name], [name, value]]: shell-style words (no shell ever sees them), --name=value or
    --name value."""
    if not text or not text.strip():
        return []
    if len(text) > OTHER_MAX or '\x00' in text or '\n' in text.strip():
        raise FlagError('Other flags is one line, up to 4000 characters')
    try:
        words = shlex.split(text)
    except ValueError as e:
        raise FlagError(f'Other flags: {e}')
    out, i = [], 0
    while i < len(words):
        w = words[i]
        name, eq, val = w.partition('=')
        name = ALIASES.get(name, name)
        if not name.startswith('--'):
            raise FlagError(f'Other flags: {w[:40]!r} is not a flag (flags start with --)')
        spec = START_FLAGS.get(name, CHIP_FLAGS.get(name, '?'))
        if spec == '?':
            raise FlagError(f'{name[:40]} is not a flag Switchboard passes to Claude Code')
        if spec is None:
            if eq:
                raise FlagError(f'{name} takes no value')
            out.append([name])
        elif eq:
            out.append([name, val])
        else:
            if i + 1 >= len(words) or words[i + 1].startswith('--'):
                raise FlagError(f'{name} needs a value')
            out.append([name, words[i + 1]])
            i += 1
        i += 1
    return out


def parse(flags, other=None):
    """The New sheet's chips (flags: [[name], [name, value]]) and its Other flags text, checked. Returns
    {'flags': [...], 'chrome': bool, 'model': str|None, 'effort': str|None}; FlagError says what is wrong."""
    if flags is None:
        flags = []
    if not isinstance(flags, list) or (other is not None and not isinstance(other, str)):
        raise FlagError('flags must be a list')
    items = list(flags) + tokens(other or '')
    if len(items) > FLAGS_MAX:
        raise FlagError(f'At most {FLAGS_MAX} flags')
    out, seen, picked = [], {}, {'model': None, 'effort': None}
    for f in items:
        if not isinstance(f, list) or not 1 <= len(f) <= 2 or not all(isinstance(x, str) for x in f):
            raise FlagError('each flag is [name] or [name, value]')
        name = ALIASES.get(f[0], f[0])
        if name in CHIP_FLAGS:
            if len(f) != 2 or not re.fullmatch(CHIP_FLAGS[name], f[1]):
                raise FlagError(f'{name}: that value is not allowed')
            picked[name[2:]] = f[1]
            continue
        if name not in START_FLAGS:
            raise FlagError(f'{name[:40]!r} is not a flag Switchboard passes to Claude Code')
        seen[name] = seen.get(name, 0) + 1
        if seen[name] > FLAG_REPEAT.get(name, 1):
            raise FlagError(f'{name} is given too many times')
        spec = START_FLAGS[name]
        if spec is None:
            if len(f) != 1:
                raise FlagError(f'{name} takes no value')
            out.append([name])
            continue
        if len(f) != 2 or not f[1]:
            raise FlagError(f'{name} needs a value')
        v = check_folder(f[1]) if spec == 'folder' else f[1]
        if spec != 'folder' and not re.fullmatch(spec, v):
            raise FlagError(f'{name}: that value is not allowed')
        out.append([name, v])
    if '--chrome' in seen and '--no-chrome' in seen:
        raise FlagError('--chrome and --no-chrome cancel out')
    chrome = '--chrome' in seen
    return {'flags': [f for f in out if f[0] != '--chrome'], 'chrome': chrome, **picked}


def command_line(flags):
    """What the flags add to the command line, for showing."""
    return ' '.join(f[0] if len(f) == 1 else f'{f[0]} {shlex.quote(f[1])}' for f in flags)


# ---- a running session's flags: what it has now, and what a change needs (the flags drawer in every client)
MODEL_WORD = {'opus': 'Opus', 'sonnet': 'Sonnet', 'haiku': 'Haiku', 'fable': 'Fable'}
EFFORT_WORD = {'low': 'Low', 'medium': 'Medium', 'high': 'High', 'xhigh': 'Extra high', 'max': 'Max', 'ultracode': 'UltraCode'}
PERM_WORD = {'default': 'Asks first', 'manual': 'Asks first', 'acceptEdits': 'Accept edits', 'plan': 'Plan', 'auto': 'Auto',
             'bypassPermissions': 'Bypass', 'dontAsk': 'Never asks'}
CHIPS = ('--model', '--effort', '--permission-mode', '--chrome', '--no-chrome')
HEAD = ('--model', '--effort', '--permission-mode', '--chrome')
SWITCHBOARD_OWN = ('--remote-control',)  # every background chat Switchboard starts has it; not the user's to drop


def _item(name, value, text, how, **kw):
    return dict({'name': name, 'value': value, 'label': LABELS.get(name, name), 'text': text, 'how': how}, **kw)


def view(s):
    """What one session runs with, for the drawer's "Active now" and the header's one-line summary."""
    now = s.get('now') or {}
    launch = s.get('launch') or {}
    out = []
    if now.get('model'):
        out.append(_item('--model', now['model'], MODEL_WORD.get(now['model'], now['model']), 'live'))
    if now.get('effort'):
        out.append(_item('--effort', now['effort'], EFFORT_WORD.get(now['effort'], now['effort']), 'live'))
    perm = now.get('perm') or 'default'
    out.append(_item('--permission-mode', perm, PERM_WORD.get(perm, perm), 'live' if perm == 'plan' else 'relaunch'))
    if now.get('chrome'):
        out.append(_item('--chrome', None, 'On', 'relaunch', brief='Chrome'))
    own = False
    for f in launch.get('flags') or []:
        name = f[0]
        own = own or name in SWITCHBOARD_OWN
        if name in CHIPS or name in SWITCHBOARD_OWN:
            continue
        v = f[1] if len(f) > 1 else None
        out.append(_item(name, v, v if v is not None else 'On', 'live' if name == '--add-dir' else 'relaunch'))
    if own:
        out.append(_item('--remote-control', None, 'On', 'locked', note='Switchboard keeps it on, so the chat stays reachable.'))
    summary = ' · '.join(x.get('brief') or x['text'] for x in out if x['name'] in HEAD)
    extra = sum(1 for x in out if x['name'] not in HEAD and x['name'] not in SWITCHBOARD_OWN)
    if extra:
        summary += f' · {extra} more'
    short = ' \u00b7 '.join(x['text'] for x in out if x['name'] in ('--model', '--effort'))
    return {'active': out, 'other': list(launch.get('other') or []), 'summary': summary, 'short': short or summary,
            'source': launch.get('from'), 'complete': bool(launch)}


def plan(s, want, root=None):
    """A wanted set of flags against what the session has now: each change and how it gets there.
    want: {model, effort, perm, chrome, flags: [[name], [name, value]], other: "--more flags"} (flags and other:
    everything but the chips; the permissions and Chrome go in perm and chrome).
    Returns {changes: [{name, label, from, to, how, cmd?}], cmds: [slash commands to type, in order],
    relaunch: bool, body: {model, effort, flags} for POST /api/relaunch}. A running chat takes live changes
    typed into it; a stopped one gets everything at relaunch. FlagError says what is wrong with `want`."""
    cur = view(s)
    have = {x['name']: x for x in cur['active']}
    hv = lambda n: (have.get(n) or {}).get('value')
    running = s.get('kind') == 'background' and s.get('status') in ('idle', 'busy', 'waiting')
    want = want if isinstance(want, dict) else {}
    other = want.get('other') if isinstance(want.get('other'), str) else ''
    checked = parse([f for f in want.get('flags') or [] if isinstance(f, list) and f and f[0] not in CHIPS], other)
    if checked.get('model') or checked.get('effort') or checked.get('chrome') or any(f[0] in CHIPS for f in checked['flags']):
        raise FlagError('Pick the model, effort and Chrome above instead.')
    model = want.get('model') if want.get('model') in MODEL_WORD else hv('--model')
    effort = want.get('effort') if want.get('effort') in EFFORT_WORD else hv('--effort')
    perm = want.get('perm') if want.get('perm') in PERM_WORD else hv('--permission-mode') or 'default'
    chrome = bool(want.get('chrome')) if 'chrome' in want else '--chrome' in have
    changes, cmds = [], []

    def change(name, frm, to, how, cmd=None):
        c = {'name': name, 'label': LABELS.get(name, name), 'from': frm, 'to': to, 'how': how if running else 'relaunch'}
        if cmd and running and how == 'live':
            c['cmd'] = cmd
            cmds.append(cmd)
        changes.append(c)

    if model != hv('--model'):
        change('--model', MODEL_WORD.get(hv('--model')), MODEL_WORD.get(model), 'live', f'/model {model}')
    if effort != hv('--effort'):
        change('--effort', EFFORT_WORD.get(hv('--effort')), EFFORT_WORD.get(effort), 'live',
               '/effort ultracode on' if effort == 'ultracode' else f'/effort {effort}')
    old_perm = hv('--permission-mode') or 'default'
    if perm != old_perm and not (perm in ('default', 'manual') and old_perm in ('default', 'manual')):
        change('--permission-mode', PERM_WORD.get(old_perm), PERM_WORD.get(perm), 'live' if perm == 'plan' else 'relaunch',
               '/plan' if perm == 'plan' else None)
    if chrome != ('--chrome' in have):
        change('--chrome', 'Off' if chrome else 'On', 'On' if chrome else 'Off', 'relaunch')
    old = [[x['name']] + ([x['value']] if x['value'] is not None else []) for x in cur['active']
           if x['name'] not in CHIPS and x['name'] not in SWITCHBOARD_OWN]
    new = checked['flags']
    for f in new:
        if f not in old:
            if f[0] == '--add-dir':
                sep = '\\' if root and '\\' in root and '/' not in root else '/'
                path = f'{root.rstrip(sep)}{sep}{f[1]}' if root else f[1]
                change(f[0], None, f[1], 'live', f'/add-dir {shlex.quote(path) if any(c.isspace() for c in path) else path}')
            else:
                change(f[0], None, f[1] if len(f) > 1 else 'On', 'relaunch')
    for f in old:
        if f not in new:
            change(f[0], f[1] if len(f) > 1 else 'On', None, 'relaunch')
    relaunch = any(c['how'] == 'relaunch' for c in changes)
    flags = list(new) + ([['--permission-mode', perm]] if perm not in ('default', 'manual') else []) + ([['--chrome']] if chrome else [])
    return {'changes': changes, 'cmds': [] if relaunch else cmds, 'relaunch': relaunch, 'running': running,
            'busy': s.get('status') == 'busy', 'dropped': cur['other'],
            'body': {'model': model, 'effort': effort, 'flags': flags}}
