"""The tool library (docs/specs/2026-10-13-tool-library.md): every version of every tool, which machines have which,
the tools every new chat gets, and each chat's own list. A tool is a Claude Code plugin folder sent as a tar.gz; it is
checked here before it is kept, and the machines check it again before they unpack it. Secrets never come here: each
machine keeps its own in its keychain, and only their names travel.

server.App owns one Library through Features and calls: plan_tools() when a job is submitted, ready() just before a
job reaches its machine, job_done() when it finishes, decorate() on /api/sessions, after_poll() from the poller, and
get()/post() for the /api/library routes."""
import base64, hashlib, io, json, os, re, shutil, tarfile, threading, time, traceback, uuid, zlib
from pathlib import Path

TOOL_RE = re.compile(r'[a-z0-9][a-z0-9-]{0,39}')
SECRET_NAME_RE = re.compile(r'[A-Za-z_][A-Za-z0-9_]{0,63}')
SID_RE = re.compile(r'[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}')
UPLOAD_ID = re.compile(r'[0-9a-f]{32}')
HEX64_RE = re.compile(r'[0-9a-f]{64}')
BUNDLE_MAX = 25 * 1024 * 1024          # the same caps as the agent (sb.py BUNDLE_*)
BUNDLE_UNPACKED_MAX = 100 * 1024 * 1024
BUNDLE_FILES_MAX = 4000
BUNDLE_META_MAX = 256 * 1024
PUSH_CHUNK_NODE = 1024 * 1024          # as server.UPLOAD_CHUNK_*: a hub link takes 2 MB a request, base64 adds a third
PUSH_CHUNK_SSH = 6 * 1024 * 1024
TOOLS_MAX = 32                         # on one chat
STATE_EVERY = 300                      # a machine's library_state is asked again at most this often by the poller
UPLOAD_KEEP = 3600                     # a publish that never sent its last chunk is dropped after an hour
CHATS_KEEP = 3000
BRIEF_MAX = 4000
WHEN_MAX = 300                         # the "when to use it" hint a chat's catalog shows
TOOL_NAME_ERR = 'A tool name is 1 to 40 lowercase letters, digits and dashes, starting with a letter or digit.'
NEW_TOOL_PROMPT = """Build a Claude Code tool named "{tool}" for Switchboard's tool library, here in this folder.

What it should do: {brief}

Lay this folder out as a Claude Code plugin folder:
- .claude-plugin/plugin.json: {{"name": "{tool}", "description": "<one line>", "version": "1"}}
- skills/<skill name>/SKILL.md, plus any scripts or CLIs the skill runs, for skills
- .mcp.json: {{"mcpServers": {{...}}}} for MCP servers; write paths inside this folder as ${{CLAUDE_PLUGIN_ROOT}}/...
- files/: anything else an MCP server needs

If it needs API keys or other secrets, never write their values anywhere. List their names in
.claude-plugin/switchboard.json as {{"description": "<one line>", "secrets": [{{"name": "API_KEY", "about": "<what it is>"}}],
"mode": "local"}} and read them from the environment. Prefer an MCP server for anything that needs a secret: a skill's
scripts run through Bash, where a secret could be read.

Test it here. When it works, call the switchboard MCP's publish_tool with path "." and tool "{tool}"."""


def log(msg):
    print(time.strftime('%Y-%m-%d %H:%M:%S'), msg, flush=True)


def jload(p, default):
    try:
        d = json.loads(Path(p).read_text())
        return d if isinstance(d, type(default)) else default
    except (OSError, ValueError):
        return default


def jsave(p, obj):
    p = Path(p)
    tmp = p.with_name(f'{p.name}.{os.getpid()}.{threading.get_ident()}.tmp')
    tmp.write_text(json.dumps(obj, indent=1))
    os.replace(tmp, p)


def ck(machine, sid):
    return f'{machine}/{sid}'


def valid_tool(s):
    return isinstance(s, str) and TOOL_RE.fullmatch(s) is not None


def tool_slug(s):
    """The agent's tool_slug: a tool name from a skill folder or an MCP server's name."""
    t = re.sub(r'-{2,}', '-', re.sub(r'[^a-z0-9]+', '-', str(s or '').lower())).strip('-')[:40].strip('-')
    return t or 'tool'


# ---- bundles: the agent's bundle_info, kept the same by a test (tests/test_library.py)
def _member_parts(name):
    if not isinstance(name, str) or not name or '\x00' in name or '\\' in name or name.startswith('/') or re.match(r'^[A-Za-z]:', name):
        return None
    parts = [p for p in name.split('/') if p not in ('', '.')]
    if not parts or '..' in parts:
        return None
    return parts


