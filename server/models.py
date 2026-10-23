"""The models the New sheet and the flags drawer offer (GET /api/models), from what the machines' Claude Code says.

No API key is needed: each machine's agent reads its own `claude --help` (the aliases it names under --model) and the
user's availableModels setting, and every chat's transcript names the full model id it ran on. Aliases always
pick the newest model of their family, so the list is the aliases, each with the newest version seen lately, plus
any family the chats ran on that no alias covers. With nothing reported yet, the built-in list stands in."""

import re

from flags import MODEL_ALIASES, MODEL_ID_RE, model_word

# The aliases every Claude Code this app knows takes, in the order the picker shows them. A machine's help text
# may name more (a new family); those join after these.
BASE = MODEL_ALIASES
ALIAS_RE = re.compile(r'[a-z][a-z0-9]{1,23}')
FALLBACK = [{'id': a, 'label': model_word(a), 'detail': ''} for a in BASE]


def _version(mid):
    x = MODEL_ID_RE.fullmatch(mid or '')
    return (x.group(1), (int(x.group(2)), int(x.group(3) or 0))) if x else (None, None)


def _allowed(alias, allowed):
    """availableModels lets an alias through when it names it, or names a model of its family."""
    return any(a == alias or a.startswith(f'claude-{alias}') or f'-{alias}-' in a for a in allowed)


def reported_aliases(machines):
    """The aliases the machines' Claude Code names in its help, beyond the known ones."""
    out = []
    for m in machines or []:
        info = m.get('models') if isinstance(m.get('models'), dict) else {}
        for a in info.get('aliases') or []:
            if isinstance(a, str) and ALIAS_RE.fullmatch(a) and a not in BASE and a != 'default' and a not in out:
                out.append(a)
    return out


def view(machines):
    """{ok, models: [{id, label, detail}], source: 'machines'|'fallback'} from the fleet snapshot's machines.
    `id` is what --model gets; `detail` names the newest version of that alias seen in the chats ("Opus 5.5")."""
    aliases, allowed_sets, newest = list(BASE), [], {}
    reported = False
    for m in machines or []:
        info = m.get('models') if isinstance(m.get('models'), dict) else None
        if info:
            reported = True
            if isinstance(info.get('allowed'), list) and info['allowed']:
                allowed_sets.append([a for a in info['allowed'] if isinstance(a, str)])
        for s in m.get('sessions') or []:
            mid = s.get('model') if isinstance(s.get('model'), str) else ''
            fam, ver = _version(mid)
            if fam and (fam not in newest or ver > newest[fam][0]):
                newest[fam] = (ver, re.sub(r'(-\d{8})?(-[a-z]+)?(\[1m\])?$', '', mid))
    aliases += reported_aliases(machines)
    if allowed_sets:  # narrowed on every machine that narrows it: an alias none of them allows is left out
        aliases = [a for a in aliases if any(_allowed(a, al) for al in allowed_sets)] or aliases
    out = [{'id': a, 'label': model_word(a), 'detail': model_word(newest[a][1]) if a in newest else ''} for a in aliases]
    for fam, (_, mid) in sorted(newest.items()):
        if fam not in aliases:  # a family the chats ran on that no alias here covers: its own id
            out.append({'id': mid, 'label': model_word(mid), 'detail': ''})
    return {'ok': True, 'models': out, 'source': 'machines' if reported or newest else 'fallback'}
