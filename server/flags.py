"""Flags for a new session, checked here before they travel to a machine (whose agent checks them again, against
its own `claude --help`). The list is the agent's START_FLAGS; tests keep the two the same."""
import re
import shlex

FLAG_TEXT_RE = r"[A-Za-z0-9 .,:;'?()\[\]/_+=*#@$-]{1,2000}"
FLAG_TOOLS_RE = r"[A-Za-z0-9_*:().,/ @-]{1,400}"
FLAG_MODEL_RE = r'(?:fable|opus|sonnet|haiku)(?:\[1m\])?|claude-[a-z0-9]+(?:-[a-z0-9]+){1,6}(?:\[1m\])?'  # an alias or a full id
START_FLAGS = {
    '--chrome': None, '--no-chrome': None,
    # manual and default both mean "asks every time": Claude Code lists one or the other, by version (ask_mode)
    '--permission-mode': r'acceptEdits|auto|bypassPermissions|manual|default|dontAsk|plan',
    '--add-dir': 'folder',
    '--agent': r'[A-Za-z0-9][A-Za-z0-9_.-]{0,63}',
    '--worktree': r'[A-Za-z0-9][A-Za-z0-9_.-]{0,63}',
    '--fallback-model': rf'(?:{FLAG_MODEL_RE})(?:,(?:{FLAG_MODEL_RE})){{0,3}}',
    '--append-system-prompt': FLAG_TEXT_RE,
    '--allowed-tools': FLAG_TOOLS_RE, '--disallowed-tools': FLAG_TOOLS_RE, '--tools': FLAG_TOOLS_RE,
    '--autocompact': r'auto|[1-9][0-9]{2}k|1000k|1M',  # Claude Code takes 100k to 1M
    '--setting-sources': r'(?:user|project|local)(?:,(?:user|project|local)){0,2}',
    '--brief': None, '--verbose': None, '--safe-mode': None, '--restricted': None, '--disable-slash-commands': None,
    '--strict-mcp-config': None, '--exclude-dynamic-system-prompt-sections': None,
}
FLAG_REPEAT = {'--add-dir': 8}
FLAG_OPTIONAL = ('--worktree',)  # the value may be left out: Claude Code then names the worktree itself
FLAGS_MAX = 24
ASK_MODES = ('manual', 'default')
# What a Switchboard from before 0.8.20 takes for a flag whose values have grown since (its own check refuses the
# rest and fails the launch). Such a machine is told apart by not saying its permission modes (ask_mode is None).
OLD_VALUES = {'--fallback-model': r'(?:opus|sonnet|haiku|fable)(?:,(?:opus|sonnet|haiku|fable)){0,3}'}
# Typed in "Other flags" but shown as chips: they go to the session's own fields.
MODEL_ALIASES = ('fable', 'opus', 'sonnet', 'haiku')  # the aliases every Claude Code this app knows takes
MODEL_PATTERN = FLAG_MODEL_RE
CHIP_FLAGS = {'--model': MODEL_PATTERN, '--effort': r'low|medium|high|xhigh|max'}
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
    '--computer-use': 'Computer use',
}
# Not a claude flag: the name the drawer shows computer use under (Claude Code turns it on per folder, not by a flag)
COMPUTER = '--computer-use'


class FlagError(ValueError):
    pass


def check_folder(v):
    """A folder written the way the folder picker writes it: inside the machine's Claude folder ("notes/a"), in its
    home ("~/Documents/a") or in one of its places ("@Drive/a"). The machine checks that it is there and may be
    opened; this only keeps out anything that is not such a name."""
    parts = [p for p in v.replace('\\', '/').split('/') if p not in ('', '.')]
    head = []
    if parts and (parts[0] == '~' or (parts[0].startswith('@') and NAME_PART_RE.fullmatch(parts[0][1:])
                                      and parts[0] == parts[0].strip())):
        head, parts = parts[:1], parts[1:]
    if v.startswith(('/', '\\')) or re.match(r'^[A-Za-z]:', v) or not (head or parts) or len(parts) > 12 or \
            not all(NAME_PART_RE.fullmatch(p) and p == p.strip() and p != '..' for p in parts) or \
            (head and any(p.startswith('.') for p in parts)):  # outside the Claude folder, dot folders are private
        raise FlagError('--add-dir takes a folder on that machine (pick it in the folder browser)')
    return '/'.join(head + parts)


