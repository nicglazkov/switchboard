---
description: A chat shows as cold, or a message is held, although you answered it minutes ago. How long the cache lasts and what cache ? means.
---

# A chat says cold sooner than you expect

<p class="sb-meta">Last validated: Nov 23, 2026</p>

A chat you answered a few minutes ago already sits in <span class="sb-pill cold">Gone cold</span>, or the
message you sent to it was held as if the chat were cold.

## Cause

Claude keeps a ready copy of a recent chat, known as the cache. The cache lasts 5 minutes or 1 hour after each
reply, and Claude Code decides which one a chat gets. A chat with the 5 minute cache is cold ten minutes after
your last message, even though it feels recent.

Switchboard doesn't guess the time. It reads the end of the cache from Claude Code's own status line. When the
status line hasn't drawn since the last reply, Switchboard uses how long that chat's cache lasted before. When
neither is known, the chat says **cache ?**.

## Fix

1. Open the chat's terminal and read the strip on top. It says **Warm** with the time left, **Cold** with what
   the next message reads again, or **Cache ?**.
2. If the chat is cold and small, send your message. A chat under 80k tokens is cheap to go back to.
3. If the chat is cold and big, choose between **Resume** and **Start fresh**. Refer to
   [Resume or start fresh](../features/resume-or-start-fresh.md).
4. If your message was held, send the same message again or type `go` within 15 minutes.

If the card is gray, the state is old, not wrong: the computer hasn't reported for 45 seconds. Refer to
[A computer is offline or missing](computer-offline.md).

For more information, refer to [Warm and cold](../concepts/warm-and-cold.md) and
[Cold chat guard](../features/cold-chat-guard.md).
