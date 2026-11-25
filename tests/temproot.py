"""One temp folder for a whole test run, removed when the run ends.

The tests make a few thousand temp folders a run and most never remove theirs. On a computer where the suite
runs often they pile up in the system's temp folder: one held 109,000 of them after a week, and everything that
lists that folder slowed down, Switchboard's own agent included. So every temp folder a run makes goes inside
one folder of the run's own, and that one is removed at exit. Programs the tests start get it too.
"""
import atexit
import os
import shutil
import tempfile

KEY = 'SB_TESTS_TMP'


def use():
    root = os.environ.get(KEY)
    if root and os.path.isdir(root):  # a program a test started, or this module imported under a second name
        tempfile.tempdir = root
        return root
    root = os.path.realpath(tempfile.mkdtemp(prefix='sbt-'))
    for k in (KEY, 'TMPDIR', 'TEMP', 'TMP'):
        os.environ[k] = root
    tempfile.tempdir = root
    pid = os.getpid()

    def remove():
        if os.getpid() == pid:  # not a forked child on its way out
            shutil.rmtree(root, ignore_errors=True)

    atexit.register(remove)
    return root


use()
