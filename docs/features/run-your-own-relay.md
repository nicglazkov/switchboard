---
description: Run your own relay for Switchboard on a small Linux server. What to rent, which ports to open, what it costs, how to check it and how to point Switchboard at it.
---

# Run your own relay

<p class="sb-meta">Last validated: Nov 22, 2026</p>

!!! note ""
    Device keys arrived with Switchboard 0.9.0 and are off until you turn them on. A relay is only
    used with device keys.

A relay lets two of your computers connect when they can't reach each other directly, for example a laptop on a
cafe network and a home computer behind a home router. Running your own means the only server between your
devices is one that you control.

## Why it matters

The relay passes traffic that is encrypted between your devices, so it can't read any of it. It does see which
devices talk to each other, from which addresses, and how much. On your own relay, nobody else sees even that.

## Benefits

- **Yours alone to trust**: who connects to whom stays on a server you rent.
- **Small and cheap**: the relay copies bytes and does no heavy work. The smallest server plan is enough.
- **One command**: a script installs the relay, gets its certificate and starts it.
- **Unchanged software**: the relay is the open source `iroh-relay`, version 1.5.0, with nothing added.

## Use cases

- **A household**: a few computers in two or three places that should always find each other.
- **Short codes**: pairing with a code such as `K7PM-2XQ9` needs a relay for the two computers to meet at.
- **Phones**: a phone finds the home computer through the relay named in its pairing link.
- **A private test**: a relay on a Tailscale address, with no certificate, to try keys out.

## How it works

Every helper connects outward to the relay and stays connected. When two computers have no direct path, the
relay passes their traffic to each other. It also helps them find a direct path, and most connections move to
one within a second. From then on they cost the relay nothing.

For more information, refer to [Connections](../concepts/connections.md).

## Set up a relay

To run your own relay:

1. Rent a small Linux server and give it a name.
2. Open three ports.
3. Run the install script.
4. Point Switchboard at the relay.

### Prerequisites

Before you can set up a relay, you must:

- Have a Linux server with systemd and a public IPv4 address, which you can log in to as root.
- Have a DNS name that you control, such as `relay.example.com`.
- Have a copy of Switchboard's source, which holds the script `scripts/deploy-relay.sh`.

### Rent a server

The smallest plan at most providers is enough:

| | What you need | Why |
|---|---|---|
| CPU | 1 shared vCPU | The relay only copies bytes. |
| Memory | 512 MB | It used 12 MB after relaying 1.6 GB in a test. |
| Disk | The smallest on offer | The program is 10 MB. It keeps a certificate and nothing else. |
| System | Any Linux with systemd, x86_64 or arm64 | Ubuntu and Debian are the easy choices. |
| Address | A public IPv4 address | Computers behind home routers must be able to reach it. |

Pick a location close to your devices. This is because relayed traffic goes there and back.

Then create a DNS `A` record for your name that points at the server, and wait until it resolves. The
certificate request fails otherwise, and Let's Encrypt limits how often you can try again.

### Open the ports

Open these on the server's firewall, and on your provider's firewall if it has one:

| Port | What it carries |
|---|---|
| TCP `443` | The relay itself |
| TCP `80` | The check Let's Encrypt makes for the certificate |
| UDP `7842` | Address discovery, which helps computers find a direct path |

With UDP `7842` closed the relay still works. Computers take longer to find a direct path, or stay relayed.

### Run the install script

From your own computer, in the folder that holds Switchboard's source, run:

```text
scripts/deploy-relay.sh root@relay.example.com install --hostname relay.example.com --contact you@example.com
```

Replace `relay.example.com` with your relay's name, both times, and `you@example.com` with your email address.
Let's Encrypt keeps that address for the certificate.

The script acts only on the one server you name. It:

1. Downloads the `iroh-relay` 1.5.0 release and checks it against a checksum written in the script.
2. Creates a user, `sb-relay`, that the relay runs as.
3. Writes the relay's settings to `/etc/switchboard-relay/relay.toml` and a systemd service named
   `switchboard-relay`.
4. Starts the service and waits until the relay answers.

When it is done, it prints the address to give Switchboard:

```text
Relay address: https://relay.example.com
```

You can run the same line again at any time. With the same options nothing changes.

### Point Switchboard at the relay

On the home computer, the one of your computers that runs Switchboard itself, run:

```text
switchboard-node relay https://relay.example.com
```

Replace `https://relay.example.com` with your relay's address. The helper answers **The relay is now
https://relay.example.com.**

