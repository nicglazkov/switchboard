import os

# The tests never ask GitHub for releases (server/releases.py); a test that wants them says so.
os.environ.setdefault('SWITCHBOARD_RELEASES', 'off')
