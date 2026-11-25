---
description: See every limit of your Claude plan and when it resets, extra usage, usage by product and by day, and the chats that used the most this week.
---

# Usage and budget

<p class="sb-meta">Last validated: Nov 22, 2026</p>

**Usage** lets you see how much of your Claude plan you have used, on every limit your plan has, without
opening claude.ai. It also shows which days and which chats used the most.

## Benefits

- **Every limit in one panel**: the 5-hour session, the week, and the week for a single model when your plan
  has one, each with its reset time.
- **Your whole account**: what you use in the Claude apps counts too, not only chats on your computers.
- **The chats behind the number**: the five chats that used the most this week, biggest first.
- **A pace, not only a level**: the week says where you will end at the current rate.

## Use cases

- **Before a big task**: check that the 5-hour window has room for it.
- **Midweek**: see whether the week is on pace to run out before it resets.
- **After a heavy day**: find the chat that took the most and decide whether to start it fresh.

## How it works

Switchboard doesn't sign in to your Claude account and never reads your sign-in. Every number comes from
Claude Code on your own computers, in three ways.

**From each reply.** Claude Code's status line reports the 5-hour session and the week after every reply. Each
chat only knows what its last reply saw, so Switchboard takes the highest number any chat has seen in the
current window. This is because use only goes up until a window resets. After a reset, a limit shows **--**
until the next reply brings a number.

**From Claude Code's usage report.** The limits for one model, extra usage, the week by product and your
plan's name come from Claude Code's own usage report. One of your computers asks its Claude Code for it:

- Every 15 minutes while chats are replying.
- Every hour otherwise.
- When you open the panel or select **Refresh**.

Asking uses none of your plan. The report covers your whole account.

**From the chats' history.** For **Most used this week**, the helper on each computer adds up the tokens of
every reply since the weekly reset, from the history Claude Code keeps there.

## Open Usage

Select the **Week** chip at the top of the window. The panel has these sections:

| Section | What it shows |
|---|---|
| **Plan limits** | A bar for each limit your plan has, how much is used, and when it resets, as a time and as how long from now: **Resets Tue 9pm, in 6d 20h**. The week also says **On pace for** and a percent. |
| **Extra usage** | Whether it is on, **Spent this month**, **Monthly limit**, **Left this month**, and **Balance** if you have a prepaid one. |
| **This week by product** | How the week splits between Claude Code, the Claude apps and the rest. |
| **This week by day** | A bar for each day since the reset, and under it how the **Last 4 weeks** ended. |
| **Most used this week** | The five chats that used the most since the weekly reset. |
| **Budget** | Today against your daily budget, and whether the hard limit is on. |

The line at the top says how old the numbers are. **Manage on claude.ai** opens your account's usage page.
Limits and extra usage are changed there. Switchboard only shows them.

## Read the limits

### The 5-hour session

The 5-hour session is a window that opens with your first message and resets five hours later. A window that
has ended reads as empty, and the next reply starts a new one. On a plan with no such window the panel says
**5-hour session: not reported on this plan.**

### The week

The week is your plan's main limit. Reset times are shown in the time zone of the device you are looking at.
**On pace for** is where the week ends if the rest of it goes like the part so far.

### The last four weeks

Under **This week by day**, **Last 4 weeks** shows where each of the past weeks ended. A new setup says
**History starts this week** until there is a past week to show.

In **This week by day**, today's bar turns amber over your daily budget and red at the hard limit. Refer to
[Daily budget](features/daily-budget.md).

## Most used this week

**Most used this week** lists the five chats that used the most since the weekly limit last reset, each with
its computer and its share. Select one to open it.

The tokens are weighted the way cost estimates are: reading from the cache 0.1, writing it 1.25 or 2, plain
input 1 and Claude's output 5. A chat's share is its part of everything Switchboard counted this week on all
your computers. For more information, refer to [What a chat costs](concepts/what-a-chat-costs.md).

- **It is an estimate**: your plan doesn't say what each chat took. The shares say which chats were the big
  ones. They aren't percents of your weekly limit.
- **Only chats on your computers count**: chats in the Claude apps aren't in the list. **This week by
  product** shows those.
- **A chat's subagents count as the chat**: so do the chats it replaced. A chat you resumed, started fresh or
  compacted is one row with what came before it.
- **A stopped or deleted chat stays**: it is listed by name until the week ends, with nothing to open.
- **An offline computer keeps its numbers**: the last ones it sent, until the weekly reset.

The list is brought up to date every two minutes.

## Daily budget

A daily budget is how much of the week one day should use. The default, 14.3%, spreads the week evenly over
seven days. When a day reaches it, Switchboard tells you, and each chat mentions it once that day.

For more information, refer to [Daily budget](features/daily-budget.md).

## Hard limit

The hard limit is off unless you turn it on. When a day reaches it, new messages are blocked and working
background chats are stopped. **Lift for today** turns it off until midnight.

For more information, refer to [Daily budget](features/daily-budget.md#the-hard-limit).

## Limitations

- **Numbers arrive with replies.** With no chat replying, the 5-hour session and the week can be minutes or
  hours old. The top of the panel says how old.
- **One account is assumed.** Switchboard treats all your computers as signed in to the same Claude account.
- **No date for extra usage.** Switchboard doesn't show when extra usage starts over, because Claude Code
  doesn't report one.
- **Days follow the home computer's clock.** A day runs from midnight to midnight in the time zone of your
  home computer, the one that keeps the board.

## Troubleshooting

### The panel says No plan numbers yet

No chat has replied since the numbers were last cleared. They show after a chat's next reply. Select
**Refresh** to ask Claude Code for them now.

### Most used this week is empty after an update

After a Switchboard update, each computer reads this week's history once, a little at a time. The list can
take a few minutes to fill. A computer whose helper is too old to count is left out, and the panel says so.

### The days are split at the wrong hour

The home computer counts days in its own time zone. If it runs in a different zone from you, set
`SWITCHBOARD_TZ` on it:

```text
SWITCHBOARD_TZ=America/Los_Angeles
```

Replace `America/Los_Angeles` with your time zone.
