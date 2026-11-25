---
description: A chat is warm while Claude still holds a ready copy of it, and cold once that copy runs out. What the cache is, how long it lasts, and how Switchboard knows.
---

# Warm and cold

<p class="sb-meta">Last validated: Nov 22, 2026</p>

A chat is warm while Claude still holds a ready copy of it, known as the cache. A chat is cold once that copy
has run out. A message to a warm chat is cheap. A message to a cold chat makes Claude read most of the chat
again, which uses more of your plan.

## The cache

Claude doesn't remember a chat between messages. Each time you send one, Claude reads the whole chat up to that
point: your messages, its replies, and everything its tools returned.

To keep that affordable, Claude keeps a ready copy of what it has already read. Reading a chat from that copy
counts at about a tenth of the price of reading it new. That copy is the cache.

Think of a book left open on your desk. While it lies there, you pick up where you stopped. Once it is back on
the shelf, you have to find your place again from the first page.

## How long a chat stays warm

Claude Code's cache lasts 5 minutes or 1 hour after each reply. Claude Code decides which one a chat gets, and
Switchboard doesn't change it.

Every reply starts the time again. While Claude is working, each step it takes refreshes the cache, so a chat
that is working is always warm.

## What going cold means

When the time runs out, the copy is gone. Nothing in the chat is lost, and you can still go back to it. Only
the price of the next message changes.

<figure class="sb-fig" markdown="0">
--8<-- "docs/diagrams/warm-timeline.svg"
<figcaption>Figure 1. The same message costs about seventeen times more once the chat is cold. The numbers are
estimates for a chat of 180k tokens with a 1-hour cache.</figcaption>
</figure>

The next message to a cold chat reads almost all of it in full and writes it back to the cache. Writing to the
cache counts for more than plain reading: 1.25 times for a 5-minute cache and 2 times for a 1-hour cache.

One part stays cheap. Every chat begins with the same instructions and tools from Claude Code, about 29k tokens,
and some other chat usually keeps that part cached.

After that one message the chat is warm again, and replies are cheap until the time next runs out.

## Size decides how much it matters

Going cold costs little for a short chat and a lot for a long one. Switchboard sorts chats into three sizes:

| Size of the chat | Going back to it cold |
|---|---|
| Under 80k tokens | Cheap anyway. Switchboard suggests **Resume**. |
| 80k to 150k | Reads it all again. **Start fresh** is cheaper if you don't need the whole history. |
| 150k or more | Reads it all again at full price. The guard holds your message first. |

On the board, only a cold chat of 150k or more moves to the <span class="sb-pill cold">Gone cold</span> column.
A smaller cold chat stays in **Quiet**, and its card still says it is cold.

For more information, refer to [What a chat costs](what-a-chat-costs.md).

## How Switchboard knows

Switchboard doesn't guess. It reads the time the cache ends from Claude Code's own status line on the chat's
computer.

When the status line hasn't drawn since the last reply, Switchboard uses how long that chat's cache lasted
before, 5 minutes or 1 hour, counted from the last reply.

!!! note ""
    When neither is known, the chat says **cache ?** instead of warm or cold. This is because a wrong countdown
    is worse than none.

## Where you see it

- **A card on the board**: a warm chat sits in <span class="sb-pill warm">Warm now</span> and counts down,
  for example **Cache good for 4:12**. A cold one says **Next message re-reads 290k**.
- **The strip above a chat's terminal**: **Warm 4:12** on the right, amber in the last minute, then **Cold,
  re-reads 290k**, or **Cache ?** when it isn't known.
- **Claude Code's status line**: Switchboard has it redraw every 15 seconds, so what it says about the cache
  stays true while nobody types.

To learn what to do with a cold chat, refer to [Resume or start fresh](../features/resume-or-start-fresh.md).
