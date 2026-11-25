import os

# The tests never ask GitHub for releases (server/releases.py); a test that wants them says so.
os.environ.setdefault('SWITCHBOARD_RELEASES', 'off')

from . import temproot  # noqa: E402,F401  every temp folder of the run in one place, removed at exit
