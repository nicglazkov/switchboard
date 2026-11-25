---
description: Two ways back to a quiet chat. Resume reopens it as it is; Start fresh carries on in a new chat from a summary. What each does, what each costs, and which one Switchboard suggests.
---

# Resume or start fresh

<p class="sb-meta">Last validated: Nov 22, 2026</p>

Resume and Start fresh are the two ways back to a chat you have left alone. **Resume** reopens the chat as it
is, with its whole history. **Start fresh** writes a summary of the chat and carries on in a new chat that
starts from that summary.

## Why it matters

A chat that has gone cold makes Claude read all of it again on your next message. For a long chat that single
message can cost more than a whole new chat. Switchboard shows both costs before you choose.

## Benefits

- **You see the cost first**: a chat's details show what Resume and Start fresh would each cost right now.
- **Your instructions survive**: the summary keeps every instruction you gave that still applies, in your own
  words where they matter.
- **One click**: Switchboard writes the summary, opens the new chat in the same folder and moves the old one
  out of the way.
- **Nothing is deleted**: the old chat moves to **Replaced**, where you can still resume it.

## Use cases

- **A long chat from yesterday**: start fresh, so the next message doesn't read 200k tokens again.
- **A short chat that went cold**: resume it. Under 80k tokens, going back costs less than a summary.
- **A chat close to being compacted**: start fresh before Claude Code shortens the chat by itself.
- **A chat you need word for word**: resume it, and accept the one expensive message.

## How it works

Resume changes nothing about the chat. If the chat is warm, the next message is cheap. If it is cold, the next
message reads all of it again.

<figure class="sb-fig" markdown="0">
--8<-- "docs/diagrams/fresh-resume.svg"
<figcaption>Figure 1(a). Resume keeps everything, and a cold chat pays for all of it once more.</figcaption>
</figure>

Start fresh leaves the old chat where it is and opens a new one. The new chat reads only what every new chat
reads, plus the summary.

<figure class="sb-fig" markdown="0">
--8<-- "docs/diagrams/fresh-start.svg"
<figcaption>Figure 1(b). Start fresh replaces the history with a summary. The cost no longer depends on how
long the old chat was.</figcaption>
</figure>

The numbers in both figures are estimates for one cold chat of 180k tokens. For how they are worked out, refer
to [What a chat costs](../concepts/what-a-chat-costs.md).

### The summary

The summary is written on the chat's own computer by Claude, from a condensed copy of the chat. The condensed
copy holds your messages, text you pasted, Claude's replies and the names of the tools it used. It leaves out
what the tools returned.

A summary is 500 to 900 words, and never more than 1,000. It has seven parts:

- **Goal**: what you are trying to get done.
- **Standing instructions from the user**: every rule, preference, approval and refusal that still applies.
- **Decisions made**: what was settled, and why.
- **Current state**: what is done, and where it is.
- **Open threads and next steps**: what was in progress or promised.
- **Gotchas**: what went wrong, and how to avoid it.
- **Key files and places**: where the new chat should look first.

The new chat loads the summary with its first message. If a summary is still too long to load that way, the
new chat gets it without its least needed parts and is told where the whole summary is. The goal and your
standing instructions are never left out.

After the summary, Switchboard adds two things itself: a list of your instructions, word for word, and where
the old chat's full history is kept. Claude can search that history when the summary is missing a detail.

### What Switchboard suggests

A chat's details mark one of the two as **Suggested**. Switchboard applies these rules in order:

1. **The chat is working**: nothing is suggested. Open its terminal to watch or steer it.
2. **A message is held by the guard**: Start fresh.
3. **The chat is warm**: Resume. You only pay for your new message.
4. **The chat is within a tenth of where Claude Code compacts it**: Start fresh. With chats compacting at 300k,
   that is from 270k.
5. **The chat is cold**: whichever costs less. Under 80k tokens that is always Resume.

Where chats compact is a setting. For more information, refer to [Settings](../settings.md#context-window).

## Use Resume or Start fresh

To go back to a chat:

1. Open the chat's details.
2. Compare the two costs.
3. Choose **Resume** or **Start fresh**.

### Prerequisites

Before you can resume or start a chat fresh, you must:

- Have the chat's computer online. Refer to [Add your computers](../machines.md).

### Open the chat's details

Select the chat on [the board](../board.md). Its details show **Start fresh** and **Resume** with a line under
each that says what it does and what it costs. **How is this estimated?** shows the numbers behind the costs.

### Resume the chat

Select **Resume**, or press ++r++ with the chat selected. In a chat's menu the same action is **Resume as is**.
A chat that was stopped also offers **Resume and open**, which resumes it and opens its terminal.

### Start the chat fresh

Select **Start fresh**, or press ++f++. Switchboard asks you to confirm and says what the fresh start costs.
Writing the summary takes about a minute. The new chat has the old chat's name with "(part 2)" added.

If Claude is still replying, Switchboard asks whether to start fresh when the reply finishes, or now, which
stops the reply.

**Start fresh** is also on the strip above a chat's terminal, beside the cache. It isn't there while Claude is
working. For more information, refer to [Terminals](../terminals.md).

## Change who writes the summary

Summaries are written by Opus unless you change it. To change it, open **Settings**, then **Chats**, and under
**Summaries** set **Who writes them** to **Sonnet**. Sonnet is quicker and uses less of your plan. Opus writes
the most careful summaries.

## What a chat keeps

Both Resume and Start fresh keep what the chat ran with: its model, effort, permission mode, extra folders,
agent, extra instructions and Chrome setting. A star on the chat follows it into the new chat.

## Limitations

- **A summary is not the chat.** Details that didn't make it into 900 words are only in the old chat. Claude
  can read them there, but it has to go and look.
- **Tool output is not summarized.** What a command printed or a file contained reaches the summary only
  through what you and Claude said about it.
- **The costs are estimates.** They use API price ratios, and your plan may count tokens differently.
- **Resume is for chats that have stopped.** A chat that is still running is already open, so **Resume as is**
  is greyed out. Open its terminal instead, or answer it on its own computer if it runs in a terminal there.
- **Start fresh saves on single messages, not on your whole week.** Measured over a month, it saved about 1.2%
  of all usage, or about 4% counting later replies.
