# Security

## What Switchboard protects and what it doesn't

Switchboard has no passwords. Anyone who can reach its ports (8130 to 8132) can read your chats, type into their
terminals and start new chats on your computers. That is on purpose: it is meant for your own home network or your
own Tailscale network. Never forward its ports to the internet.

Linking a computer uses a one-time code that expires after 10 minutes. A computer added over ssh gets a key that
can only run the Switchboard helper, not a shell.

## Reporting a problem

Please do not open a public issue for a security problem. Use GitHub's private report instead: the **Security**
tab of this repository, then **Report a vulnerability**. Include what you found, how to reproduce it, and which
version you ran (Settings, About in the app, or `switchboard status`).

You should hear back within a week. Fixes ship in the next release, and the release notes credit you unless you
would rather they didn't.

## Supported versions

Only the latest release gets security fixes.