def _read_meta(tf, m):
    if m.size > BUNDLE_META_MAX:
        raise ValueError(f'{m.name} is too big.')
    try:
        d = json.loads(tf.extractfile(m).read().decode('utf-8'))
    except (ValueError, UnicodeDecodeError):
        raise ValueError(f'{m.name} is not valid JSON.')
    if not isinstance(d, dict):
        raise ValueError(f'{m.name} must be a JSON object.')
    return d


def check_secrets(v):
    if v is None:
        return []
    if not isinstance(v, list) or len(v) > 32:
        raise ValueError('secrets must be a list of up to 32 {"name", "about"}')
    out = []
    for s in v:
        name = s.get('name') if isinstance(s, dict) else s
        if not isinstance(name, str) or not SECRET_NAME_RE.fullmatch(name):
            raise ValueError('A secret name is letters, digits and _ (like WEB_FETCH_KEY).')
        about = s.get('about') if isinstance(s, dict) else ''
        if name not in [x['name'] for x in out]:
            out.append({'name': name, 'about': str(about or '')[:300]})
    return out


def check_bundle(data, tool=None):
    """What a bundle holds, once it is known to be safe: plain files and folders only, every path inside the tool
    folder, under the caps, with a plugin.json naming the tool and at least one skill or MCP server."""
    if not isinstance(data, (bytes, bytearray)) or not data:
        raise ValueError('The bundle is empty.')
    if len(data) > BUNDLE_MAX:
        raise ValueError(f'The bundle is over {BUNDLE_MAX // (1024 * 1024)} MB.')
    seen, total, plugin, sbj, mcp, skills = set(), 0, None, {}, None, set()
    try:
        with tarfile.open(fileobj=io.BytesIO(bytes(data)), mode='r:gz') as tf:
            for i, m in enumerate(tf):
                if i >= BUNDLE_FILES_MAX:
                    raise ValueError(f'The bundle has more than {BUNDLE_FILES_MAX} files.')
                parts = _member_parts(m.name)
                if parts is None:
                    raise ValueError(f'{m.name[:120]!r} points outside the tool folder.')
                if not (m.isfile() or m.isdir()):
                    raise ValueError(f'{m.name[:120]!r} is a link or a device. A tool holds plain files only.')
                key = '/'.join(parts).lower()
                if key in seen:
                    raise ValueError(f'{m.name[:120]!r} is in the bundle twice.')
                seen.add(key)
                if m.isdir():
                    continue
                total += m.size
                if total > BUNDLE_UNPACKED_MAX:
                    raise ValueError(f'The tool unpacks to over {BUNDLE_UNPACKED_MAX // (1024 * 1024)} MB.')
                path = '/'.join(parts)
                if path == '.claude-plugin/plugin.json':
                    plugin = _read_meta(tf, m)
                elif path == '.claude-plugin/switchboard.json':
                    sbj = _read_meta(tf, m)
                elif path == '.mcp.json':
                    mcp = _read_meta(tf, m)
                elif len(parts) == 3 and parts[0] == 'skills' and parts[2] == 'SKILL.md':
                    skills.add(parts[1])
    except (tarfile.TarError, EOFError, zlib.error, OSError):
        raise ValueError('That is not a tar.gz bundle.')
    if plugin is None:
        raise ValueError('The bundle has no .claude-plugin/plugin.json.')
    name = plugin.get('name')
    if not valid_tool(name):
        raise ValueError(f'plugin.json: {TOOL_NAME_ERR}')
    if tool is not None and name != tool:
        raise ValueError(f'plugin.json names the tool {name!r}, not {tool!r}.')
    mode = sbj.get('mode') or 'local'
    if mode != 'local':
        raise ValueError('Only local tools can be published for now.')
    kinds = (['skill'] if skills else []) + (['mcp'] if mcp is not None and isinstance(mcp.get('mcpServers'), dict) and mcp['mcpServers'] else [])
    if not kinds:
        raise ValueError('The tool has no skills/<name>/SKILL.md and no .mcp.json servers, so a chat would get nothing.')
    if mcp is not None and not isinstance(mcp.get('mcpServers'), dict):
        raise ValueError('.mcp.json needs {"mcpServers": {...}}.')
    secrets = check_secrets(sbj.get('secrets'))
    desc = sbj.get('description') or plugin.get('description') or ''
    return {'name': name, 'description': str(desc)[:500], 'kinds': kinds, 'mode': mode, 'secrets': secrets,
            'skill_secrets': 'skill' in kinds and bool(secrets), 'skills': sorted(skills),
            'servers': sorted(mcp['mcpServers']) if 'mcp' in kinds else [], 'files': len(seen), 'unpacked': total}


