---
description: With device keys, two computers connect directly when they can and through a relay when they can't. What the relay can see, what happens when a path dies, and the three modes.
---

# Connections

<p class="sb-meta">Last validated: Nov 22, 2026</p>

!!! note ""
    Device keys arrived with Switchboard 0.9.0 and are off until you turn them on.

A connection is the encrypted link between the helper on one of your computers and the helper on the home
computer, the one of your computers that runs Switchboard itself. It is either direct, straight between the two
computers, or relayed, through a small server that both can reach. A paired phone connects the same way, with
the app doing the helper's part.

## Direct and relayed

Each computer's helper connects to the home computer by its device key, not by an address. The helper tries to
find a path straight to the home computer. When there is none, the two meet at a relay.

<figure class="sb-fig" markdown="0">
--8<-- "docs/diagrams/connection-direct.svg"
<figcaption>Figure 1(a). Direct. The two helpers talk straight to each other, and the relay only helps them find
the way.</figcaption>
</figure>

<figure class="sb-fig" markdown="0">
--8<-- "docs/diagrams/connection-relay.svg"
<figcaption>Figure 1(b). Relayed. The same connection, with the same encryption, passes through the relay because
no direct path exists.</figcaption>
</figure>

The figures don't draw the encryption. In both, everything is encrypted from one helper to the other, with keys
that only those two computers hold.

A connection can change from one to the other while it is open. It often begins at the relay and moves to a
direct path a moment later.

## What Direct means

The helper's status says **connected directly** for any path that doesn't pass through the relay. The desktop
app and the phone apps say **Direct**. That covers more than one kind of network:

- **Your home network**: both computers are on the same router.
- **Tailscale**: a private network between your own devices. Switchboard counts a Tailscale path as direct.
- **Any address that reaches**: for example a home computer with a public address.

## What Through the relay means

The status says **connected through the relay**, and the apps say **Through the relay**, when the two devices
can't reach each other, for example a laptop in a cafe and a home computer behind a home router. A relayed connection is slower than a direct one. It
isn't broken, and nothing about what you can do changes.

Neither computer needs a port opened on its router for this. Both connect outward to the relay.

## What the relay can and can't see

The relay passes sealed traffic between two devices. What it can see:

- **Which device ids talk to each other**: the public halves of their keys.
- **Where from**: the network address each device connects from.
- **How much**: the amount of traffic, and when.

What it can't see is anything inside: your chats, your files, your terminal, the names of your computers or the
commands Switchboard sends. It also can't join in. The home computer accepts only the keys on its paired list,
and the relay holds none of them.

Switchboard uses one relay, the one it is given, and no other outside service to find your computers. You can
run that relay yourself. Refer to [Run your own relay](../features/run-your-own-relay.md).

## When a path dies

A path can die without a word: a laptop changes networks, or a router restarts. The helper notices this itself.
A request that gets no answer for 2 seconds, on a connection that has heard nothing at all for 10 seconds, closes
the connection, and the next request connects again.

The helper then keeps trying, first after 1 second and then at longer waits, up to 30 seconds apart. It also
remembers the address where it last reached the home computer directly, and tries that first the next time it
starts.

!!! note ""
    Without a relay, computers connect only where they can reach each other directly, on the same network or
    over Tailscale. The helper's status says so.

## The three modes

One setting on the home computer decides how your devices may connect. It is `keys` in
`~/.config/switchboard/server.json`, and it has three values:

| Value | What it means |
|---|---|
| `off` | By address only, as before keys. This is what you have until you change it. |
| `both` | By address and by key. Paired computers use their keys, and everything else keeps working. |
| `only` | By key only. The home computer answers nothing but itself and its paired devices. |

In `only`, the home computer stops listening on your network altogether. It no longer announces itself there,
and the downloads for adding a computer by address are not served. A value that Switchboard doesn't recognize
counts as `only`. This is because the careful reading of a typing mistake is the one that opens nothing.

To change the mode, run `switchboard keys on` for `both`, `switchboard keys only` or `switchboard keys off` on
the home computer. The command writes the setting and restarts what needs it. If you edit the file by hand, run
`switchboard restart` afterwards, because the home computer reads the setting when it starts.

The desktop app changes the mode too, in **Settings**, **Devices**: **Turn keys on** for `both`, and **Close the
network port** for `only`.

## Before you choose only

In `only`, everything that connects by address loses the board. That includes a browser on another device, a
phone that connects by address, and the iPhone widgets. Stay on `both` while you use any of them.

The home computer can tell you whether `only` would cut anything off. On the home computer, run:

```text
switchboard keys status
```

It lists each computer and says "Ready for keys only." when both of these hold:

- **Every other computer is paired** and has connected with its key since the home computer's helper started.
- **Nothing has connected by address for 7 days**, and the home computer has been watching for that long.

Otherwise it says "Not ready for keys only yet:" and lists each thing that stands in the way, in plain words,
such as a computer that isn't paired yet or the iPhone app that connected by address two hours ago. A computer
you removed is not waited for.

`switchboard keys only` refuses while something would be cut off, and changes nothing. `--force` switches anyway.

### Change the number of days

The 7 days are a setting, `keys_ready_days` in `~/.config/switchboard/server.json`. It takes a whole number from
1 to 90, and the home computer reads it without a restart.

```json
"keys_ready_days": 14
```

### Close the port from the desktop app

While the mode is `both`, **Settings**, **Devices** has a card named **Moving to keys**. It lists what still
stands in the way, in the same words as the command.

When nothing does, the card's title is **Ready to close the network port**. Select **Close the network port**,
then **Click again to close the port**. The home computer restarts its server, and from then on it answers paired
devices only. Paired devices are not interrupted.

A home computer whose server can't restart itself, such as a Windows one, shows the command to run in place of
the switch.