def tokens(text):
    """'Other flags' as [[name], [name, value]]: shell-style words (no shell ever sees them), --name=value or
    --name value."""
    if not text or not text.strip():
        return []
    if len(text) > OTHER_MAX or '\x00' in text or '\n' in text.strip():
        raise FlagError('Other flags has to be one line, up to 4000 characters')
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
            raise FlagError(f'Other flags: {w[:40]!r} isn\'t a flag (flags start with --)')
        spec = START_FLAGS.get(name, CHIP_FLAGS.get(name, '?'))
        if spec == '?':
            raise FlagError(f'{name[:40]} isn\'t a flag Switchboard passes to Claude Code')
        if spec is None:
            if eq:
                raise FlagError(f'{name} doesn\'t take a value')
            out.append([name])
        elif eq:
            out.append([name, val])
        elif name in FLAG_OPTIONAL and (i + 1 >= len(words) or words[i + 1].startswith('-')):
            out.append([name])
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
                raise FlagError(f'{name}: that value isn\'t allowed')
            picked[name[2:]] = f[1]
            continue
        if name not in START_FLAGS:
            raise FlagError(f'{name[:40]!r} isn\'t a flag Switchboard passes to Claude Code')
        seen[name] = seen.get(name, 0) + 1
        if seen[name] > FLAG_REPEAT.get(name, 1):
            raise FlagError(f'{name} is in there too many times')
        spec = START_FLAGS[name]
        if spec is None:
            if len(f) != 1:
                raise FlagError(f'{name} doesn\'t take a value')
            out.append([name])
            continue
        if len(f) == 1 and name in FLAG_OPTIONAL:
            out.append([name])
            continue
        if len(f) != 2 or not f[1]:
            raise FlagError(f'{name} needs a value')
        v = check_folder(f[1]) if spec == 'folder' else f[1]
        if spec != 'folder' and not re.fullmatch(spec, v):
            raise FlagError(f'{name}: that value isn\'t allowed')
        out.append([name, v])
    if '--chrome' in seen and '--no-chrome' in seen:
        raise FlagError('--chrome and --no-chrome cancel out')
    chrome = '--chrome' in seen
    return {'flags': [f for f in out if f[0] != '--chrome'], 'chrome': chrome, **picked}


def full_path(root, rel):
    """A folder the picker names as a full path on its machine, when the server can tell: one under the Claude
    folder. "~/..." is left to the shell; "@Place/..." only the machine can resolve."""
    if not root or rel.startswith(('~', '@')):
        return rel
    sep = '\\' if '\\' in root and '/' not in root else '/'
    return f'{root.rstrip(sep)}{sep}{rel.replace("/", sep)}'


def quote(v, win=False):
    """One word of a command the user pastes into a terminal: POSIX quoting, or double quotes on Windows, where
    cmd.exe and PowerShell both read them (no value the flags allow holds a double quote)."""
    if not win:
        return shlex.quote(v)
    return v if re.fullmatch(r'[A-Za-z0-9_.,:/\\=@+~-]+', v) else '"' + v.replace('"', '') + '"'


def command_line(flags, root=None, win=False):
    """What the flags add to a command the user runs in the chat's own folder. An extra folder goes in as its full
    path: it is written relative to the machine's Claude folder (root), not to where the chat runs."""
    def word(f):
        if f[0] != '--add-dir':
            return quote(f[1], win)
        if f[1].startswith('~/') and not win:  # the shell fills in home only for a ~ outside the quotes
            return '~/' + quote(f[1][2:])
        return quote(full_path(root, f[1]), win)
    return ' '.join(f[0] if len(f) == 1 else f'{f[0]} {word(f)}' for f in flags)


def ask_mode(perms):
    """What a machine's Claude Code calls the mode that asks every time: "manual" today, "default" before that.
    None when its agent does not say (an older Switchboard there): the flag is then left out, as it always was,
    and the chat starts in that machine's own default mode."""
    perms = perms if isinstance(perms, (list, tuple)) else ()
    return next((m for m in ASK_MODES if m in perms), None)


