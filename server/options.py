"""Opt-in settings: features that send chat text to Claude, and changes to each machine's own Claude setup.
All are off for a new server. A server that already had state when this list arrived turns them on once
(migrate), so an install that had them keeps them. They travel to the agents with every poll (fleet._list_req)."""

# (key, kind, label, what it does when on, what happens when off). Shown in Settings as written.
OPTIONS = (
    ('ai_ask_cards', 'ai', 'Summarize waiting chats',
     "Uses your Claude plan to sum up what a waiting chat is asking. Its last message goes to Claude.",
     "When off, the card shows the chat's own words."),
    ('ai_chat_names', 'ai', 'Name new chats',
     "Uses your Claude plan to name a new chat from its first message, which goes to Claude.",
     'When off, a new chat is named from its first few words.'),
    ('ai_auto_machine', 'ai', 'Let Claude pick for Auto',
     'Uses your Claude plan to pick a machine for tasks sent to Auto. The task and each machine\'s tools, '
     'free memory and repo names go to Claude.',
     'When off, Auto goes by the tools the task needs, then a repo it names, then free memory.'),
    ('setup_subagent_rule', 'setup', 'Subagent model rule',
     'Adds a short rule to ~/.claude/CLAUDE.md on each machine: Haiku for single lookups, Sonnet for search and '
     'reading, and the default model for design and code.',
     'When off, Switchboard removes only the rule it added. Anything you wrote stays.'),
    ('setup_explore_agent', 'setup', 'Explore agent on Sonnet',
     'Writes ~/.claude/agents/Explore.md on each machine, so the Explore agent runs on Sonnet and only reads. '
     "If you wrote your own Explore.md, it's left alone.",
     'When off, Switchboard deletes the file only if it wrote it.'),
    # {k} and {n} are the value picked in Settings (compact_at), as "300k" and "300000": view() fills them in
    ('setup_compact_window', 'setup', 'Compact long chats at {k} tokens',
     'Sets CLAUDE_CODE_AUTO_COMPACT_WINDOW={n} in ~/.claude/settings.json on each machine, so long chats '
     'compact at about {k} tokens. A machine where you set a compact window yourself is left alone.',
     "When off, Switchboard removes the setting only if it's still the value Switchboard set, and Claude Code "
     'compacts where it does by default.'),
)
KEYS = tuple(o[0] for o in OPTIONS)
MIGRATION = 'opt-in-2026-10'  # recorded in prefs.json "migrations" once it has run, so it never runs again

# Compact long chats at (setup_compact_window is its switch): the tokens a chat may reach before Claude Code
# compacts it. Claude Code takes 100k to 1M and holds a bigger value to the model's own window. A server that had
# the switch before the value existed means 300k, which is also the value recommended.
COMPACT_MIN, COMPACT_MAX, COMPACT_DEFAULT = 100_000, 1_000_000, 300_000
COMPACT_STOPS = (150_000, 200_000, 300_000, 500_000)  # the slider's stops; the last stop after these is "off"


def compact_at(v):
    """The stored compact window: a whole number of tokens in range, else 300k (nothing stored yet, or not a value)."""
    return v if isinstance(v, int) and not isinstance(v, bool) and COMPACT_MIN <= v <= COMPACT_MAX else COMPACT_DEFAULT


def compact_check(v):
    """A compact window as POSTed: the same rule, but anything else is refused (ValueError) instead of replaced."""
    if compact_at(v) is not v:
        raise ValueError(f'compact_at is a whole number of tokens from {COMPACT_MIN} to {COMPACT_MAX}')
    return v


def compact_seen(d):
    """What a machine said its own Claude Code settings hold (agent compact_state), checked; None when it said
    nothing (an older helper) or nothing usable."""
    if not isinstance(d, dict) or not isinstance(d.get('set'), bool):
        return None
    v, raw = d.get('value'), d.get('raw')
    return {'set': d['set'], 'own': d.get('own') is True and d['set'],
            'value': v if isinstance(v, int) and not isinstance(v, bool) and COMPACT_MIN <= v <= COMPACT_MAX else None,
            'raw': raw[:24] if isinstance(raw, str) else None,
            'other': d.get('other') if d.get('other') in ('setting', 'models') else None}


def _k(n):
    return f'{n // 1000}k' if n % 1000 == 0 and n < 1_000_000 else '1M' if n == 1_000_000 else f'{n:,}'


# Notifications, one setting for each kind: (key, label, what it sends). All on unless turned off.
NOTIFY = (
    ('held', 'Held messages', 'When a message you typed into a big, cold chat gets held. One per message.'),
    ('jobs', 'Finished tasks', "When something you started from the app finishes or fails while you're not looking at it."),
    ('cleanup', 'Chats not in use', 'At most once a day, when idle chats are using memory.'),
    ('budget', 'Hard limit', 'When the daily usage budget stops new chats.'),
)
NOTIFY_KEYS = tuple(n[0] for n in NOTIFY)


def notify_clean(d):
    """{kind: bool} for every kind of notification; only an explicit false turns one off."""
    d = d if isinstance(d, dict) else {}
    return {k: d.get(k) is not False for k in NOTIFY_KEYS}


PUSH_ON = ('Notifications go through Apple. The server sends each one (chat names, machine names and short '
           'status lines) to Apple\'s push service, which delivers it to your iPhone.')
PUSH_OFF = ('Notifications are off. To turn them on, add an APNs key and ~/.config/switchboard/apns.json to the '
            'server. After that, each one (chat names, machine names and short status lines) goes through Apple\'s push service.')


def clean(d):
    """{key: bool} for every option; anything missing or not a bool is off."""
    d = d if isinstance(d, dict) else {}
    return {k: d.get(k) is True for k in KEYS}


def migrate(stored, existing):
    """The options after the one-time migration: (options, migrations, ran). `stored` is prefs.json as read; an
    install that had state before this change gets every option on, once."""
    done = [m for m in stored.get('migrations') or [] if isinstance(m, str)]
    opts = clean(stored)
    if MIGRATION in done:
        return opts, done, False
    if existing:
        opts = {k: True for k in KEYS}
    return opts, done + [MIGRATION], True


def view(opts, push_ready, notify=None, compact=None):
    """GET /api/prefs: each option with its words, the compact window with its range and stops, and whether
    notifications are set up."""
    at = compact_at(compact)
    fill = lambda words: words.replace('{k}', _k(at)).replace('{n}', str(at))  # noqa: E731
    return {'options': [{'key': k, 'kind': kind, 'label': fill(label), 'on': bool(opts.get(k)), 'explain': fill(on), 'off': off}
                        for k, kind, label, on, off in OPTIONS],
            'compact': {'at': at, 'on': bool(opts.get('setup_compact_window')), 'min': COMPACT_MIN, 'max': COMPACT_MAX,
                        'recommended': COMPACT_DEFAULT, 'stops': list(COMPACT_STOPS)},
            'notify_kinds': [{'key': k, 'label': label, 'explain': what, 'on': notify_clean(notify)[k]} for k, label, what in NOTIFY],
            'push': {'ready': bool(push_ready), 'explain': PUSH_ON if push_ready else PUSH_OFF,
                     'setup': None if push_ready else '~/.config/switchboard/apns.json'}}
