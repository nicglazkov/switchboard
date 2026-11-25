---
description: Set how much of your weekly Claude limit one day should use, get told when a day goes over, and optionally stop new messages at a hard limit.
---

# Daily budget

<p class="sb-meta">Last validated: Nov 22, 2026</p>

The daily budget lets you decide how much of your weekly Claude limit one day should use. Switchboard tells you
when a day goes over it. A separate hard limit, which is off unless you turn it on, blocks new messages and
stops background chats when a day goes too far.

## Why it matters

A weekly limit doesn't warn you on Monday that Friday will be short. A daily budget turns the week into seven
smaller amounts, so one heavy day shows up the day it happens.

## Benefits

- **An early warning**: you hear about a heavy day while there is still a week left to even it out.
- **No lost room at the end**: in the last two days of the week the budget gives way, so you can use what
  earlier days saved.
- **A real stop when you want one**: the hard limit keeps a runaway task from spending the week overnight.
- **Nothing is deleted**: chats the hard limit stops can be resumed.

## Use cases

- **A tight plan**: keep the default and watch for the over-budget line.
- **Long unattended tasks**: turn on the hard limit, so a task left running can't take more than a set share.
- **Bursty weeks**: set a higher daily budget and a 5-hour cap, so one sitting can't fill the window.

## How it works

The budget is a percent of your weekly limit per day. The default is 14.3%, which spreads the week evenly over
seven days. Switchboard works out what today has used from the weekly number Claude Code reports.

When a day reaches the budget:

- The **Budget** section of **Usage** says **Over budget**.
- Each chat mentions it to you once that day, then carries on.

Nothing is blocked. The budget only tells you.

In the last 48 hours before the weekly reset, the budget gives way. A day may then use whatever the week has
left, divided by the days left, when that is more than the budget. Before those 48 hours, the budget is exactly
what you set.

A **5-hour cap** does the same for the 5-hour window. It is off unless you set one, it never gives way, and it
clears when the window resets.

### The hard limit

The hard limit is a second, higher line that acts. When it is on and a limit is reached:

- New messages in every chat are blocked, with a line in the chat that says why and how to lift it.
- Background chats that are working are stopped, once that day. They can be resumed.
- Chats open in a terminal on their own computer are left running. Their next message is blocked.
- Your iPhone gets **Hard limit hit**, if notifications are set up.

The hard limit has a **Daily limit** and an optional **5-hour limit**. The daily limit gives way in the last 48
hours of the week the same way the budget does. The 5-hour limit never does.

## Set a daily budget

To set a budget:

1. Open **Settings**, then **Budget**.
2. Set **Daily budget**.
3. Optionally set a **5-hour cap**.

### Prerequisites

Before you can set a budget, you must:

- Have at least one chat that has replied this week, so Switchboard knows your weekly number.

### Set the daily budget

In **Settings**, under **Budget**, **Daily budget** has a switch and, when it is on, a number with − and +
buttons. The number is a percent of the week per day, from 0.1 to 100.

### Set a 5-hour cap

In the same group, turn on **5-hour cap**. It starts at 50% of the 5-hour window, and − and + change it. On a
plan that reports no 5-hour window, the cap can't be checked, and Settings says so.

## Turn on the hard limit

In **Settings**, under **Budget**, turn on **Hard limit**. Two more lines appear:

- **Daily limit**: a percent of the week per day. It starts at 20%.
- **5-hour limit**: a percent of the 5-hour window. It is off until you turn it on, and then starts at 80%.

Set the daily limit above the daily budget, so you are told before anything is stopped.

## Lift the hard limit for a day

When the hard limit has been reached, **Settings**, under **Budget**, says **Hard limit reached**. Select
**Lift for today**. New messages go through again until midnight on your home computer, the computer that
keeps the board.

**Undo** turns the limit back on. New messages are blocked again, but no more background chats are stopped
that day.

## Turn off the budget

In **Settings**, under **Budget**:

- Turn off **Hard limit** to stop Switchboard from blocking anything.
- Turn off the switch beside **Daily budget** to stop the over-budget notes. **Usage** then says **No daily
  budget**.

## Limitations

- **The numbers arrive with replies.** Switchboard learns today's use from each reply. A limit is noticed at
  the next reply after it is crossed, not at the exact moment.
- **Only your computers are stopped.** Use in the Claude apps counts toward the week, but Switchboard can't
  block or stop anything there.
- **Scheduled prompts are blocked too.** While the hard limit is reached, a prompt fired by a schedule is
  blocked like one you type. Reports from background tasks and subagents still go through, so work already
  under way isn't left hanging. Refer to [The cold chat guard](cold-chat-guard.md) for the guard's different
  rule.
- **A day follows the home computer's clock.** Refer to [Usage and budget](../usage.md#troubleshooting).

## Troubleshooting

### A chat says the hard limit blocked my message

The line in the chat names the limit that was reached and today's number. To go on, select **Lift for today**
in **Settings**, under **Budget**, or wait until tomorrow. For the 5-hour limit, wait for the window to reset.

### The 5-hour cap does nothing

Your plan may not report a 5-hour window. Settings then says **Your plan reports no 5-hour window right now, so
this cannot be checked.**
