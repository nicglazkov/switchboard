---
description: A device key is a computer's own secret, and the home computer accepts only the keys on its paired list. How a code, a link or a QR code adds a computer, and what removing one does.
---

# Device keys and pairing

<p class="sb-meta">Last validated: Nov 24, 2026</p>

!!! note ""
    Device keys arrived with Switchboard 0.9.0 and are off until you turn them on.

A device key is a secret that one computer makes for itself and never shares. The home computer, the one of your
computers that runs Switchboard itself, keeps a list of the keys it accepts. Pairing is how a computer gets onto
that list.

## The device key

Each computer makes its key the first time it needs one. The key is 32 random bytes in a file that only your
account can read:

```text
~/.config/switchboard/device.key
```

The key has two halves. The secret half stays in that file. The public half is the computer's id, a long
number that is safe to show. Switchboard prints the first eight characters of it, such as `07ef0adb`, wherever it
names a device.

A computer proves who it is by using the secret half, never by sending it. Nothing that crosses the network lets
someone else pretend to be that computer.

## The paired list

The home computer keeps the ids of the devices it has paired with in one file:

```text
~/.local/share/switchboard/paired.json
```

Think of it as the guest list at a door. A device whose key is on the list gets in. Any other device is turned
away, however it reached the door and whatever it says about itself.

The home computer checks the list every time: when a connection arrives, and again for each thing sent over it.
It never remembers an earlier yes.

## Pairing

To pair a computer, you carry a secret from the home computer to the new one. It comes in three forms:

- **A short code**: eight letters and digits, shown as `K7PM-2XQ9`. You type it on the other computer.
- **A link**: one line that starts with `switchboard://pair/`. You paste it.
- **A QR code**: the same link as a picture, for a phone to scan.

<figure class="sb-fig" markdown="0">
--8<-- "docs/diagrams/pairing-flow.svg"
<figcaption>Figure 1. The code itself never crosses the network. Each side only proves to the other that it
holds it.</figcaption>
</figure>

1. The home computer makes a code and a link. You carry one of them to the new computer.
2. The new computer contacts the home computer and proves that it holds the code, without sending the code.
3. The home computer proves the same in return, adds the new computer's id to the paired list, and tells it
   the name it has on the board.

Both sides learn each other's id in that exchange, and it is tied to the two ids and to the connection it runs
over. Someone who sits between the two computers ends up with nothing that works, exactly as if they had
guessed the code wrong.

## One use and ten minutes

A code and its link work once. They stop working ten minutes after they are made, used or not.

A wrong code counts as a failed try, and five failed tries end that code. This is because a short code has to
stay unguessable for the whole ten minutes. A lost connection doesn't count as a try.

A short code leaves out `0`, `O`, `1`, `I` and `L`, so you can't misread one for another. Typing it in lower
case, or with spaces in place of the dash, works too.

## Code or link

A link names the home computer by its id, so the new computer knows exactly who to contact. A short code is
too short to hold an id. The new computer finds the home computer through a relay, a small server that both
sides can reach, at a meeting point worked out from the code.

So a short code needs a relay, and a link doesn't. For more information, refer to [Connections](connections.md).

## Computers you already added

A computer that was on your board before keys doesn't need a code. When you turn keys on, the home computer asks
each connected computer's helper to make a key, and adds it to the paired list by itself.

That request travels over the connection the computer already has, so it is trusted exactly as much as that
connection was. A computer whose helper is too old to answer is listed as needing an update first.

## Phones

A phone has a device key too, and pairs with the same code, link or QR code. It has no helper, so the app makes
the key and keeps it where the phone keeps secrets: the Keychain on an iPhone, and a file sealed by the Android
Keystore on an Android phone. The key stays on that one phone. A backup restored to another phone doesn't carry
it, so the other phone pairs again.

The phone apps that pair are not released yet. Refer to [Pair a phone](../features/pair-a-phone.md).

## Removing a device

Removing a device takes its id off the paired list. The home computer closes that device's connections in the
same moment and refuses its key from then on. Nothing has to restart.

On the removed computer, the helper's status then says that the home computer no longer lists it. To bring the
computer back, pair it again with a new code.

On a removed phone, the app says "Can't reach home: it no longer lists this phone."

To pair or remove a computer, refer to [Pair a computer](../features/pair-a-computer.md).

## A signed copy of the list

Each time the paired list changes, the home computer signs it with its own device key, and every paired device
keeps a copy. A device can check the signature, and nobody but the home computer can make one.

This is what lets your other computers answer for their own chats when the home computer is off. Each one
answers only the devices on its copy, and only while the copy is less than 7 days old. Refer to
[When the home computer is away](../features/when-home-is-away.md).

## The home computer's key

The home computer's device key is what every device is paired with, so the home computer is whichever computer
holds that key. It is the one key that can leave its computer, and only when you move the home computer or back
it up. Refer to [Move or replace the home computer](../features/move-or-replace-home.md).
