# Contributing

Thanks for wanting to help. Bug reports, fixes and small features are all welcome. For anything large, open an
issue first so we can agree on the shape before you spend time on it.

## Layout

- `server/` and `agent/` are Python 3.9+, with no packages beyond the standard library.
- `native/` is a Rust workspace: `sb-core` (models, rules, the HTTP client; no UI), `sb-desktop` (the app, built
  with [GPUI](https://www.gpui.rs)), `sb-hub` (terminals and linked computers) and `sb-node`.
- [How the pieces fit](https://nicglazkov.github.io/switchboard/architecture/) (`docs/architecture.md`) explains how
  the pieces talk to each other.

## Running the tests

```sh
python3 -m unittest discover -s tests -t .
cd native && cargo test --workspace --locked
```

Before you send a change, also run:

```sh
cd native && cargo fmt --all --check && cargo clippy --workspace --locked -- -D warnings
python3 scripts/public/leakscan.py .
```

CI runs the same checks on macOS and Windows.

## Building the app

```sh
cd native && cargo build --release --locked -p sb-desktop
```

On Linux you need the usual GPUI system packages (`libxkbcommon-dev`, `libvulkan-dev`, `libwayland-dev`,
`libx11-xcb-dev`, `libfontconfig-dev`). Release builds, installers and signing are covered in
[Building and releasing](https://nicglazkov.github.io/switchboard/building/). Signing keys and update feed addresses come from your environment and are
never stored in the repository.

## Writing style

The app's text and the docs follow a few rules:

- Sentence case, plain verbs: "Open terminal", not "Open Terminal" or "Launch the terminal experience".
- No em dashes. Use a comma, a colon or a new sentence.
- Say what a thing does, in words a person would use out loud. Skip marketing words; `scripts/public/leakscan.py`
  flags the common ones in Markdown.
- A feature that is off or not set up says so and links to where to set it up. It never just fails.

## Personal details

Never commit host names, IP addresses, user names or home folder paths from your own setup. Use `192.0.2.x`
addresses, `example.com` and `/Users/you` in examples and tests. The leak scan in CI catches the usual cases, and
`--deny <file>` lets you add your own private terms from a file kept outside the repo.

## Commits and pull requests

- Keep each commit to one change, with a message that says what changed and why.
- Note any judgment call as one dated line in `docs/decisions.md`, and update `docs/architecture.md` in the same
  change when you change how the pieces fit together.
- By sending a pull request you agree that your change is released under the GNU AGPL-3.0, the same license as the rest of the project.
