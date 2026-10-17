"""Opt-in settings: features that send chat text to Claude, and changes to each machine's own Claude setup.
All are off for a new server. A server that already had state when this list arrived turns them on once
(migrate), so an install that had them keeps them. They travel to the agents with every poll (fleet._list_req)."""

# (key, kind, label, what it does when on, what happens when off). Shown in Settings as written.
OPTIONS = (
    ('ai_ask_cards', 'ai', 'Summarize waiting chats',
     "Uses your Claude plan to write a short summary of what a waiting chat is asking. The chat's last message is sent to Claude.",
     'When off, the card shows what the chat says it needs, as it said it.'),
    ('ai_chat_names', 'ai', 'Name new chats',
     "Uses your Claude plan to name a new chat from its first message. The first message is sent to Claude.",
     'When off, a new chat is named from the first words of its message.'),
    ('ai_auto_machine', 'ai', 'Let Claude pick for Auto',
     'Uses your Claude plan to pick the machine for a task sent to Auto. The task, and each machine\'s tools, '
     'free memory and repo names, are sent to Claude.',
     'When off, Auto picks by fixed rules: the tools the task needs, a repo it names, then free memory.'),
    ('setup_subagent_rule', 'setup', 'Subagent model rule',
     'Adds a short rule to ~/.claude/CLAUDE.md on each machine: Haiku for single lookups, Sonnet for search and '
     'reading, and the default model for design and code.',
     'When off, Switchboard removes only the rule it added. Anything you wrote stays.'),
    ('setup_explore_agent', 'setup', 'Explore agent on Sonnet',
     'Writes ~/.claude/agents/Explore.md on each machine, so the Explore agent runs on Sonnet and only reads. '
     'An Explore.md you wrote yourself is never replaced.',
     'When off, Switchboard deletes the file only if it is the one Switchboard wrote.'),
    ('setup_compact_window', 'setup', 'Compact at 300k tokens',
     'Sets CLAUDE_CODE_AUTO_COMPACT_WINDOW=300000 in ~/.claude/settings.json on each machine, so long chats '
     'compact at about 300k tokens.',
     'When off, Switchboard removes the setting only if it still has the value Switchboard set.'),
)
KEYS = tuple(o[0] for o in OPTIONS)
MIGRATION = 'opt-in-2026-10'  # recorded in prefs.json "migrations" once it has run, so it never runs again

PUSH_ON = ('Notifications go through Apple: the server sends each one (chat names, machine names and short '
           'status lines) to Apple\'s push service, which delivers it to your iPhone.')
PUSH_OFF = ('Notifications are off. To turn them on, put an APNs key and ~/.config/switchboard/apns.json on the '
            'server. Each notification (chat names, machine names and short status lines) then goes through Apple\'s push service.')


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


def view(opts, push_ready):
    """GET /api/prefs: each option with its words, and whether notifications are set up."""
    return {'options': [{'key': k, 'kind': kind, 'label': label, 'on': bool(opts.get(k)), 'explain': on, 'off': off}
                        for k, kind, label, on, off in OPTIONS],
            'push': {'ready': bool(push_ready), 'explain': PUSH_ON if push_ready else PUSH_OFF,
                     'setup': None if push_ready else '~/.config/switchboard/apns.json'}}
