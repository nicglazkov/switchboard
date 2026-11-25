---
description: Which folders Switchboard may open on each computer, the Claude folder, Anywhere in home, places added at a computer, the private folders that are never shown, and when Claude asks before working in a folder.
---

# Places and folders

<p class="sb-meta">Last validated: Nov 22, 2026</p>

A place is a folder on one of your computers that Switchboard may open: to start a chat in it, or to show it in
Files. Every computer has its own places, and the computer itself decides what they are.

This matters because the app you hold isn't always the computer that holds the files. A rule kept only in an app
could be skipped by another app, so each computer checks every request against its own places before it answers.

## The parts of a computer's disk

<figure class="sb-fig" markdown="0">
--8<-- "docs/diagrams/places-allowed.svg"
<figcaption>Figure 1. The Claude folder is always open. The rest of the home folder depends on one switch, private
folders never show, and a folder outside home has to be added at the computer.</figcaption>
</figure>

An app never names a folder by its full path. It names a folder under the Claude folder, a folder under the home
folder, or a folder under a place that was added by name. The small helper that Switchboard runs on each
computer turns that name into a real folder, and refuses anything that falls outside the groups in the figure.

## The Claude folder

The Claude folder is the folder on a computer where new chats start unless you pick another one. It's the first
place in every list, and it's always open to the app.

You pick it for each computer in **Settings**, under **Machines**, on that computer's page. Until you pick one,
chats on that computer start in its home folder. A chat that starts inside the Claude folder starts without a
question.

## Anywhere in home

**Anywhere in home** lets chats and Files use any folder in a computer's home folder, such as Documents, Desktop
or iCloud Drive. It's on for each computer unless you turn it off, in **Settings**, under **Machines**, on that
computer's page, in the group **Beyond the Claude folder**.

With it on, the computer's places also include **Home**, **Desktop**, **Documents**, **Downloads**, the usual code
folders (**Developer**, **Projects**, **Code**, **src** and **repos**) when they exist, and **iCloud Drive** on a
Mac. With it off, chats and Files stay in the Claude folder and the places added at that computer.

Turning it on gives an app less than it seems to. Settings can already move the Claude folder to any folder in
home, so the switch only saves you from moving it back and forth.

## Places added at a computer

A folder outside the home folder, such as a second drive, is reachable only as a place that someone added while
sitting at that computer. The home computer and the apps can never add one. This is because a place widens what
every app can reach on that computer, and that decision belongs to whoever is at its keyboard.

You add a place with the helper's `places add` command, give it a short name, and it then shows in that
computer's list under **Added on this machine**. Refer to [Commands](../commands.md#sbpy-the-helper).

## Private folders

Some folders hold keys, tokens, mail and browser data. Switchboard never lists them and never opens them,
whatever an app asks for:

- **Dot folders and dot files**: anything whose name starts with a dot, such as `.ssh`, `.aws`, `.config` and
  `.claude`, at any depth.
- **`~/Library`** on a Mac. The one exception is iCloud Drive, which lives inside it.
- **`AppData`**, **`Application Data`** and **`Local Settings`** on Windows.
- **Anything behind a symbolic link**: links are never followed.

Files that look like keys or secrets, such as `id_rsa` or a name ending in `.pem`, are listed but never read.
For the full list, refer to [Files](../features/files.md).

Think of the helper as a doorman who works from a short list kept at the door. An app can ask for any room it
likes, and the doorman only ever opens the ones on the list.

## Trust for a folder Claude hasn't worked in

Claude Code asks before it works in a folder for the first time, and remembers the answer. Switchboard keeps to
the same answers.

Inside the Claude folder there's no question. Anywhere else, a chat starts without a question only in a folder
you already let Claude into, or in a folder inside one. For any other folder the app shows **Let Claude work in**
followed by the folder's name, and says what yes means: Claude can read and change the files there and run
commands there, in that folder and the ones inside it.

Saying yes records the same answer that accepting Claude Code's own prompt would, so Claude Code doesn't ask
again in its terminal. If you once said no to that folder in Claude Code, the app tells you, and it asks again
every time. It never opens the folder silently.

Files shows where each folder stands before you start anything: **Claude can work here**, **Claude hasn't worked
here yet** or **You said no to Claude here before**.

## macOS privacy switches

On a Mac, macOS itself guards Desktop, Documents, Downloads and iCloud Drive. Each has a switch in System
Settings, under Privacy & Security, in Files and Folders, for the app that runs Claude Code on that Mac.

!!! note ""
    A place that macOS keeps closed says **No permission**. It doesn't show as an empty folder, and Switchboard
    can't turn the switch on for you.

Files offers **Open the switch on** followed by the Mac's name. That opens the right pane of System Settings on
that Mac's own screen, for the person sitting there to turn the switch on. **Check again** then reads the folder
once more.

For more information, refer to [Privacy and security](../privacy.md).
