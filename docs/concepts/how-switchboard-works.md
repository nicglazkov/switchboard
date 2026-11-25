---
description: The parts of Switchboard and how they talk. The home computer, the helper on every computer, where your chats live, and what leaves your devices.
---

# How Switchboard works

<p class="sb-meta">Last validated: Nov 23, 2026</p>

Switchboard is a few small programs that run on your own computers. One computer, known as the home computer,
keeps the board. A helper on every computer reports that computer's chats to it. The apps show the board and send
back what you type.

## The parts

<figure class="sb-fig" markdown="0">
--8<-- "docs/diagrams/how-whole.svg"
<figcaption>Figure 1. Every computer and every app connects to the home computer, not to each other.</figcaption>
</figure>

There are three kinds of part:

- **The home computer**: the one of your computers that runs Switchboard itself. It keeps the board, your
  settings, the history and the library. Pick a computer that stays on.
- **The helper**: a small part of Switchboard on every computer where you use Claude Code. It reports that
  computer's chats, starts and stops them, and keeps a connection open to the home computer.
- **The apps**: the desktop app and the phone apps. They show the board and send your answers.

The home computer is an ordinary computer of yours. It can run chats too, and it has a helper of its own.

## Where your chats live

A chat lives on the computer it runs on. Claude Code keeps the chat's text there, in its own files, and the chat
works with that computer's folders and tools.

The home computer keeps what the board needs to show: each chat's name, its state, and what a reply would cost.
When you open a chat's terminal from another computer, you look at the terminal on the chat's own computer. The
chat doesn't move.

Think of a hotel's front desk. The desk knows which room every guest is in and passes messages along, but the
guests and their luggage stay in their rooms.

## What happens when you answer a chat

<figure class="sb-fig" markdown="0">
--8<-- "docs/diagrams/how-request.svg"
<figcaption>Figure 2. Your answer travels out along the top. The chat's new state travels back along the
bottom.</figcaption>
</figure>

1. You type an answer in the app. The app sends it to the home computer.
2. The home computer knows which computer the chat runs on. It passes the answer to the helper there.
3. The helper types the answer into the chat's own terminal, the same as if you had typed it at that computer.
4. Claude carries on. The helper notices that the chat is working again.
5. The helper reports the new state to the home computer.
6. The home computer updates the board, and the app shows the chat in its new column.

Every other action takes the same path: starting a chat, stopping one, opening a folder.

## How the parts find each other

Today, the parts connect by address. The apps and the helpers reach the home computer at its address on your
home network or on Tailscale, a private network between your own devices. The home computer turns away
a request for its board that comes from any other address.

With device keys, the parts connect by key. Each computer has a key of its own, and the home computer keeps a
list of the keys it accepts. The helpers then connect to each other directly where they can, and through a relay
where they can't, with everything encrypted between the two computers. A phone pairs the same way, and the app
does the helper's part.

!!! note ""
    Device keys arrived with Switchboard 0.9.0 and are off until you turn them on. Until then,
    Switchboard connects by address, as described in [Add your computers](../machines.md).

With keys on, a program on any of your computers talks only to the helper on that same computer. The helper
carries the traffic to the home computer, so the app and Claude Code's hooks need no address at all.

For more information, refer to [Device keys and pairing](device-keys-and-pairing.md) and
[Connections](connections.md).

## What leaves your devices

Switchboard has no account and no service of its own in the cloud. Your chats, your files and your history stay
on your computers.

A few things do leave, and each is yours to turn off or to host yourself:

- **Checks for updates**: the home computer asks GitHub for the newest release.
- **Phone notifications**: these pass through Apple's push service once you set them up.
- **A relay**: with device keys, two computers that can't reach each other meet through a relay, which can't
  read what it passes.

Your chats themselves go to Claude, the same as they do without Switchboard. Switchboard doesn't change how
Claude Code talks to Claude.

For more information, refer to [Privacy and security](../privacy.md).
