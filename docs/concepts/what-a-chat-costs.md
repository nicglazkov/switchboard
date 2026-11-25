---
description: How Switchboard estimates what the next message in a chat will cost, what Resume and Start fresh each cost, and where one becomes cheaper than the other.
---

# What a chat costs

<p class="sb-meta">Last validated: Nov 22, 2026</p>

What a chat costs is how much of your Claude plan its next message will use. Switchboard estimates that before
you send the message, so you can choose between going back to the chat and starting a new one from a summary.

## Weighted tokens

Claude counts text in tokens, which are pieces of words. Not every token costs the same, so Switchboard gives
each kind a weight and adds them up into one number, such as **about 92k**.

| Kind of token | Weight |
|---|---|
| Read from the cache | 0.1 |
| Read new | 1 |
| Written to a 5-minute cache | 1.25 |
| Written to a 1-hour cache | 2 |
| Written by Claude as its reply | 5 |

The weights are the ratios between prices on Claude's API (application programming interface). Your plan may
weigh tokens differently, and that isn't published. This is why every number is an estimate, and the app says
so.

## Not a percent of your week

Switchboard shows these estimates in tokens, not as a percent of your weekly limit. This is because nothing
reports how many tokens one percent of a plan holds, and Switchboard doesn't make that number up.

For a sense of scale: in one month measured on one setup, one percent of the weekly limit came to about 8.3
million weighted tokens, and it moved between 5.6 and 9.7 million from week to week. By that measure a cold
message costing 300k is about a thirtieth of a percent.

## What Resume costs

Resume goes back to the chat as it is, so the cost follows the size of the chat:

- **While warm**: the whole chat is read from the cache, at 0.1. A chat of 180k counts as about 18k.
- **Once cold**: all but the first 29k is written to the cache again, at 1.25 or 2. The first 29k is the start
  every chat shares, and it is read from the cache. A chat of 180k with a 1-hour cache counts as about 305k.
- **Every reply after that**: the chat is warm again, and each reply reads all of it at 0.1.

## What Start fresh costs

Start fresh has two parts, and neither depends much on how long the old chat was:

- **Writing the summary**: the model that writes it reads a condensed copy of the chat, without tool output,
  and writes about 2.4k tokens. The condensed copy is about 5% of the chat. For a chat of 180k this part counts
  as about 24k. A summary written after the chat's last reply is used again and costs nothing.
- **The new chat**: a new chat is not empty. It holds about 58k tokens of its own before your summary: Claude
  Code's instructions, tools, your `CLAUDE.md` and memory. About half of that is already cached. With the
  summary and a short recap, the first message counts as about 68k.

Together that is about 92k, however long the old chat was.

## Where one becomes cheaper

Resume grows with the chat and Start fresh hardly does, so there is a size where they meet.

<figure class="sb-fig" markdown="0">
--8<-- "docs/diagrams/cost-break-even.svg"
<figcaption>Figure 1. Below about 80k tokens, going back to a cold chat costs less than starting fresh. Above
it, the gap widens with every token. The lines are estimates for a 1-hour cache.</figcaption>
</figure>

In real use the two met between 75k and 80k tokens. Switchboard therefore calls a chat under 80k small and
suggests **Resume** for it, even when it is cold.

The measured savings by size of the cold chat:

- **Under 70k**: the median fresh start saved 2k, and two in five cost more than going back.
- **70k to 100k**: a fresh start saved 25k to 44k.
- **150k or more**: a fresh start saved 184k to 294k, and none cost more.

## How much Start fresh saves over a month

These sizes are medians from 178 fresh starts on three computers, from October 20 to November 21, 2026. Your
own numbers differ with your tools and your `CLAUDE.md`.

Over that month Start fresh saved about 1.2% of everything the plan spent, counting the first message of each
new chat. Counting the cheaper replies that follow in a smaller chat, the estimate is about 4%.

The total is small because cold messages to big chats are rare. Most of a plan goes to replies in warm chats
and to subagents, which Start fresh doesn't touch. Start fresh makes single expensive messages much cheaper. It
doesn't halve your usage.

## Where you see the numbers

A chat's details show both costs side by side, for the cache as it is that second. **How is this estimated?**
under them lists the sizes and weights used for that chat. The guard shows the same two numbers when it holds a
message.

Since version 0.8.23, each computer records what every summary really used and counts it against what the
fresh start saved.

For more information, refer to [Resume or start fresh](../features/resume-or-start-fresh.md).
