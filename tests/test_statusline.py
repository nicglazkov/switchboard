import json, re, tempfile, unittest
from pathlib import Path
from tests.helpers import fresh_sb

FIX = json.loads((Path(__file__).parent / 'fixtures' / 'statusline_sample.json').read_text())
strip = lambda s: re.sub(r'\x1b\[[0-9;]*m', '', s)

class StatusTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(); self.sb = fresh_sb(self.tmp)

    def data(self, ctx=None, warm=True, expires_in=1800, five=None, new=False, cwd=None):
        d = json.loads(json.dumps(FIX))
        if cwd:
            d['cwd'] = d['workspace']['current_dir'] = cwd
        if new:                              # what Claude Code sends before a session's first reply
            d['context_window']['current_usage'] = None
            d.pop('prompt_cache', None)
            d.pop('rate_limits', None)
            return d
        if ctx is not None:
            d['context_window']['current_usage'].update(input_tokens=0, cache_creation_input_tokens=0, cache_read_input_tokens=ctx)
        d['prompt_cache']['warm'] = warm
        d['prompt_cache']['expires_at'] = 1_000_000 + expires_in
        if five is not None:
            d['rate_limits']['five_hour'].update(used_percentage=five, resets_at=1_003_000)
        return d

    def render(self, cols=80, env=None, **kw):
        import os
        old = os.environ.get('COLUMNS')
        os.environ['COLUMNS'] = str(cols)
        os.environ.update(env or {})
        try:
            return self.sb.render_statusline(self.data(**{k: v for k, v in kw.items() if k != 'window'}), now=1_000_000,
                                             compact_window=kw.get('window', 300_000))
        finally:
            for k in env or {}:
                os.environ.pop(k, None)
            if old is None:
                os.environ.pop('COLUMNS', None)
            else:
                os.environ['COLUMNS'] = old

    def git_folder(self):
        p = Path(self.tmp) / 'proj'
        (p / '.git').mkdir(parents=True, exist_ok=True)
        (p / '.git' / 'HEAD').write_text('ref: refs/heads/main\n')
        return str(p)

    def seen_limits(self):
        """Another session's state file, for what a brand-new session shows."""
        (self.sb.paths()['state'] / 'aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee.json').write_text(json.dumps(
            {'five_hour': 31, 'seven_day': 78, 'five_hour_reset': 1_007_200, 'seven_day_reset': 1_259_200, 'updated': 999_700}))

    MODES = {  # name -> (render kwargs, extra env, the tag, whether a second row gives advice)
        'ok': (dict(ctx=40_000), {}, r'warm 30m', False),
        'guard': (dict(ctx=180_000, expires_in=1500), {}, r'warm 25m', True),
        'heavy': (dict(ctx=280_000), {}, r'warm 30m', True),
        'stale': (dict(ctx=180_000, warm=False, expires_in=-7200), {}, r'stale 2h', True),
        'cold': (dict(ctx=40_000, warm=False, expires_in=-3 * 3600), {}, r'cold 3h', False),
        'limit': (dict(ctx=40_000, five=86), {}, r'warm 30m', True),
        'new': (dict(new=True), {}, r'new', False),
        'fresh': (dict(new=True), {'SB_HANDOFF': '1'}, r'fresh', False),
    }

    def test_same_segments_in_every_state(self):
        self.seen_limits()
        cwd = self.git_folder()
        for cols in (80, 120, 200):
            for mode, (kw, env, tag, advice) in self.MODES.items():
                rows = self.render(cols, env=env, cwd=cwd, **kw).split('\n')
                first, label = strip(rows[0]), f'{mode} at {cols}'
                self.assertEqual(len(rows), 2 if advice else 1, label)
                self.assertEqual(len(first), cols - 4, label)                      # one full-width row every time
                self.assertRegex(first, r'^ ' + tag + ' ', label)                    # the tag, always first
                self.assertIn('Haiku', first, label)
                self.assertRegex(first, r'Haiku(?: 5\.5)? med(?:ium)? ', label)     # model and effort
                self.assertRegex(first, r'\d+k(?:/300k)? │', label)                # the context gauge
                self.assertRegex(first, r'│ ●{3,6} │', label)                        # the cache
                self.assertRegex(first, r'│ 5h ?. ?\d+% ', label)
                self.assertRegex(first, r'│ wk ?. ?\d+% ', label)
                self.assertRegex(first, r'[ /]proj main $', label)                    # folder and branch stay put
                if advice:
                    self.assertIn('\x1b[48;2;224;162;58m', rows[1], label)           # amber advice row
                    self.assertLessEqual(len(strip(rows[1])), cols - 4, label)

    def test_wide_rows_show_more_but_the_same_segments(self):
        cwd = self.git_folder()
        wide, narrow = strip(self.render(200, cwd=cwd, ctx=40_000)), strip(self.render(80, cwd=cwd, ctx=40_000))
        self.assertIn('Haiku 5.5 medium', wide)
        self.assertIn('40k/300k', wide)
        self.assertIn('●●●●●●', wide)
        self.assertRegex(wide, r'5h ▁ 6% \S+ │ wk ▄ 49% \S+')                    # reset times when there is room
        self.assertIn('/proj main', wide)                                            # the whole path
        self.assertIn('warm 30m', narrow)
        self.assertNotIn('/proj', narrow)

    def test_new_session_shows_placeholders_and_the_limits_last_seen(self):
        self.seen_limits()
        out = strip(self.render(120, new=True))
        self.assertTrue(out.startswith(' new '))
        self.assertRegex(out, r' 0k(/300k)? │ ●{6} │')
        self.assertRegex(out, r'5h ▃ 31%')
        self.assertRegex(out, r'wk ▇ 78%')
        self.assertTrue(strip(self.render(120, env={'SB_HANDOFF': '1'}, new=True)).startswith(' fresh '))
        for p in self.sb.paths()['state'].glob('*.json'):   # nothing seen anywhere yet: still every segment
            p.unlink()
        bare = strip(self.render(120, new=True, cwd=self.git_folder()))
        self.assertRegex(bare, r'│ 5h\s+-- │ wk\s+-- ')

    def test_limits_seen_before_a_reset_read_zero(self):
        (self.sb.paths()['state'] / 'aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee.json').write_text(json.dumps(
            {'five_hour': 64, 'seven_day': 78, 'five_hour_reset': 999_000, 'seven_day_reset': 1_259_200, 'updated': 990_000}))
        out = strip(self.render(120, new=True))
        self.assertRegex(out, r'5h . 0%')
        self.assertRegex(out, r'wk ▇ 78%')

    def test_one_row_that_fills_the_width(self):
        for cols in (80, 120, 200):
            out = self.render(cols, ctx=40_000)
            self.assertNotIn('\n', out)
            self.assertEqual(len(strip(out)), cols - 4, cols)        # the strip's own background runs edge to edge

    def test_core_fields(self):
        out = strip(self.render(80, ctx=40_000))
        self.assertIn('Haiku', out)
        self.assertIn('40k', out)
        self.assertIn('warm 30m', out)
        self.assertRegex(out, r'5h ?. ?6%')
        self.assertRegex(out, r'wk ?. ?49%')
        wide = strip(self.render(140, ctx=40_000))
        self.assertIn('medium', wide)             # effort and the limit's size show when there is room
        self.assertIn('40k/300k', wide)
        self.assertIn('●●●●●●', wide)             # one dot per 10 min left of the hour-long cache, rounded up

    def test_big_warm_chat_gets_the_reply_or_fresh_advice(self):
        out = self.render(80, ctx=180_000, expires_in=1500)
        top, advice = out.split('\n')
        self.assertIn('Claude', strip(top))            # the folder is still on row 1
        self.assertIn('big chat: reply while warm, or fresh, then /clear', strip(advice))
        self.assertNotIn('25m', strip(advice))         # the warm tag on row 1 already has the minutes
        self.assertIn('\x1b[38;2;224;162;58m', out)    # amber
        self.assertTrue(strip(self.render(140, ctx=180_000, warm=False, expires_in=-60)).split('\n')[1].strip()
                        .endswith('/clear'))

    def test_heavy_and_cold(self):
        self.assertIn('wrap up', strip(self.render(140, ctx=280_000)))
        self.assertIn('\x1b[38;2;239;106;94m', self.render(80, ctx=280_000))   # red
        cold = strip(self.render(140, ctx=180_000, warm=False, expires_in=-7200, window=None))
        self.assertIn('stale 2h', cold)
        self.assertIn('re-reads', cold)

    def test_small_chat_has_no_advice(self):
        out = self.render(80, ctx=40_000)
        self.assertNotIn('/clear', strip(out))
        self.assertNotIn('\x1b[48;2;224;162;58m', out)

    def test_very_narrow_terminal_keeps_the_advice(self):
        top, advice = (strip(r) for r in self.render(60, ctx=180_000, cwd=self.git_folder()).split('\n'))
        self.assertEqual(len(top), 56)                 # below 80 columns the row is cut at the right edge
        self.assertLessEqual(len(advice), 56)
        self.assertIn('/clear', advice)
        for part in ('warm', 'Haiku', 'k │', '●', '5h'):
            self.assertIn(part, top)

    def test_bad_input_never_blanks_the_line(self):
        cases = [None, [], 'x', 7, {}, {'model': None}, {'model': {'display_name': 5}},
                 {'model': {'display_name': 'Opus 5.5'}, 'context_window': {'current_usage': [1, 2]}},
                 {'model': {'display_name': 'Opus 5.5'}, 'rate_limits': {'five_hour': {'used_percentage': 'lots', 'resets_at': 'soon'}}},
                 {'model': {'display_name': 'Opus 5.5'}, 'cost': {'total_cost_usd': 'free'}, 'prompt_cache': {'warm': True, 'expires_at': 'x'}},
                 {'model': {'display_name': 'Opus 5.5'}, 'rate_limits': {'seven_day': {'used_percentage': 99, 'resets_at': 1e30}}}]
        for d in cases:
            out = self.sb.render_statusline(d, now=1_000_000, compact_window=None)
            self.assertTrue(strip(out).strip(), repr(d))
        self.assertIn('Opus 5.5', strip(self.sb.render_statusline(cases[-1], 1_000_000, None)))

    def test_write_and_read_state(self):
        d = self.data(ctx=200_000)
        self.sb.write_state(d, now=1_000_000)
        s = self.sb.read_state(d['session_id'])
        self.assertEqual(s['ctx_tokens'], 200_000)
        self.assertEqual(s['cache_expires_at'], 1_001_800)
        self.assertEqual(s['seven_day'], 49)
        self.assertEqual(self.sb.read_state('nope'), {})


class StatusEncodingTest(unittest.TestCase):
    def test_utf8_io_when_console_codepage_is_cp1252(self):
        # Windows pipes default to cp1252: the '·' separator crashed the status line there.
        import os, subprocess, sys
        sample = json.loads((Path(__file__).parent / 'fixtures' / 'statusline_sample.json').read_text())
        sample['workspace']['current_dir'] = '/tmp/café'
        with tempfile.TemporaryDirectory() as d:
            env = dict(os.environ, PYTHONIOENCODING='cp1252', SB_HOME=d)
            r = subprocess.run([sys.executable, str(Path(__file__).parent.parent / 'agent' / 'sb.py'), 'statusline'],
                               input=json.dumps(sample, ensure_ascii=False).encode('utf-8'),
                               capture_output=True, env=env, timeout=30)
        out = r.stdout.decode('utf-8')
        self.assertIn('│', out)
        self.assertIn('café', out)
