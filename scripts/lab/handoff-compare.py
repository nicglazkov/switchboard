#!/usr/bin/env python3
"""Opus vs Sonnet for Start fresh summaries, on real transcripts. Run it on a machine with Claude Code signed in:

    python3 scripts/lab/handoff-compare.py ~/.claude/projects/<project>/<sid>.jsonl [more.jsonl ...] > compare.md

For each transcript it condenses the chat exactly as a Start fresh does (agent/sb.py condense), asks each model for
the handoff note with the same HANDOFF_PROMPT, and reports:
- the tokens each run used (from `claude -p --output-format json`), and its cost estimate in API price ratios;
- which of the prompt's sections the note has;
- how many of the user's directives the note keeps word for word (the section that matters most);
- the note's length, and the note itself, so the two can be read side by side.

The notes go to <out-dir> (default ./handoff-compare) and the report to stdout. Nothing is sent anywhere but
Claude Code, and the notes stay on this machine: they hold the chats' content. Do not commit them.
"""
import json
import os
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / 'agent'))
import sb  # noqa: E402

MODELS = ('opus', 'sonnet')
SECTIONS = ('## Goal', '## Standing instructions from the user', '## Decisions made', '## Current state',
            '## Open threads and next steps', '## Gotchas', '## Key files and places')


def run(model, text):
    """One `claude -p` run, as run_claude_opus makes it, keeping the usage numbers it reports."""
    r = sb.run_tree([sb._claude_exe(), '-p', '--model', model, '--no-session-persistence', '--tools', '',
                     '--setting-sources', '', '--strict-mcp-config', '--system-prompt', sb.HANDOFF_PROMPT,
                     '--output-format', 'json'], text, 900, env=sb.claude_env(), cwd=str(Path.home()))
    d = json.loads(r.stdout or '{}')
    if r.returncode != 0 or d.get('is_error') or not d.get('result'):
        raise RuntimeError(f'{model}: claude -p failed rc={r.returncode} {r.stderr[-300:]!r}')
    return d['result'], d.get('usage') or {}, d.get('total_cost_usd')


def weight(u):
    """Token weight in API price ratios, as the cost model counts it."""
    r = sb.COST_RATIOS
    return round((u.get('input_tokens') or 0) * r['input'] + (u.get('cache_creation_input_tokens') or 0) * r['cache_write_5m']
                 + (u.get('cache_read_input_tokens') or 0) * r['cache_read'] + (u.get('output_tokens') or 0) * r['output'])


def kept(note, directives):
    flat = ' '.join(note.split())
    return sum(1 for d in directives if ' '.join(d.split()) in flat)


def main(argv):
    paths = [a for a in argv[1:] if not a.startswith('--')]
    out = Path(next((a.split('=', 1)[1] for a in argv[1:] if a.startswith('--out=')), 'handoff-compare'))
    if not paths:
        sys.stderr.write(__doc__)
        return 2
    out.mkdir(parents=True, exist_ok=True)
    print('# Opus vs Sonnet for Start fresh summaries\n')
    print('| Transcript | Condensed | Model | Seconds | Tokens in / out | Weight | Sections | Directives kept | Words |')
    print('|---|---|---|---|---|---|---|---|---|')
    for p in paths:
        c = sb.condense(p)
        text = sb.render_condensed(c)
        size = round(len(text) / sb.CHARS_PER_TOKEN)
        for m in MODELS:
            t0 = time.time()
            try:
                note, u, usd = run(m, text)
            except Exception as e:  # one failed run does not end the comparison
                print(f'| {Path(p).stem[:8]} | {sb._k(size)} | {m} | | failed: {e} | | | | |')
                continue
            secs = round(time.time() - t0)
            (out / f'{Path(p).stem}.{m}.md').write_text(note, encoding='utf-8')
            tin = (u.get('input_tokens') or 0) + (u.get('cache_creation_input_tokens') or 0) + (u.get('cache_read_input_tokens') or 0)
            sec = sum(1 for h in SECTIONS if h in note)
            print(f'| {Path(p).stem[:8]} | {sb._k(size)} | {m} | {secs} | {sb._k(tin)} / {sb._k(u.get("output_tokens") or 0)} | '
                  f'{sb._k(weight(u))}{f" (${usd:.2f} at API prices)" if isinstance(usd, (int, float)) else ""} | {sec}/{len(SECTIONS)} | '
                  f'{kept(note, c["directives"])}/{len(c["directives"])} | {len(note.split())} |')
    print(f'\nThe notes are in {out}{os.sep}. Read the "Standing instructions" sections side by side: a summary that drops a '
          'rule the user gave costs more later than the tokens it saves now.')
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv))