def old_takes(f):
    """Would a Switchboard from before 0.8.20 pass this checked flag on (OLD_VALUES)?"""
    return f[0] not in OLD_VALUES or (len(f) > 1 and re.fullmatch(OLD_VALUES[f[0]], f[1]) is not None)


def asks_dropped(flags, ask):
    """True when fit_perm leaves out an "Asks first" somebody picked, because the machine cannot be told by name."""
    return not ask and any(f[0] == '--permission-mode' and f[1:] and f[1] in ASK_MODES for f in flags)


def fit_perm(flags, ask):
    """Checked flags with "asks every time" written the way the machine's Claude Code takes it (ask_mode), or left
    out when the machine cannot be told."""
    out = []
    for f in flags:
        if f[0] == '--permission-mode' and f[1] in ASK_MODES:
            if ask:
                out.append([f[0], ask])
            continue
        out.append(f)
    return out


# ---- a running session's flags: what it has now, and what a change needs (the flags drawer in every client)
MODEL_WORD = {'opus': 'Opus', 'sonnet': 'Sonnet', 'haiku': 'Haiku', 'fable': 'Fable'}
MODEL_ID_RE = re.compile(r'claude-([a-z]+)-(\d+)(?:-(\d{1,2}))?(?:-\d{8})?(?:-[a-z]+)?(\[1m\])?')


def model_ok(m, more=()):
    """An alias (opus, fable), a full model id (claude-opus-5-5), or one of `more`: aliases a machine's own Claude Code
    names in its help (a newer family), for --model."""
    return isinstance(m, str) and (re.fullmatch(MODEL_PATTERN, m) is not None or m in more)


def model_word(m):
    """How a model is shown: "Opus" for an alias, "Opus 5.5" for claude-opus-5-5, None for nothing."""
    if not isinstance(m, str) or not m:
        return None
    if m.endswith('[1m]') and m[:-4] in MODEL_WORD:  # "opus[1m]": the alias with the 1M window
        return MODEL_WORD[m[:-4]]
    if m in MODEL_WORD:
        return MODEL_WORD[m]
    x = MODEL_ID_RE.fullmatch(m)
    if x:
        return f'{x.group(1).capitalize()} {x.group(2)}' + (f'.{x.group(3)}' if x.group(3) else '')
    return m.capitalize() if re.fullmatch(r'[a-z][a-z0-9]{1,23}', m) else m
EFFORT_WORD = {'low': 'Low', 'medium': 'Medium', 'high': 'High', 'xhigh': 'Extra high', 'max': 'Max', 'ultracode': 'UltraCode'}
PERM_WORD = {'default': 'Asks first', 'manual': 'Asks first', 'acceptEdits': 'Accept edits', 'plan': 'Plan', 'auto': 'Auto',
             'bypassPermissions': 'Bypass', 'dontAsk': 'Never asks'}
CHIPS = ('--model', '--effort', '--permission-mode', '--chrome', '--computer-use')  # each has its own control in the drawer
HEAD = ('--model', '--effort', '--permission-mode', '--chrome', '--computer-use')
SWITCHBOARD_OWN = ('--remote-control',)  # every background chat Switchboard starts has it; not the user's to drop
# Read when the chat is first started and never sent again: a relaunch runs in the worktree the chat already has, and
# passing the flag again there would ask Claude Code for a worktree inside the worktree.
START_ONLY = {'--worktree': 'Set when the chat starts. A relaunch stays in the same worktree.'}
# What the agent reads off a terminal chat's command line that no relaunch can pass. Shown by name only.
VIEW_ONLY = ('--dangerously-skip-permissions', '--allow-dangerously-skip-permissions', '--ide')


def _item(name, value, text, how, **kw):
    return dict({'name': name, 'value': value, 'label': LABELS.get(name, name), 'text': text, 'how': how}, **kw)


def _family(m):
    return next((a for a in MODEL_ALIASES if a in m), None) if isinstance(m, str) else None


def launch_model(s):
    """The model a relaunch asks for: what the chat was launched with (a pinned version such as claude-opus-4-8, or
    opus[1m]) while it still runs on that family, else the family it runs on now."""
    now = (s.get('now') or {}).get('model')
    for f in reversed((s.get('launch') or {}).get('flags') or []):
        if isinstance(f, list) and len(f) == 2 and f[0] == '--model' and model_ok(f[1]) and _family(f[1]) == now:
            return f[1]
    return now