class Library:
    def __init__(self, app, http_error, background=None):
        self.app, self.HttpError = app, http_error
        self.root = Path(app.state_dir) / 'library'
        self.file = Path(app.state_dir) / 'library.json'
        self.state_file = Path(app.state_dir) / 'library_state.json'
        self.lock = threading.RLock()
        self.push_locks, self.refreshing, self.errors = {}, set(), {}
        self.background = background or (lambda fn: threading.Thread(target=fn, daemon=True).start())
        self.d = self._load()
        self.states = jload(self.state_file, {})  # machine -> its last library_state answer, kept for when it is offline

    # ---- the store
    def _load(self):
        d = jload(self.file, {})
        d['tools'] = d.get('tools') if isinstance(d.get('tools'), dict) else {}
        d['defaults'] = [t for t in d.get('defaults') or [] if t in d['tools']] if isinstance(d.get('defaults'), list) else []
        for k in ('machines', 'chats', 'modes'):  # modes: {machine/sid: 'auto'} for chats whose tools are picked at launch
            d[k] = d.get(k) if isinstance(d.get(k), dict) else {}
        return d

    def _save(self):  # caller holds lock
        if len(self.d['chats']) > CHATS_KEEP:
            for k in list(self.d['chats'])[:len(self.d['chats']) - CHATS_KEEP]:
                del self.d['chats'][k]
                self.d['modes'].pop(k, None)
        try:
            jsave(self.file, self.d)
        except OSError:
            log('could not save library.json: ' + traceback.format_exc(limit=1))

    def _bundle(self, tool, v):
        return self.root / tool / f'{v}.tar.gz'

    def _err(self, code, msg):
        return self.HttpError(code, msg)

    def tool_name(self, v):
        if not valid_tool(v):
            raise self._err(400, TOOL_NAME_ERR)
        return v

    def known(self, v):
        self.tool_name(v)
        if v not in self.d['tools']:
            raise self._err(404, f'There is no tool {v} in the library.')
        return v

    def check_tools(self, v):
        """A tool list from a request: known names, each once; or "auto" (plan_tools picks them at submit)."""
        if v is None:
            return []
        if v == 'auto':
            return 'auto'
        if not isinstance(v, list) or len(v) > TOOLS_MAX:
            raise self._err(400, f'tools must be a list of up to {TOOLS_MAX} tool names')
        return list(dict.fromkeys(self.known(t) for t in v))

    def _label(self, name):
        return self.app._label(name)

    def has_feature(self, name):
        return 'library' in ((self.app.fleet.state.get(name) or {}).get('features') or [])

    def _ready(self, name):
        """The machine is online and its agent knows the library, or an HttpError that says what to do."""
        st, label = self.app.fleet.state.get(name) or {}, self._label(name)
        if not st.get('online'):
            raise self._err(409, f'{label} is offline. Try again once it is back.')
        if not self.has_feature(name):
            raise self._err(409, f'Update Switchboard on {label} to use the tool library.')
        return name

    def tools_meta(self, tools):
        """What a chat's catalog says about each tool (agent: tools_catalog)."""
        with self.lock:
            out = {}
            for n in tools or []:
                t = self.d['tools'].get(n)
                if t:
                    out[n] = {'when': t.get('when') or t.get('description') or '', 'kinds': list(t.get('kinds') or []),
                              'latest': t.get('latest')}
            return out

    def store(self, blob, tool, by, note='', description=None, when=None):
        """Keep a checked bundle as the tool's next version and make it latest; the machines that have the tool get
        it in the background. The same bytes as the latest version add nothing."""
        try:
            info = check_bundle(blob, tool)
        except ValueError as e:
            raise self._err(400, str(e))
        sha = hashlib.sha256(blob).hexdigest()
        with self.lock:
            t = self.d['tools'].get(tool) or {'description': '', 'versions': []}
            cur = next((x for x in t['versions'] if x.get('v') == t.get('latest')), None)
            if cur and cur.get('sha256') == sha:
                return {'ok': True, 'tool': tool, 'version': cur['v'], 'same': True}
            v = max([x.get('v') or 0 for x in t['versions']] + [0]) + 1
            p = self._bundle(tool, v)
            p.parent.mkdir(parents=True, exist_ok=True)
            tmp = p.with_name(f'.{v}.{uuid.uuid4().hex}.tmp')
            tmp.write_bytes(blob)
            os.replace(tmp, p)
            t.update(description=(description or info['description'] or t.get('description') or '')[:500], kinds=info['kinds'],
                     mode=info['mode'], secrets=info['secrets'], skill_secrets=info['skill_secrets'], latest=v)
            t.setdefault('auto', True)
            if isinstance(when, str) and when.strip():
                t['when'] = ' '.join(when.split())[:WHEN_MAX]
            t['versions'].append({'v': v, 'at': self.app.clock(), 'by': str(by or '')[:64], 'sha256': sha, 'size': len(blob),
                                  'note': str(note or '')[:500]})
            self.d['tools'][tool] = t
            self._save()
        log(f'library: {tool} v{v} from {by}')
        self._push_all(tool)
        self.app.bump()
        return {'ok': True, 'tool': tool, 'version': v}

    def _connected(self, tool):
        return [m for m, ts in self.d['machines'].items() if tool in ts and m in self.app.fleet.machines]

    def _push_all(self, tool):
        """Every machine that has the tool gets its latest version; one that is offline gets it when it is back."""
        for m in self._connected(tool):
            if (self.app.fleet.state.get(m) or {}).get('online') and self.has_feature(m):
                self.background(lambda m=m: self._try(self.push, m, tool))

    def _try(self, fn, *a):
        try:
            fn(*a)
        except Exception as e:
            log(f'library: {fn.__name__}{a}: {e}')

    # ---- moving bundles to and from the machines
    def _step(self, name):
        node = (self.app.fleet.machines.get(name) or {}).get('target', '').startswith('node:')
        return PUSH_CHUNK_NODE if node else PUSH_CHUNK_SSH

    def push(self, name, tool, refresh=True):
        """The tool's latest version to one machine (library_put, in chunks). Idempotent: the agent answers skipped
        when it already has that version with the same sha256, and makes it current."""
        with self.lock:
            t = self.d['tools'].get(tool)
            v = t and t.get('latest')
        if not v:
            raise self._err(404, f'There is no tool {tool} in the library.')
        data = self._bundle(tool, v).read_bytes()
        sha, step = hashlib.sha256(data).hexdigest(), self._step(name)
        with self.lock:
            lk = self.push_locks.setdefault((name, tool), threading.Lock())
        try:
            with lk:
                for off in range(0, len(data), step):
                    r = self.app._call(name, {'verb': 'library_put', 'tool': tool, 'version': v, 'sha256': sha, 'offset': off,
                                              'size': len(data), 'done': off + step >= len(data),
                                              'data': base64.b64encode(data[off:off + step]).decode()}, timeout=90)
                    if r.get('skipped'):
                        break
        except self.HttpError as e:
            self.errors[(name, tool)] = str(e)[:300]
            raise
        self.errors.pop((name, tool), None)
        with self.lock:
            st = self.states.setdefault(name, {})
            st.setdefault('tools', {}).setdefault(tool, {})['installed'] = v
        if refresh:
            self._try(self.refresh, name)
        return v

    def refresh(self, name):
        """The machine's library_state, kept for /api/library (and for when it is offline)."""
        r = self.app._call(name, {'verb': 'library_state'}, timeout=30)
        with self.lock:
            self.states[name] = {'at': self.app.clock(), 'tools': r.get('tools') if isinstance(r.get('tools'), dict) else {},
                                 'keychain': r.get('keychain'), 'warning': r.get('warning')}
            try:
                jsave(self.state_file, self.states)
            except OSError:
                pass
        return self.states[name]

    def reconcile(self, name):
        """A machine that was away catches up: tools it should have get their latest version, tools it should not
        (disconnected or deleted while it was offline) are dropped."""
        st = self.refresh(name)
        with self.lock:
            want = [t for t in self.d['machines'].get(name) or [] if t in self.d['tools']]
            latest = {t: self.d['tools'][t].get('latest') for t in want}
        for tool, row in (st.get('tools') or {}).items():
            if valid_tool(tool) and tool not in want:
                self._try(self._drop, name, tool)
        for tool in want:
            if ((st.get('tools') or {}).get(tool) or {}).get('installed') != latest[tool]:
                self._try(self.push, name, tool, False)

    def _drop(self, name, tool):
        self.app._call(name, {'verb': 'library_drop', 'tool': tool}, timeout=60)
        with self.lock:
            ((self.states.get(name) or {}).get('tools') or {}).pop(tool, None)

    def after_poll(self):
        """Every online machine with the library is asked what it has at most every STATE_EVERY seconds."""
        now = self.app.clock()
        for name in list(self.app.fleet.machines):
            st = self.app.fleet.state.get(name) or {}
            if not st.get('online') or not self.has_feature(name) or name in self.refreshing:
                continue
            if now - ((self.states.get(name) or {}).get('at') or 0) < STATE_EVERY:
                continue
            self.refreshing.add(name)

            def run(name=name):
                try:
                    self.reconcile(name)
                except Exception as e:
                    log(f'library: {name}: {e}')
                finally:
                    self.refreshing.discard(name)
            self.background(run)

    # ---- chats: what a start, resume or relaunch carries, and what a finished job records
    def plan_tools(self, kind, machine, plan):
        """At submit: the job's plan gets the tool list its chat starts with. New chats (start, fresh) get the
        defaults too; resume, relaunch, fresh, compact and continue carry the chat's recorded list."""
        with self.lock:
            defaults = [t for t in self.d['defaults'] if t in self.d['tools']]
            chats = self.d['chats']
            req = plan.get('req') or {}
            auto = list(dict.fromkeys(sorted(n for n, t in self.d['tools'].items() if t.get('auto') is not False) + defaults))
            if kind == 'start':
                if plan.get('tools') == 'auto':  # picked now from the library as it is; the machine drops what lacks a key
                    plan['tools'], plan['tools_asked'], plan['tools_mode'] = auto, [], 'auto'
                else:
                    asked = [t for t in plan.get('tools') or [] if t in self.d['tools']]
                    plan['tools'], plan['tools_asked'] = list(dict.fromkeys(asked + defaults)), asked
            elif kind in ('resume', 'fresh', 'relaunch', 'compact') and isinstance(req.get('sid'), str):
                key = ck(machine, req['sid'])
                rec = chats.get(key)
                if self.d['modes'].get(key) == 'auto':  # an auto chat gets the library as it is now
                    rec, plan['tools_mode'] = auto, 'auto'
                if kind == 'fresh' and (rec or defaults):
                    rec = list(dict.fromkeys((rec or []) + defaults))
                if rec is not None:
                    plan['tools'] = [t for t in rec if t in self.d['tools']]
            elif kind == 'continue' and isinstance(plan.get('sid'), str):
                key = ck(plan.get('src'), plan['sid'])
                rec = chats.get(key)
                if self.d['modes'].get(key) == 'auto':
                    rec, plan['tools_mode'] = auto, 'auto'
                if rec is not None:
                    plan['tools'] = [t for t in rec if t in self.d['tools']]
        return plan

    def ready(self, name, plan, req):
        """Just before the job reaches its machine: tools it does not have yet are pushed first, and the request
        carries the list. An agent without the library gets nothing extra, unless tools were asked for by name."""
        tools = plan.get('tools')
        if tools is None or req.get('verb') not in ('start', 'resume', 'fresh', 'compact'):
            return req
        if not self.has_feature(name):
            if plan.get('tools_asked'):
                raise self._err(409, f'Update Switchboard on {self._label(name)} to use tools from the library.')
            return req
        for tool in tools:
            with self.lock:
                have = ((self.states.get(name) or {}).get('tools') or {}).get(tool) or {}
                latest = (self.d['tools'].get(tool) or {}).get('latest')
                fresh = tool in (self.d['machines'].get(name) or []) and have.get('installed') == latest
            if fresh:
                continue
            try:
                self.push(name, tool, refresh=False)
                self._connect_record(name, tool)
            except self.HttpError as e:  # the chat still starts; the agent names what it could not find
                log(f'library: {tool} to {name} before a start: {e}')
        out = dict(req, tools=list(tools), tools_meta=self.tools_meta(tools))
        if plan.get('tools_mode') == 'auto':
            out['tools_mode'] = 'auto'
        return out

    def _connect_record(self, name, tool):
        with self.lock:
            ts = self.d['machines'].setdefault(name, [])
            if tool not in ts:
                ts.append(tool)
                self._save()

    def job_done(self, job):
        """A finished start, resume, fresh, relaunch, compact or continue: the new chat keeps the list it started with."""
        try:
            plan = job.get('plan') or {}
            tools, sid = plan.get('tools'), job.get('sid')
            if job.get('state') != 'done' or tools is None or not isinstance(sid, str) or not SID_RE.fullmatch(sid):
                return
            got = job.get('tools')  # what the machine attached (an auto chat leaves out tools that lack a key there)
            if isinstance(got, list) and all(valid_tool(t) for t in got):
                tools = got
            with self.lock:
                key = ck(job['machine'], sid)
                self.d['chats'].pop(key, None)  # newest last, so the oldest go first when the list is trimmed
                self.d['chats'][key] = list(tools)
                if plan.get('tools_mode') == 'auto':
                    self.d['modes'][key] = 'auto'
                else:
                    self.d['modes'].pop(key, None)
                self._save()
        except Exception:
            log('library job_done failed: ' + traceback.format_exc(limit=2))

    def decorate(self, snap):
        with self.lock:
            chats, modes = dict(self.d['chats']), dict(self.d['modes'])
        for m in snap['machines']:
            for s in (m.get('sessions') or []) + (m.get('replaced') or []):
                key = ck(m['name'], s.get('sid'))
                t = chats.get(key)
                if t:
                    s['tools'] = list(t)
                if t is not None:
                    s['tools_mode'] = 'auto' if modes.get(key) == 'auto' else 'pick'

    # ---- routes
    def view(self):
        with self.lock:
            tools = [dict({k: t.get(k) for k in ('description', 'kinds', 'mode', 'secrets', 'skill_secrets', 'latest', 'versions')},
                          name=n, auto=t.get('auto') is not False, when=t.get('when') or '')
                     for n, t in sorted(self.d['tools'].items())]
            defaults = list(self.d['defaults'])
            conn = {m: set(ts) for m, ts in self.d['machines'].items()}
            states = json.loads(json.dumps(self.states))
            errors = dict(self.errors)
        machines, keychains = {}, {}
        for name in self.app.fleet.machines:
            has, cached = self.has_feature(name), states.get(name) or {}
            row = {}
            for t in tools:
                n = t['name']
                inst = (cached.get('tools') or {}).get(n) or {}
                err = errors.get((name, n)) or inst.get('error')
                if not has and n in conn.get(name, ()):
                    err = err or f'Update Switchboard on {self._label(name)} to use the tool library.'
                row[n] = {'connected': n in conn.get(name, ()), 'installed': inst.get('installed'),
                          'missing_secrets': list(inst.get('missing_secrets') or []), 'error': err}
            machines[name] = row
            if cached.get('keychain'):
                keychains[name] = {'kind': cached['keychain'], 'warning': cached.get('warning')}
        return {'ok': True, 'tools': tools, 'defaults': defaults, 'machines': machines, 'keychains': keychains}

    def sources(self, query):
        name = self.app._machine(((query or {}).get('machine') or [''])[0])
        self._ready(name)
        r = self.app._call(name, {'verb': 'library_sources'}, timeout=60)
        return {'ok': True, 'skills': r.get('skills') or [], 'mcp': r.get('mcp') or []}

    def import_(self, body):
        """Pack a skill or MCP server on its machine, pull it here in chunks and keep it as the tool's next version."""
        name = self._ready(self.app._machine(body))
        kind, source = body.get('kind'), body.get('source')
        if kind not in ('skill', 'mcp') or not isinstance(source, str) or not source or len(source) > 1024:
            raise self._err(400, 'Say what to import: kind "skill" or "mcp", and its source.')
        tool = self.tool_name(body['tool']) if body.get('tool') else tool_slug(source.replace('\\', '/').rstrip('/').rsplit('/', 1)[-1])
        desc = body.get('description') if isinstance(body.get('description'), str) else None
        req = {'verb': 'library_pack', 'kind': kind, 'source': source, 'tool': tool}
        if desc:
            req['description'] = desc[:500]
        if isinstance(body.get('scope'), str):
            req['scope'] = body['scope']
        r = self.app._call(name, req, timeout=120)
        size, sha, pid = r.get('size'), r.get('sha256'), r.get('id')
        if not isinstance(size, int) or not 0 < size <= BUNDLE_MAX or not isinstance(sha, str) or not isinstance(pid, str):
            raise self._err(502, f'{self._label(name)} sent back a pack that is too big or incomplete.')
        buf, step = bytearray(), self._step(name)
        while len(buf) < size:
            c = self.app._call(name, {'verb': 'library_chunk', 'id': pid, 'offset': len(buf), 'size': step}, timeout=90)
            try:
                part = base64.b64decode(c.get('data') or '', validate=True)
            except (ValueError, TypeError):
                raise self._err(502, f'{self._label(name)} sent back a damaged chunk.')
            if not part:
                break
            buf += part
            if len(buf) > size:
                break
        if len(buf) != size or hashlib.sha256(buf).hexdigest() != sha:
            raise self._err(502, f'The tool arrived from {self._label(name)} damaged. Try again.')
        return self.store(bytes(buf), tool, name, f'Imported from {self._label(name)}', desc)

    def publish(self, body):
        """POST /api/library/publish, from the publish_tool MCP tool: {upload, offset, data, done?}. The last chunk
        carries done: {tool, description?, note?, sha256, machine?} and answers {tool, version}."""
        uid, off = body.get('upload'), body.get('offset')
        if not isinstance(uid, str) or not UPLOAD_ID.fullmatch(uid):
            raise self._err(400, 'bad upload id')
        if not isinstance(off, int) or isinstance(off, bool) or off < 0:
            raise self._err(400, 'bad offset')
        try:
            data = base64.b64decode(body.get('data') or '', validate=True)
        except (ValueError, TypeError):
            raise self._err(400, 'bad data')
        d = self.root / '.uploads'
        d.mkdir(parents=True, exist_ok=True)
        part = d / f'{uid}.part'
        if off == 0:
            now = time.time()
            for old in d.glob('*.part'):
                try:
                    if old.stat().st_mtime < now - UPLOAD_KEEP:
                        old.unlink()
                except OSError:
                    pass
            part.write_bytes(data)
        else:
            if not part.is_file() or part.stat().st_size != off:
                raise self._err(400, 'The upload lost its place, so it stopped. Publish again.')
            with open(part, 'ab') as f:
                f.write(data)
        if part.stat().st_size > BUNDLE_MAX:
            part.unlink(missing_ok=True)
            raise self._err(413, f'A tool can be up to {BUNDLE_MAX // (1024 * 1024)} MB packed.')
        done = body.get('done')
        if done is None:
            return {'ok': True, 'have': part.stat().st_size}
        blob = part.read_bytes()
        part.unlink(missing_ok=True)
        if not isinstance(done, dict):
            raise self._err(400, 'done must say which tool this is')
        tool = self.tool_name(done.get('tool'))
        sha = done.get('sha256')
        if not isinstance(sha, str) or not HEX64_RE.fullmatch(sha) or hashlib.sha256(blob).hexdigest() != sha:
            raise self._err(400, 'The tool arrived damaged (sha256 differs). Publish again.')
        by = done.get('machine') if isinstance(done.get('machine'), str) and done['machine'] in self.app.fleet.machines else 'chat'
        desc = done.get('description') if isinstance(done.get('description'), str) else None
        note = done.get('note') if isinstance(done.get('note'), str) else ''
        when = done.get('when') if isinstance(done.get('when'), str) else None
        return self.store(blob, tool, by, note, desc, when)

    def connect(self, body):
        tool, name = self.known(body.get('tool')), self.app._machine(body)
        on = body.get('on')
        if not isinstance(on, bool):
            raise self._err(400, 'on must be true or false')
        if on:
            self._ready(name)
            v = self.push(name, tool)
            self._connect_record(name, tool)
            self.app.bump()
            return {'ok': True, 'tool': tool, 'machine': name, 'connected': True, 'installed': v}
        with self.lock:
            ts = self.d['machines'].get(name) or []
            if tool in ts:
                ts.remove(tool)
                self._save()
            self.errors.pop((name, tool), None)
        st = self.app.fleet.state.get(name) or {}
        if st.get('online') and self.has_feature(name):  # offline: dropped when it is back (reconcile)
            self._drop(name, tool)
        self.app.bump()
        return {'ok': True, 'tool': tool, 'machine': name, 'connected': False}

    def update(self, body):
        """POST /api/library/update {tool, auto?, when?, description?}: a tool's own settings, no new version."""
        tool = self.known(body.get('tool'))
        extra = set(body) - {'tool', 'auto', 'when', 'description'}
        if extra:
            raise self._err(400, f'Unknown field: {sorted(extra)[0]}.')
        if len(body) == 1:
            raise self._err(400, 'Nothing to change.')
        if 'auto' in body and not isinstance(body['auto'], bool):
            raise self._err(400, 'auto must be true or false')
        for k, cap in (('when', WHEN_MAX), ('description', 500)):
            if k in body and (not isinstance(body[k], str) or len(body[k]) > cap or any(ord(c) < 32 for c in body[k])):
                raise self._err(400, f'{k} is one line of text, up to {cap} characters.')
        with self.lock:
            t = self.d['tools'][tool]
            if 'auto' in body:
                t['auto'] = body['auto']
            if 'when' in body:
                t['when'] = ' '.join(body['when'].split())
            if 'description' in body and body['description'].strip():
                t['description'] = ' '.join(body['description'].split())
            self._save()
            out = {'ok': True, 'tool': tool, 'auto': t.get('auto') is not False, 'when': t.get('when') or '',
                   'description': t.get('description') or ''}
        self.app.bump()
        return out

    def defaults(self, body):
        tool, on = self.known(body.get('tool')), body.get('on')
        if not isinstance(on, bool):
            raise self._err(400, 'on must be true or false')
        with self.lock:
            ds = self.d['defaults']
            if on and tool not in ds:
                ds.append(tool)
            elif not on and tool in ds:
                ds.remove(tool)
            self._save()
            out = list(ds)
        self.app.bump()
        return {'ok': True, 'defaults': out}

    def rollback(self, body):
        tool, v = self.known(body.get('tool')), body.get('version')
        with self.lock:
            t = self.d['tools'][tool]
            if not isinstance(v, int) or isinstance(v, bool) or not any(x.get('v') == v for x in t['versions']) \
                    or not self._bundle(tool, v).is_file():
                raise self._err(400, f'{tool} has no version {v}.')
            t['latest'] = v
            self._save()
        log(f'library: {tool} rolled back to v{v}')
        self._push_all(tool)
        self.app.bump()
        return {'ok': True, 'tool': tool, 'latest': v}

    def delete(self, body):
        tool = self.known(body.get('tool'))
        with self.lock:
            had = self._connected(tool)
            del self.d['tools'][tool]
            if tool in self.d['defaults']:
                self.d['defaults'].remove(tool)
            for ts in list(self.d['machines'].values()) + list(self.d['chats'].values()):
                if tool in ts:
                    ts.remove(tool)
            for k in [k for k, e in self.errors.items() if k[1] == tool]:
                del self.errors[k]
            self._save()
        shutil.rmtree(self.root / tool, ignore_errors=True)
        for m in had:
            if (self.app.fleet.state.get(m) or {}).get('online') and self.has_feature(m):
                self.background(lambda m=m: self._try(self._drop, m, tool))
        log(f'library: {tool} deleted')
        self.app.bump()
        return {'ok': True, 'tool': tool}

    def attach(self, body):
        """A chat's tool list. Nothing restarts here: the client relaunches the chat after warning about context."""
        name, sid = self.app._machine(body), body.get('sid')
        if not isinstance(sid, str) or not SID_RE.fullmatch(sid):
            raise self._err(400, 'bad session id')
        tools = self.check_tools(body.get('tools'))
        if tools and not self.has_feature(name):
            raise self._err(409, f'Update Switchboard on {self._label(name)} to use tools from the library.')
        with self.lock:
            self.d['chats'].pop(ck(name, sid), None)
            self.d['chats'][ck(name, sid)] = tools
            self.d['modes'].pop(ck(name, sid), None)  # picked by hand: no longer auto
            self._save()
        self.app.bump()
        return {'ok': True, 'machine': name, 'sid': sid, 'tools': tools}

    def new(self, body):
        """A chat that builds a tool in <Claude folder>/library-src/<tool> and publishes it with publish_tool."""
        name, tool, brief = self._ready(self.app._machine(body)), self.tool_name(body.get('tool')), body.get('brief')
        if not isinstance(brief, str) or not brief.strip():
            raise self._err(400, 'Say what the tool should do.')
        if len(brief) > BRIEF_MAX:
            raise self._err(400, f'The description is too long (at most {BRIEF_MAX} characters).')
        folder = f'library-src/{tool}'
        req = {'verb': 'start', 'folder': folder, 'prompt': NEW_TOOL_PROMPT.format(tool=tool, brief=brief.strip()),
               'mode': 'normal', 'name': f'New tool {tool}'}
        return self.app._submit('start', name, {'req': req, 'mkdir': folder}, f'New tool {tool}')

    def get(self, path, query=None):
        if path == '/api/library':
            return self.view()
        if path == '/api/library/sources':
            return self.sources(query)
        return None

    def post(self, path, body):
        routes = {'/api/library/import': self.import_, '/api/library/publish': self.publish, '/api/library/connect': self.connect,
                  '/api/library/defaults': self.defaults, '/api/library/rollback': self.rollback, '/api/library/delete': self.delete,
                  '/api/library/attach': self.attach, '/api/library/new': self.new, '/api/library/update': self.update}
        fn = routes.get(path)
        return fn(body) if fn else None
