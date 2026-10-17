"""Uploads, end to end: a file dropped or pasted into a terminal goes to POST /api/upload, the server sends it to the
chat's machine in chunks, and the machine's agent writes it under its uploads folder and answers with the path. The
lab's machines run the real agent code for this (sb.rpc_upload), each with its own home in the control folder."""
import base64, datetime, hashlib, http.client, json, os
from pathlib import Path

from tests.e2e.lab import LabCase, lab_test


@lab_test
class Upload(LabCase):
    def uploads(self, machine):
        return self.lab.ctrl / 'home' / machine / 'uploads'

    def check_landed(self, r, data, machine='studio'):
        p = Path(r['path'])
        day = datetime.date.today().strftime('%Y-%m-%d')
        self.assertEqual(p.parent, self.uploads(machine) / day)
        self.assertEqual(p.read_bytes(), data)
        self.assertEqual((r['size'], r['name'], r['machine']), (len(data), p.name, machine))
        return p

    def test_a_file_lands_on_the_chats_machine(self):
        data = os.urandom(50_000)
        code, r = self.lab.upload('studio', 'Screen Shot 2026-10-13.png', data)
        self.assertEqual(code, 200, r)
        p = self.check_landed(r, data)
        self.assertEqual(p.name, 'Screen-Shot-2026-10-13.png')  # never needs quotes when typed into the chat
        self.assertEqual(list(self.uploads('studio').glob('.part-*')), [])

    def test_the_same_name_twice_keeps_both(self):
        code, a = self.lab.upload('laptop', 'notes.txt', b'one')
        code, b = self.lab.upload('laptop', 'notes.txt', b'two')
        self.assertEqual((Path(a['path']).name, Path(b['path']).name), ('notes.txt', 'notes-2.txt'))
        self.assertEqual(Path(a['path']).read_bytes(), b'one')
        self.assertEqual(Path(b['path']).read_bytes(), b'two')

    def test_a_name_can_never_leave_the_uploads_folder(self):
        for name in ('../../../.ssh/authorized_keys', '..\\..\\evil.bat', '/etc/passwd', 'CON.txt', '.bashrc', ''):
            code, r = self.lab.upload('tower', name, b'x')
            self.assertEqual(code, 200, (name, r))
            p = Path(r['path']).resolve()
            self.assertIn(self.uploads('tower').resolve(), p.parents, (name, p))
            self.assertNotIn('..', p.name)
            self.assertFalse(p.name.upper().startswith('CON.'), p.name)

    def test_a_big_file_goes_in_chunks(self):
        data = os.urandom(7 * 1024 * 1024)
        before = len(self.lab.rpcs('upload', 'studio'))
        code, r = self.lab.upload('studio', 'build.zip', data)
        self.assertEqual(code, 200, r)
        self.check_landed(r, data)
        chunks = self.lab.rpcs('upload', 'studio')[before:]
        self.assertEqual([(c['req']['offset'], c['req']['last']) for c in chunks], [(0, False), (6 * 1024 * 1024, True)])
        self.assertEqual(len({c['req']['id'] for c in chunks}), 1)

    def test_the_iphone_form(self):
        data = b'%PDF-1.7 fake'
        body = {'machine': 'studio', 'sid': 'whatever', 'name': 'résumé final.pdf', 'data': base64.b64encode(data).decode()}
        code, r = self.lab.post('/api/upload', body, ua='SwitchboardApp/3.0')
        self.assertEqual(code, 200, r)
        self.assertEqual(Path(r['path']).name, 'resume-final.pdf')
        self.assertEqual(Path(r['path']).read_bytes(), data)
        code, r = self.lab.post('/api/upload', dict(body, data='not base64!'), ua='SwitchboardApp/3.0')
        self.assertEqual((code, r['error']), (400, 'bad file'))
        code, r = self.lab.post('/api/upload', dict(body, data=base64.b64encode(os.urandom(800_000)).decode()), ua='SwitchboardApp/3.0')
        self.assertEqual((code, r['error']), (413, 'That file is too big to send from the phone.'))

    def test_refusals_say_why(self):
        code, r = self.lab.request('POST', '/api/upload?machine=studio&name=a.txt', b'x', {'Content-Type': 'application/octet-stream'})
        self.assertEqual(code, 403, r)
        code, r = self.lab.upload('studio', 'a.txt', b'')
        self.assertEqual((code, r['error']), (400, 'That file is empty.'))
        code, r = self.lab.upload('nope', 'a.txt', b'x')
        self.assertEqual((code, r['error']), (400, 'unknown machine'))

    def test_too_big_is_refused_with_its_size(self):
        """The server answers 413 from Content-Length alone. The app should get that answer, not a reset connection."""
        n = 20 * 1024 * 1024 + 1
        c = http.client.HTTPConnection('127.0.0.1', self.lab.port, timeout=60)
        c.putrequest('POST', '/api/upload?machine=studio&name=big.bin', skip_host=True)
        for k, v in (('Host', f'127.0.0.1:{self.lab.port}'), ('X-Switchboard', '1'), ('Content-Type', 'application/octet-stream'),
                     ('Content-Length', str(n))):
            c.putheader(k, v)
        c.endheaders()
        try:
            chunk = b'\0' * (1024 * 1024)
            for _ in range(20):
                c.send(chunk)
            c.send(b'\0')
        except OSError:
            pass  # the server may stop reading once it has answered
        resp = c.getresponse()
        r = json.loads(resp.read())
        c.close()
        self.assertEqual(resp.status, 413)
        self.assertEqual(r['error'], 'That file is 20 MB. Files up to 20 MB can be attached.')

    def test_offline_and_old_machines(self):
        self.lab.flag('off-tower')
        try:
            self.lab.wait(lambda: self.lab.machine('tower')['online'] is False, 15, what='the tower to drop off')
            code, r = self.lab.upload('tower', 'a.txt', b'x')
            self.assertEqual(code, 409)
            self.assertEqual(r['error'], "Tower is offline, so the file can't go there. Try again once Tower is back.")
        finally:
            self.lab.flag('off-tower', False)
        self.lab.flag('old-agent')
        try:
            self.lab.wait(lambda: 'upload' not in (self.lab.machine('studio').get('features') or []), 15, what='the old agent')
            code, r = self.lab.upload('studio', 'a.txt', b'x')
            self.assertEqual((code, r['error']), (409, 'Update Switchboard on Studio to attach files to its chats.'))
        finally:
            self.lab.flag('old-agent', False)
        self.lab.wait(lambda: self.lab.machine('tower')['online'] and 'upload' in self.lab.machine('studio')['features'], 15,
                      what='the machines back')
