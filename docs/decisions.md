# Decisions

One line per judgment call, newest last. Older history lives with the private working copy.

- 2026-10-16: The public repository starts with a fresh history; the private working copy is never pushed to it.
- 2026-10-16: Every export to the public repository passes `scripts/public/leakscan.py` with a private term list kept outside the repo, and CI runs the scan on files and history.
- 2026-10-16: License is MIT, held by "Switchboard contributors" so no personal name is in the file.
- 2026-10-16: Bundle ids keep their existing prefix so installed apps, caches and keychain items carry over; it lives only in `branding.toml`.
- 2026-10-16: User-facing text calls `agent/sb.py` "the helper" and the server "the server", introduced as "one computer that stays on"; code names stay as they are.
- 2026-10-16: macOS and Windows CI jobs run only by hand while the repository is private, to save runner minutes, and on every push once it is public.
- 2026-10-16: Example machines in tests and docs are Studio, Laptop, Tower, NAS, Lab and Office; example addresses are 192.0.2.x, with 10.0.0.x and 100.64.0.x only where a test needs a LAN or tailnet shape.
