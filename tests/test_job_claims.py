"""A queued job always gets an answer.

The launcher takes a job by renaming it to <id>.claimed. A job file that could not be read, one with no id in it, or
a launcher that died right after the rename left no result behind, and the server waited out its whole timeout."""
import json, os, unittest
from tests import test_jobs
from tests.test_rpc import NOW


class JobClaimTest(test_jobs.JobBase):
    def q(self):
        return self.sb.paths_q()['queue']

    def result(self, jid):
        return self.sb._jget(self.q() / f'{jid}.result.json')

    def claims(self):
        return [n for n in os.listdir(self.q()) if n.endswith('.claimed')]

    def test_a_job_file_that_cannot_be_read_gets_an_error_not_silence(self):
        (self.q() / 'abc123.json').write_bytes(b'\x00' * 40)
        good = self.job({'verb': 'start', 'folder': 'proj', 'prompt': 'hello'})
        self.drain()
        r = self.result('abc123')
        self.assertEqual((r['state'], 'finished' in r), ('error', True))
        self.assertIn('could not be read', r['error'])
        self.assertEqual(self.result(good)['state'], 'done', 'and the drain goes on')

    def test_a_job_with_no_id_inside_does_not_stop_the_drain(self):
        for name, body in (('noid1', {'verb': 'start', 'created': NOW}), ('alist', [1, 2])):
            (self.q() / f'{name}.json').write_text(json.dumps(body), encoding='utf-8')
        good = self.job({'verb': 'start', 'folder': 'proj', 'prompt': 'hello'})
        self.drain()
        self.assertEqual(self.result(good)['state'], 'done')
        self.assertEqual(self.result('alist')['state'], 'error')
        self.assertIsNotNone(self.result('noid1'), 'answered under the name of its file')

    def test_a_job_claimed_by_a_launcher_that_died_is_answered_by_the_next_one(self):
        jid = self.job({'verb': 'start', 'folder': 'proj', 'prompt': 'hello'})
        os.replace(self.q() / f'{jid}.json', self.q() / f'{jid}.claimed')  # renamed, and then the launcher died
        self.drain()
        r = self.result(jid)
        self.assertEqual(r['state'], 'error')
        self.assertIn('Try again', r['error'])
        self.assertEqual(self.claims(), [])
        self.assertEqual(self.launched, [], 'never run late')

    def test_a_claim_whose_job_did_get_its_result_is_only_tidied(self):
        jid = self.job({'verb': 'start', 'folder': 'proj', 'prompt': 'hello'})
        os.replace(self.q() / f'{jid}.json', self.q() / f'{jid}.claimed')
        self.sb._jput(self.q() / f'{jid}.result.json', {'state': 'done', 'finished': NOW})
        self.drain()
        self.assertEqual(self.result(jid), {'state': 'done', 'finished': NOW})
        self.assertEqual(self.claims(), [])

    def test_the_claim_is_dropped_once_the_job_is_running(self):
        jid = self.job({'verb': 'start', 'folder': 'proj', 'prompt': 'hello'})
        self.drain()
        self.assertEqual(self.result(jid)['state'], 'done')
        self.assertEqual(self.claims(), [])


if __name__ == '__main__':
    unittest.main()