def view(s):
    """What one session runs with, for the drawer's "Active now" and the header's one-line summary."""
    now = s.get('now') or {}
    launch = s.get('launch') or {}
    out, other = [], []
    model = launch_model(s)
    if model:
        out.append(_item('--model', model, (model_word(model) or model) + (' (1M)' if model.endswith('[1m]') else ''), 'live'))
    if now.get('effort'):
        out.append(_item('--effort', now['effort'], EFFORT_WORD.get(now['effort'], now['effort']), 'live'))
    perm = now.get('perm') or 'default'
    out.append(_item('--permission-mode', perm, PERM_WORD.get(perm, perm), 'live' if perm == 'plan' else 'relaunch'))
    if now.get('chrome'):
        out.append(_item('--chrome', None, 'On', 'relaunch', brief='Chrome'))
    if now.get('computer'):  # Switchboard's, not a claude flag: computer use is on for the chat's folder
        out.append(_item(COMPUTER, None, 'On', 'relaunch', brief='Computer use'))
    own = False
    for f in launch.get('flags') or []:
        if not isinstance(f, list) or not f or not all(isinstance(x, str) for x in f):
            continue
        name = ALIASES.get(f[0], f[0])
        own = own or name in SWITCHBOARD_OWN
        if name in CHIPS or name in SWITCHBOARD_OWN:
            continue
        if name not in START_FLAGS:
            # on its command line, but not a flag a relaunch can pass (--ide): named, never sent back. Skipping
            # permissions already shows as Bypass above, and a relaunch keeps that.
            if name not in other and not (name == '--dangerously-skip-permissions' and perm == 'bypassPermissions'):
                other.append(name)
            continue
        v = f[1] if len(f) > 1 else None
        if name in START_ONLY:
            out.append(_item(name, v, v if v is not None else 'On', 'locked', note=START_ONLY[name]))
            continue
        out.append(_item(name, v, v if v is not None else 'On', 'live' if name == '--add-dir' else 'relaunch'))
    if own:
        out.append(_item('--remote-control', None, 'On', 'locked', note='Switchboard keeps it on, so the chat stays reachable.'))
    summary = ' · '.join(x.get('brief') or x['text'] for x in out if x['name'] in HEAD)
    extra = sum(1 for x in out if x['name'] not in HEAD and x['name'] not in SWITCHBOARD_OWN)
    if extra:
        summary += f' · {extra} more'
    short = ' · '.join(x['text'] for x in out if x['name'] in ('--model', '--effort'))
    return {'active': out, 'other': other + [x for x in launch.get('other') or [] if isinstance(x, str) and x not in other],
            'summary': summary, 'short': short or summary, 'source': launch.get('from'), 'complete': bool(launch)}


