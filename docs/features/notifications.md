---
description: The notifications Switchboard sends, on the desktop and on the iPhone, when each one fires, and how to turn each one off.
---

# Notifications

<p class="sb-meta">Last validated: Nov 22, 2026</p>

!!! note ""
    Phone notifications exist only for the iPhone app, which you can't download today. There are none for
    Android.

Notifications tell you that a chat needs you when you aren't looking at the board. The desktop app posts them on
the computer you're at, and the home computer can send them to an iPhone.

## Benefits

- **You hear about the chat that waits**: a question on a computer in another room reaches the screen in front of
  you.
- **No repeats**: a chat that needs you is announced once for each time it starts waiting, not at every update.
- **Each kind has its own switch**: turn off the ones you don't want and keep the rest.

## Use cases

- **Work in another app**: keep Switchboard in the background and switch to it only when a chat asks something.
- **Start a task and walk away**: learn that a fresh start or a new chat is ready without watching it.

## How it works

There are three places a message from Switchboard can show.

| Where | Who sends it | Needs |
|---|---|---|
| A system notification on your computer | The desktop app | The app to be open |
| A notice inside the app | The desktop app | Nothing |
| A notification on your iPhone | The home computer, through Apple's push service | The iPhone app, and a push key on the home computer |

The home computer is the one of your computers that runs Switchboard. Desktop notifications never leave your
computers. An iPhone notification carries a chat's name, a computer's name and a short status line through
Apple's push service. For more information, refer to [Privacy and security](../privacy.md).

### On your computer

The desktop app posts three system notifications.

| Notification | When it fires |
|---|---|
| A chat needs you | A chat starts waiting for you: Claude asked a question or wants a permission, or a message you typed is held. It shows the chat's name, what it's waiting for and its computer |
| **A task Claude sent is waiting** | A task that one of your chats sent to another computer is stopped at a prompt that only you can answer |
| A task finished | Something you started from the app, such as **Start fresh**, a new chat or **Resume**, finished or failed while you weren't watching it |

None of them shows while a Switchboard window is in front, because the board already shows the same thing. A chat
is announced once for each time it starts waiting, and the notification is taken back when the chat stops
waiting. Nothing is announced when the app starts, and a chat that has waited for more than a day is parked and
stays silent.

Click a notification to open the board at that chat.

### Inside the app

Short messages appear in the bottom right corner of the window. When one leaves the corner, it waits behind
**Notices** at the right of the status bar until you act on it or select **Ignore**. **Ignore** and **Clear all**
each ask you to confirm.

### On your iPhone

The home computer sends four kinds, to every iPhone that is signed in to it, even when the app there is closed.

| Kind | When it fires |
|---|---|
| **Held messages** | A message you typed into a large chat that has gone cold was held. It's sent 20 seconds after the hold, unless you answer first. One for each held message, and not again for the same chat within 30 minutes of your answer |
| **Finished tasks** | Something you started from an app finished or failed while no app was showing it. Tasks that a chat sent, and ones you cancelled, don't notify |
| **Chats not in use** | Chats sit open and use memory without doing anything. At most once a day, and only when there is a new one |
| **Hard limit** | The hard limit of your budget was reached and stopped chats. Once a day, and not on a day you lifted the limit |

A notification that can't be delivered within a day is dropped, not shown late.

## Use notifications

1. Allow Switchboard to notify you on your computer.
2. Set up iPhone notifications, if you have the iPhone app.

### Allow notifications on your computer

Desktop notifications are on when you install the app. Your system may ask once whether Switchboard may show
notifications. If you said no, allow it again in your system's notification settings.

### Set up iPhone notifications

The home computer needs a push key from an Apple developer account before it can send anything. Until it has
one, **Settings**, **Notifications** says **iPhone notifications aren't set up**. Refer to
[The apps](../apps.md#iphone-notifications).

## Turn off notifications

Open **Settings** and select **Notifications**.

| Switch | Group | Default | Applies to |
|---|---|---|---|
| **When a chat needs you** | **On this Mac** (**On this computer** on Windows and Linux) | On | This computer only. It covers all three desktop notifications |
| **Held messages** | **On your iPhone** | On | Every iPhone |
| **Finished tasks** | **On your iPhone** | On | Every iPhone |
| **Chats not in use** | **On your iPhone** | On | Every iPhone |
| **Hard limit** | **On your iPhone** | On | Every iPhone |

You can also turn the desktop switch off from the command palette. Press ++cmd+k++ (++ctrl+k++ on Windows and
Linux) and select **Turn off needs-you notifications**.

Notices inside the app can't be turned off.

## Limitations

- Desktop notifications come only while the desktop app is open. This is because the app posts them itself.
- There is one switch for all desktop notifications, not one for each kind.
- iPhone notifications need the iPhone app, which isn't available to download today, and a push key on the home
  computer.
- There are no notifications for Android.

## Troubleshooting

### No notification when a chat needs you

1. Check that **When a chat needs you** is on in **Settings**, **Notifications**.
2. Check that your system allows notifications from Switchboard, and that a focus or do-not-disturb mode isn't
   holding them.
3. Check that no Switchboard window is in front. The app doesn't notify you about a board you're looking at.

### The same chat doesn't notify twice

A chat is announced once each time it starts waiting. It notifies again after it has gone back to work and then
waits for you again.
