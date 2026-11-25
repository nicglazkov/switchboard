import json, os, sys, tempfile, importlib
from pathlib import Path

try:  # one temp folder for the run (temproot.py), also when a test file is run by itself
    from tests import temproot  # noqa: F401
except ImportError:
    sys.path.insert(0, str(Path(__file__).resolve().parent))
    import temproot  # noqa: F401

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'agent'))

_ours = {}


def fresh_sb(tmp):
    """Import sb with SB_HOME pointed at a temp dir. Nothing of the computer the tests run on is read: its Claude
    folder, its paired name and the terminal this run was started from all stay out (a test that sets one of these
    itself keeps its own value)."""
    os.environ['SB_HOME'] = str(Path(tmp) / 'sbhome')
    os.environ.pop('SB_INTERNAL', None)
    os.environ.pop('SSH_ORIGINAL_COMMAND', None)
    for k, v in (('SB_CLAUDE_DIR', Path(tmp) / 'no-claude'), ('SB_CLAUDE_JSON', Path(tmp) / 'no-claude.json'),
                 ('SB_NODE_JSON', Path(tmp) / '.config' / 'switchboard' / 'node.json')):
        if os.environ.get(k) in (None, _ours.get(k)):
            os.environ[k] = _ours[k] = str(v)
    import sb
    sb = importlib.reload(sb)
    sb.spawn_ask = lambda *a: None  # tests never start a real Haiku pass; test_ask swaps in its own
    return sb

def sb_options(sb, **on):
    """Turn opt-in settings on (or off) on this agent, as the server's poll would (server/options.py)."""
    sb.remember_identity({'options': dict({k: False for k in sb.OPTION_KEYS}, **on)})

def entry(kind, ts, **kw):
    d = {'type': kind, 'timestamp': ts, 'sessionId': 'S1'}
    d.update(kw)
    return d

def user(ts, text, **kw):
    return entry('user', ts, message={'role': 'user', 'content': text}, origin={'kind': 'human'}, **kw)

def assistant(ts, text=None, usage=None, tools=(), model='claude-opus-5-5', side=False):
    content = []
    if text: content.append({'type': 'text', 'text': text})
    for name, inp in tools: content.append({'type': 'tool_use', 'name': name, 'input': inp})
    return entry('assistant', ts, isSidechain=side, message={'role': 'assistant', 'model': model, 'content': content,
                 'usage': usage or {'input_tokens': 2, 'cache_creation_input_tokens': 100, 'cache_read_input_tokens': 1000, 'output_tokens': 50}})

def write_transcript(tmp, rows, name='t.jsonl'):
    p = Path(tmp) / name
    with open(p, 'w') as f:
        for r in rows: f.write(json.dumps(r) + '\n')
    return str(p)
