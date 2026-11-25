# Security

## What Switchboard protects and what it doesn't

Switchboard has no passwords. Anyone who can reach its ports (8130 to 8132) can read your chats, type into their
terminals and start new chats on your computers. That is on purpose: it is meant for your own home network or your
own Tailscale network. Never forward its ports to the internet.

The server's API (port 8130) answers only requests that come from this computer, a private network (10.x,
172.16 to 172.31, 192.168.x), a link-local address, Tailscale (100.64.0.0/10) or the IPv6 kinds of those. It goes
by the address a request comes from, not by the `Host` it sends. `peers` in `server.json` lets more networks in.
This narrows who can reach it. It is not a password: everyone on a network that is let in can use the server.

Linking a computer uses a one-time code that expires after 10 minutes. A computer added over ssh gets a key that
can only run the Switchboard helper, not a shell.

## Reporting a problem

<!-- --8<-- [start:report] -->
Please do not open a public issue for a security problem. Use GitHub's private report instead: the **Security**
tab of [the repository](https://github.com/nicglazkov/switchboard/security), then **Report a
vulnerability**. Include what you found, how to reproduce it, and which version you ran (About in the app shows
it, and Copy version info copies it).

You should hear back within a week. Fixes ship in the next release, and the release notes credit you unless you
would rather they didn't. Only the latest release gets security fixes.
<!-- --8<-- [end:report] -->

## Supported versions

Only the latest release gets security fixes.
