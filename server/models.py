"""The models the New sheet and the flags drawer offer (GET /api/models), from what the machines' Claude Code says.

No API key is needed: each machine's agent reads its own `claude --help` (the aliases it names under --model), the
user's availableModels setting, and the model catalog Claude Code keeps and refreshes itself (what each alias points
at today, with its name), and every chat's transcript names the full model id it ran on. Aliases always pick the
newest model of their family, so the list is the aliases, each with its current version (from the newest catalog any
machine has, or the newest a chat ran on, whichever is newer), plus any family no alias covers. With nothing
reported yet, the built-in list stands in."""

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


def _catalog(machines):
    """The newest model catalog any machine reported: {'targets': {alias: id}, 'names': {id: name}, 'main': [ids]}."""
    best = None
    for m in machines or []:
        info = m.get('models') if isinstance(m.get('models'), dict) else {}
        c = info.get('catalog')
        if isinstance(c, dict) and isinstance(c.get('models'), list) and (not best or (c.get('fetched') or 0) > (best.get('fetched') or 0)):
            best = c
    if not best:
        return {'targets': {}, 'names': {}, 'descs': {}, 'main': [], 'all': []}
    rows = [x for x in best['models'] if isinstance(x, dict) and isinstance(x.get('id'), str) and MODEL_ID_RE.fullmatch(x['id'])]
    t = best.get('targets') if isinstance(best.get('targets'), dict) else {}
    return {'targets': {a: i for a, i in t.items() if isinstance(i, str) and MODEL_ID_RE.fullmatch(i)},
            'names': {x['id']: x['name'] for x in rows if isinstance(x.get('name'), str) and x['name']},
            'descs': {x['id']: x['desc'] for x in rows if isinstance(x.get('desc'), str) and x['desc']},
            'main': [x['id'] for x in rows if x.get('main')], 'all': [x['id'] for x in rows]}


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
    cat = _catalog(machines)
    for a, mid in cat['targets'].items():  # what the alias points at today, even before any chat has run on it
        fam, ver = _version(mid)
        if fam == a and (fam not in newest or ver >= newest[fam][0]):
            newest[fam] = (ver, mid)
    for mid in cat['main']:  # a family the catalog offers that the targets do not cover yet
        fam, ver = _version(mid)
        if fam and fam not in cat['targets'] and (fam not in newest or ver > newest[fam][0]):
            newest[fam] = (ver, mid)
    aliases += reported_aliases(machines)
    if allowed_sets:  # narrowed on every machine that narrows it: an alias none of them allows is left out
        aliases = [a for a in aliases if any(_allowed(a, al) for al in allowed_sets)] or aliases
    word = lambda mid: cat['names'].get(mid) or model_word(mid)  # noqa: E731

    def row(mid, label, detail, version):
        return {'id': mid, 'label': label, 'detail': detail, 'description': cat['descs'].get(version, ''), 'version': version}
    out = [row(a, model_word(a), word(newest[a][1]) if a in newest else '', newest[a][1] if a in newest else '') for a in aliases]
    for fam, (_, mid) in sorted(newest.items()):
        if fam not in aliases:  # a family no alias here covers: its own id
            out.append(row(mid, word(mid), '', mid))
    # every other version the catalog offers, pinned by its full id: the picker's collapsed "More models"
    shown = {m['id'] for m in out} | {m['version'] for m in out}
    fams = set(aliases) | set(newest)
    order = list(aliases) + sorted(fams - set(aliases))
    ids = sorted((mid for mid in cat['all'] if mid not in shown and _version(mid)[0] in fams),
                 key=lambda mid: (order.index(_version(mid)[0]), tuple(-n for n in _version(mid)[1])))
    more = [row(mid, word(mid), '', mid) for mid in ids][:40]
    return {'ok': True, 'models': out, 'more': more, 'source': 'machines' if reported or newest else 'fallback'}