A computer learns the home computer's relay when it pairs, from the code's exchange or from the link. A computer
that was already paired keeps the relay it learned then. If you change the relay later, run the same command on
each paired computer.

### Set the relay in the desktop app

The desktop app on the home computer sets the same relay.

1. Open **Settings** and select **Devices**.
2. Under **Relay**, select **Change**.
3. Type the relay's address, such as `https://relay.example.com`, and select **Save**.

The section shows the relay in use and where it came from, for example **Set on this computer** or **The one
Switchboard comes with**. It adds **Not connected** when the helper can't reach the relay. To use no relay of
your own, leave the field empty and select **Save**.

A relay set where Switchboard starts, with the `SWITCHBOARD_RELAY` environment variable, has no **Change**
button. This is because the variable wins, so the relay is changed there.

With no relay, the section says **None** and shows **This computer's address** with **Copy**. A computer that
pairs with a link then needs that address too.

## Check that it works

Ask the relay itself, from any computer:

```text
curl https://relay.example.com/healthz
```

A working relay answers with one line that says `"status":"ok"` and its version.

Then ask the helper on the home computer:

```text
switchboard-node status
```

Its relay line should say connected, and where the address came from:

```text
Relay: https://relay.example.com (connected; from settings)
```

To see how much the relay has carried, run the script with `status` in place of `install`:

```text
scripts/deploy-relay.sh root@relay.example.com status
```

It prints the state of the service and the relay's counters. `relayserver_bytes_recv_total` and
`relayserver_bytes_sent_total` are what your traffic bill follows. The script also takes `logs`, for the last
200 log lines.

## What it costs

- **The server**: the smallest plan at most providers, a few dollars a month.
- **A DNS name**: any name you control. The certificate from Let's Encrypt is free.
- **Traffic**: the one cost that can grow. Every relayed byte comes in once and goes out once, so it counts
  twice against a traffic allowance.

Terminal and chat traffic is small. Only traffic between computers that can't reach each other directly stays
on the relay, and large file uploads are what add up.

## Change the limits

The install writes limits that keep one device from using the whole relay. A device over its rate is slowed
down, not disconnected.

| Limit | Default | Option |
|---|---|---|
| What one device may send through the relay | 4 MiB a second | `--rx-bytes` |
| How far one burst may go above that | 8 MiB | `--rx-burst` |
| New connections accepted | 50 a second | `--conn-rate` |
| New connections in one burst | 100 | `--conn-burst` |

To change one, run the install line again with the option and a number. `--no-limits` leaves them all out.

## Which relay Switchboard uses

A helper takes the first of these that is set:

1. The `SWITCHBOARD_RELAY` environment variable.
2. The relay you set with `switchboard-node relay`, which is kept as `relay` in
   `~/.config/switchboard/node.json`.
3. On a paired computer, the home computer's relay, learned when it paired.
4. A relay built into the helper, when its build includes one.
5. None.

`switchboard-node status` names which one applies: **environment**, **settings**, **home**, **built in** or
**none**. To clear the one you set, run `switchboard-node relay none`.

## Remove the relay

To stop the relay and remove everything the install wrote, run:

```text
scripts/deploy-relay.sh root@relay.example.com uninstall
```

## Limitations

- **Linux with systemd only.** The script installs on nothing else. A Docker recipe is in
  `scripts/relay/docker-compose.yml`.
- **Anyone who knows the address can use the relay.** This is because a computer that isn't paired yet has to
  reach it too. The limits keep that affordable, and nobody who uses it can read or join your connections.
- **A Mac's helper can't reach a relay on a local network address.** macOS blocks it without asking. Give Macs
  the relay's public or Tailscale address.
- **A phone has no relay setting.** It uses the relay in the link it paired with, so set the relay on the home
  computer before you pair a phone. Refer to [Pair a phone](pair-a-phone.md).
- **A test relay has no certificate.** `install --dev --port 3340 --user` serves plain HTTP for a private or
  Tailscale address only. Its address is `http://` followed by the server's address and `:3340`.

## Troubleshooting

### The install says the relay did not answer

Run the script with `logs` to learn why. A certificate error usually means that the name doesn't point at this
server yet, or that TCP `80` is closed.

### The log says Address already in use

Another web server holds port `80` or `443`. The relay needs both, so stop the other server or use a different
host.

### Computers stay connected through the relay

Check that UDP `7842` is open on the server. Some networks allow no direct path at all, and a relayed connection
still works on those. It is slower, not broken.

### The helper says it cannot reach the relay

Refer to [Pair a computer](pair-a-computer.md#cannot-reach-the-relay).
