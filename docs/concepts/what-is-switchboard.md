---
description: Switchboard puts the Claude Code chats from all your computers on one board, tells you which need you, and lets you answer, open and start them from wherever you are.
---

# What is Switchboard?

<p class="sb-meta">Last validated: Nov 22, 2026</p>

Switchboard is one board for the Claude Code chats on all your computers. It shows every chat, sorted by what it
needs from you, and lets you answer a question, open a terminal or start a new chat on any of them.

Claude Code runs in a terminal, on one computer. That works well for one chat. It works less well once you have
a few running on a desktop, a laptop and a computer in the closet, because the chat that is waiting for you is
rarely on the computer in front of you.

Switchboard doesn't replace Claude Code or change how your chats run. It reads what Claude Code already reports
on each computer and gathers it in one place.

## Technical details

Without Switchboard, each chat is visible only in its own terminal. To learn that a chat has asked a question,
you have to be looking at that terminal on that computer.

<figure class="sb-fig" markdown="0">
--8<-- "docs/diagrams/without-switchboard.svg"
<figcaption>Figure 1(a). Each computer knows only about its own chats. A question asked on a computer you are not
sitting at waits until you happen to check.</figcaption>
</figure>

With Switchboard, a small helper on each computer reports the state of its chats to one of your computers, known
as the home computer. The home computer keeps the board. Any of your devices can open it.

<figure class="sb-fig" markdown="0">
--8<-- "docs/diagrams/with-switchboard.svg"
<figcaption>Figure 1(b). Every computer reports to one board. The chat that needs you comes first, whichever
computer it runs on.</figcaption>
</figure>

The chats themselves stay where they are. A chat on the closet PC still runs on the closet PC, with that
computer's files and tools. The board carries its state and your answers, not the work.

For more information, refer to [How the pieces fit](../architecture.md).

## Benefits

### You always know what needs you

The board sorts chats into columns: <span class="sb-pill need">Needs you</span>,
<span class="sb-pill run">Working</span>, <span class="sb-pill warm">Warm now</span> and
<span class="sb-pill cold">Gone cold</span>. A chat that asks a question moves to **Needs you** within seconds,
and you can answer it from its card.

For more information, refer to [The board](../board.md).

### You know what a chat costs before you go back to it

Claude keeps a recent chat ready for a few minutes. After that the chat goes cold, and the next message makes
Claude read the whole chat again, which uses more of your plan. Switchboard shows how long each chat stays warm
and what a reply would cost once it is cold. For a large cold chat it offers **Start fresh**, which continues
the work in a new chat from a summary.

For more information, refer to [Chats and what they cost](../chats.md).

### Your computers work as one

- **Start a chat anywhere**: pick the computer and the folder, or let Switchboard pick the computer.
- **Open any terminal**: a chat's own terminal opens in a tab, even when its computer is in another room.
- **Move a chat**: continue a chat on a different computer when you change desks.
- **Find old work**: search the history and browse the folders of every computer.

### It stays yours

Switchboard runs on your own computers. There is no account to create, and your chats and files are not stored
anywhere else.

For more information, refer to [Privacy and security](../privacy.md).

## Use cases

- **Several computers**: a desktop for heavy work, a laptop on the couch and a small computer that stays on.
  One board covers all three.
- **Many chats on one computer**: even with a single computer, the board sorts your chats and warns you before
  an expensive reply.
- **Long tasks**: start a task, walk away, and answer its questions from another room.
- **A tight plan**: watch the 5-hour and weekly limits, set a daily budget and see which chats use the most.

## Get started

To set up Switchboard, refer to [Install Switchboard](../install.md). It takes about ten minutes.
