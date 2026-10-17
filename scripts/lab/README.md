# Lab tools

- `fakefleet.py <repo-copy> <live.json> <port> <host,host> <ctrl-dir>` runs the real `server.App` and web files with
  every machine faked, so actions never reach a real Claude session. `live.json` is a GET of
  your server's `/api/sessions` (keep it out of git: it holds real chat titles). Control files are listed
  at the top of the script.
- `drive.py` drives headless Chrome on a Linux machine (taps, typing, screenshots) through the DevTools protocol.
  `native/scripts/fixtures/fixture.json` works as `live.json` (the end-to-end tests use it). `LAB_POLL` sets how often it
  polls (default 5 s; the real server polls every 20 s) and `LAB_HUB=1` pairs machines through a real hub.
- `e2e.sh` runs the end-to-end tests in `tests/e2e`: each test class starts its own fake fleet on a free port and
  drives it over HTTP like the apps do (pairing with the real hub and node, Auto, dispatch, relaunch, uploads,
  Start fresh, live updates). It builds `switchboard-hub` and `switchboard-node` first, for the pairing test.
- `handoff-compare.py <transcript.jsonl>...` runs Opus and Sonnet on the same condensed transcripts and reports tokens, sections and directives kept word for word, for picking the summary model in Settings. Run it where Claude Code is signed in; the notes it writes hold chat content, so keep them out of git.
