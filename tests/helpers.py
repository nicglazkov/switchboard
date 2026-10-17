import json, os, sys, tempfile, importlib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'agent'))

def fresh_sb(tmp):
    """Import sb with SB_HOME pointed at a temp dir."""
    os.environ['SB_HOME'] = str(Path(tmp) / 'sbhome')
    os.environ.pop('SB_INTERNAL', None)
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