def plan(s, want, root=None, ask=None):
    """A wanted set of flags against what the session has now: each change and how it gets there.
    want: {model, effort, perm, chrome, flags: [[name], [name, value]], other: "--more flags"} (flags and other:
    everything but the chips; the permissions and Chrome go in perm and chrome).
    Returns {changes: [{name, label, from, to, how, cmd?}], cmds: [slash commands to type, in order],
    relaunch: bool, body: {model, effort, flags} for POST /api/relaunch}. A running chat takes live changes
    typed into it; a stopped one gets everything at relaunch. FlagError says what is wrong with `want`.
    root is the machine's Claude folder; ask is what its Claude Code calls "asks every time" (ask_mode)."""
    cur = view(s)
    have = {x['name']: x for x in cur['active']}
    hv = lambda n: (have.get(n) or {}).get('value')
    running = s.get('kind') == 'background' and s.get('status') in ('idle', 'busy', 'waiting')
    want = want if isinstance(want, dict) else {}
    other = want.get('other') if isinstance(want.get('other'), str) else ''
    # A client sends back every "Active now" row it was shown. One that an older server listed there though no
    # relaunch can pass it (--ide), or that is only read at the start (--worktree), is left out, not refused.
    skip = lambda n: n in CHIPS or n in VIEW_ONLY or n in START_ONLY or n in SWITCHBOARD_OWN
    sent = [f for f in want.get('flags') or []
            if not (isinstance(f, list) and f and isinstance(f[0], str) and skip(ALIASES.get(f[0], f[0])))]
    checked = parse(sent, other)
    if checked.get('model') or checked.get('effort') or checked.get('chrome') or any(f[0] in CHIPS for f in checked['flags']):
        raise FlagError('Pick the model, effort and Chrome above instead.')
    if any(f[0] in START_ONLY for f in checked['flags']):
        raise FlagError('A worktree is picked when the chat starts. Start a new chat for another one.')
    model = want.get('model') if model_ok(want.get('model')) else hv('--model')
    effort = want.get('effort') if want.get('effort') in EFFORT_WORD else hv('--effort')
    perm = want.get('perm') if want.get('perm') in PERM_WORD else hv('--permission-mode') or 'default'
    chrome = bool(want.get('chrome')) if 'chrome' in want else '--chrome' in have
    computer = bool(want.get('computer')) if isinstance(want.get('computer'), bool) else COMPUTER in have
    changes, cmds = [], []

    def change(name, frm, to, how, cmd=None):
        c = {'name': name, 'label': LABELS.get(name, name), 'from': frm, 'to': to, 'how': how if running else 'relaunch'}
        if cmd and running and how == 'live':
            c['cmd'] = cmd
            cmds.append(cmd)
        changes.append(c)

    word = lambda m: (model_word(m) or '') + (' (1M)' if isinstance(m, str) and m.endswith('[1m]') else '') or None
    if model != hv('--model'):
        change('--model', word(hv('--model')), word(model), 'live', f'/model {model}')
    if effort != hv('--effort'):
        change('--effort', EFFORT_WORD.get(hv('--effort')), EFFORT_WORD.get(effort), 'live',
               '/effort ultracode on' if effort == 'ultracode' else f'/effort {effort}')
    old_perm = hv('--permission-mode') or 'default'
    asks = perm in ASK_MODES
    if perm != old_perm and not (asks and old_perm in ASK_MODES):
        # a machine that cannot be told to ask first starts in its own default mode: say that, not "Asks first"
        change('--permission-mode', PERM_WORD.get(old_perm), PERM_WORD.get(perm) if ask or not asks else 'Machine default',
               'live' if perm == 'plan' else 'relaunch', '/plan' if perm == 'plan' else None)
    if chrome != ('--chrome' in have):
        change('--chrome', 'Off' if chrome else 'On', 'On' if chrome else 'Off', 'relaunch')
    if computer != (COMPUTER in have):
        change(COMPUTER, 'Off' if computer else 'On', 'On' if computer else 'Off', 'relaunch')
    old = [[x['name']] + ([x['value']] if x['value'] is not None else []) for x in cur['active']
           if x['name'] not in CHIPS and x['name'] not in SWITCHBOARD_OWN and x['name'] not in START_ONLY]
    # Chrome on and --no-chrome cancel out: turning it on drops the flag that kept it off
    new = [f for f in checked['flags'] if not (chrome and f[0] == '--no-chrome')]
    for f in new:
        if f not in old:
            if f[0] == '--add-dir' and not f[1].startswith(('~', '@')):
                path = full_path(root, f[1])
                change(f[0], None, f[1], 'live', f'/add-dir {shlex.quote(path) if any(c.isspace() for c in path) else path}')
            else:  # a folder in home or in a place: only the machine knows its path, so it goes in at a relaunch
                change(f[0], None, f[1] if len(f) > 1 else 'On', 'relaunch')
    for f in old:
        if f not in new:
            change(f[0], f[1] if len(f) > 1 else 'On', None, 'relaunch')
    relaunch = any(c['how'] == 'relaunch' for c in changes)
    flags = list(new) + fit_perm([['--permission-mode', perm]], ask) + ([['--chrome']] if chrome else [])
    return {'changes': changes, 'cmds': [] if relaunch else cmds, 'relaunch': relaunch, 'running': running,
            'busy': s.get('status') == 'busy', 'dropped': cur['other'],
            'body': dict({'model': model, 'effort': effort, 'flags': flags},
                         **({'computer': computer} if computer != (COMPUTER in have) else {}))}
