---
description: Pair a computer with your home computer by its device key, using a short code or a link, from the desktop app or the command line. List paired devices, remove one, and fix what the helper's status reports.
---

# Pair a computer

<p class="sb-meta">Last validated: Nov 22, 2026</p>

!!! note ""
    Device keys arrived with Switchboard 0.9.0 and are off until you turn them on.

Pairing lets a computer connect to your home computer, the one of your computers that runs Switchboard itself, by
its own device key. You make a code on the home computer, type it on the other computer, and the two stay
connected wherever they are.

## Benefits

- **No address to remember**: a paired computer finds the home computer by its key, at home or away.
- **Encrypted between the two**: nothing in between can read what they send, including a relay.
- **One place to cut a device off**: remove a computer on the home computer and its key stops working at once.
- **Nothing to open on your router**: both computers connect outward.

## Use cases

- **A laptop that travels**: it reaches the home computer from any network, with no private network to set up.
- **A computer in another house**: two home networks that can't reach each other meet through a relay.
- **A computer you gave away**: remove it from the list, and it can't connect again.

## How it works

Each computer has a device key, a secret it makes for itself. The home computer keeps a list of the keys it
accepts. A code or a link, good for ten minutes and for one use, puts a new computer on that list.

For more information, refer to [Device keys and pairing](../concepts/device-keys-and-pairing.md).

## Pair a computer with a code

To pair a computer:

1. Turn keys on, on the home computer.
2. Make a code on the home computer.
3. Enter the code on the other computer.
4. Check the connection.

### Prerequisites

Before you can pair a computer, you must:

- Have the helper, `switchboard-node`, on both computers. Refer to [Add your computers](../machines.md).
- Have a relay set on the home computer if you want a short code. A link works without one. Refer to
  [Run your own relay](run-your-own-relay.md).

### Turn keys on

In the desktop app, on the home computer itself:

1. Open **Settings** and select **Devices**.
2. On the card **Connect your computers with keys**, select **Turn keys on**.

The card says "Switchboard is restarting on the home computer. This takes a few seconds." The page then shows
the list of paired devices. Connecting by address keeps working, and you can turn keys off again.

Some home computers can't restart their own server, and a Windows home computer is one of them. The card then
shows the command to run on the home computer, with **Copy** beside it.

Without the app, run this on the home computer:

```text
switchboard keys on
```

It answers "Keys are on. This home computer still answers by address as well." Computers can now connect by key,
and everything that connects by address keeps working. The command needs the helper on the home computer, and
tells you to run `switchboard link` first when it isn't there. For the other modes, refer to
[Connections](../concepts/connections.md#the-three-modes).

To confirm, ask the helper on the home computer:

```text
switchboard-node status
```

It answers with these lines, among others:

```text
Keys: on. This is the home computer (07ef0adb).
Paired devices: none yet. Add one with: switchboard-node code
Relay: https://relay.example.com (connected; from settings)
```

The eight characters in brackets are the start of the home computer's id.

### Make a code

On the home computer, run:

```text
switchboard-node code
```

The helper prints a short code, a link, and the link again as a QR code:

```text
Code: K7PM-2XQ9
Link: switchboard://pair/1?h=07ef0adb...&r=https%3A%2F%2Frelay.example.com&s=...&n=example-home
Both work once, for the next 10 minutes. On the other computer: switchboard-node pair <code or link>
```

The code and the link are secrets until they are used. Don't post them where others can read them.

`switchboard code` on the home computer does the same, and so do `switchboard pair`, `switchboard devices` and
`switchboard forget` for the commands below.

### Enter the code on the other computer

On the computer you are pairing, run:

```text
switchboard-node pair K7PM-2XQ9
```

Replace `K7PM-2XQ9` with your code, or paste the whole link in its place. When it works, the helper answers:

```text
Paired with example-home. This computer is examplemac there.
```

The last name is what the board calls this computer.

Common flags:

- `--name NAME`: the name the home computer lists this computer under. The default is the computer's own name.
- `--addr IP:PORT`: an address to try for the home computer. Only a link made without a relay needs it.

If the helper wasn't running, it also prints `Start it with: switchboard-node run`.

### Check the connection

On the computer you paired, run `switchboard-node status`. It answers with these lines, among others:

```text
Keys: on. This computer's id is 3f9a1c2e.
Home computer: example-home (07ef0adb), connected directly (3 ms)
127.0.0.1:8130 here leads to the home computer's server.
127.0.0.1:8132 here leads to the home computer's hub.
Relay: https://relay.example.com (connected; from home)
```

The two lines in the middle mean that programs on this computer reach the home computer through the helper, at
this computer's own address.

## Pair from the desktop app

The desktop app has the same steps on a page of its own, **Settings**, **Devices**. The page shows once keys are
on. Before that, it shows on the home computer itself, with the card that turns them on.

### Get a code

1. Open **Settings** and select **Devices**.
2. Select **Add a device**.

The page shows a code, a link, and the link as a QR code, with **Copy code** and **Copy link**. It counts down,
for example "Works once, for 9:41". When the time runs out it says **Expired**, and **Get a new code** makes
another.

A home computer with no relay shows a link and a QR code only. This is because short codes need a relay.

### Join on the other computer

1. On the other computer, open the app. The first time it starts, it shows **Join your home computer**.
2. Type the code, or paste the link.
3. Select **Join**.

The app says "Finding your home computer. This can take a few seconds." and then "Paired with example-home.
Opening the board." On the home computer, the card turns to the new computer's name and **is paired**.

A link from a home computer with no relay can't say where that computer is. The app then asks for one more thing,
**Home computer's address**. The address is shown on the home computer, in **Settings**, **Devices**, under
**This computer's address**. It looks like `192.0.2.20:41641`.

An app that already connects by address doesn't show the join screen. On that computer, open **Settings**, select
**Devices**, and select **Pair this computer**. Switchboard gets a code and uses it there, with nothing to type.

### Read the list

The heading counts the paired devices, for example **3 devices**. Each row has the device's name, whether it is a
computer or a phone, and how it was paired. Beside it is one of these:

| The row says | What it means |
|---|---|
| **Connected**, with **Direct** or **Through the relay** under it | The device is connected with its key right now |
| **Last seen**, with a time | The device connected with its key before, and isn't connected now |
| **Not connected yet** | The device is paired and hasn't connected with its key |

The page reads the list again every 10 seconds while it is open, so a row changes by itself.

To remove a device, select **Remove** beside its name, then **Remove** with the device's name to confirm. Its key
stops working right away, and it needs a new code to come back.

### Set the relay

On the home computer, the same page has a **Relay** section. It shows the relay in use and where it was set, and
**Change** lets you type another. Refer to [Run your own relay](run-your-own-relay.md#set-the-relay-in-the-desktop-app).

### Close the network port

Once every computer connects with its key, the page offers **Close the network port**. Refer to
[Connections](../concepts/connections.md#before-you-choose-only).

## List paired devices

On the home computer, run:

```text
switchboard-node devices
```

Each line is one device: its name, its kind, the start of its id, and how it is connected.

```text
example-mac  computer  3f9a1c2e  connected directly
example-linux  computer  9b41d07a  last seen 2 hours ago
```

A connected device says **connected directly** or **connected through the relay**. One that isn't connected
says when it was last seen, or **not seen since the helper started**.

## Remove a device

On the home computer, run:

```text
switchboard-node forget example-mac
```

Replace `example-mac` with the device's name as `switchboard-node devices` shows it. The start of its id, six
characters or more, works too. The helper answers:

```text
Removed example-mac. Its key no longer works here.
```

The device is disconnected in the same moment. A removed computer stays on the board, offline, until you remove
it in **Settings** too.

## Limitations

- **The phone apps that pair are not released yet.** A phone pairs with the same code, by scanning it. Refer to
  [Pair a phone](pair-a-phone.md).
- **A short code needs a relay.** A code is too short to say where the home computer is, so the two computers
  meet at a relay. Without one, `switchboard-node code` prints a link only.
- **A Mac's helper can't start a connection to a local network address** unless macOS lets it use the local
  network. Give a Mac the relay's public or Tailscale address, not an address on your home network.
- **A Windows home computer needs the hub for keys.** The hub is installed on Windows only from a copy of
  Switchboard that carries it. The desktop app can't switch keys on for a Windows home computer, and shows the
  PowerShell command to run in its place. Refer to [Install Switchboard](../install.md#a-windows-server).
- **A computer pairs with one home computer.** To move it to another, pair it again there.

## Troubleshooting

`switchboard-node status` prints one line that starts with `Problem:` for each thing that is wrong. Find yours
below.

### Status says Keys: off

Keys aren't turned on for this computer. On the home computer, run `switchboard keys on`, as described in
[Turn keys on](#turn-keys-on). Any other computer turns keys on by itself when you pair it.

### Status says the helper is not running

```text
Keys: on, but the helper is not running. Start it: switchboard-node run
```

Start the helper with `switchboard-node run`, or run `switchboard-node install` once so that it starts when you
log in.

### No relay is set

```text
Problem: No relay is set, so other computers reach this one only where they can connect to it directly (the same network or a tailnet).
```

This appears on the home computer. Pairing by link still works on the same network or over Tailscale. To pair
with a short code, or to connect from anywhere, set a relay:

```text
switchboard-node relay https://relay.example.com
```

Replace `https://relay.example.com` with your relay's address.

### Cannot reach the relay

```text
Problem: Cannot reach the relay at https://relay.example.com (...).
```

Check that the address is right and that the relay is up. Refer to
[Run your own relay](run-your-own-relay.md#check-that-it-works).

### This Mac is not letting switchboard-node use the local network

```text
Problem: This Mac is not letting switchboard-node use the local network. Turn it on in System Settings, Privacy & Security, Local Network.
```

Turn the switch on for `switchboard-node`, or use a relay address that isn't on your home network.

### Cannot reach the home computer

```text
Problem: Cannot reach the home computer (...).
```

The home computer is off, asleep or offline, or the two computers have no path to each other. Check that the
home computer is on and that both computers show the same relay in their status.

### The home computer no longer lists this computer

```text
Problem: The home computer no longer lists this computer. Pair it again: switchboard-node pair <code or link>
```

Someone removed this computer on the home computer. Make a new code there and pair again.

### Another computer is using this computer's key

```text
Problem: Another computer is using this computer's key (a copied disk or a restored backup). Remove this computer at the home computer and pair it again.
```

Two computers hold the same key file, which happens when a disk is copied or a backup is restored onto a second
computer. Remove the device on the home computer, then pair each computer again.

### A port on this computer is in use

```text
Problem: 127.0.0.1:8130 on this computer is in use by another program, so programs here cannot reach the home computer's server through it.
```

Another program holds the port the helper needs. Stop that program, then restart the helper.

### A computer needs an update

```text
Problem: examplemac needs an update before it can use keys.
```

This appears on the home computer for a computer that was on your board before keys and whose helper is too old.
Update Switchboard on that computer. Refer to [Updates](../updates.md).

### The list of paired devices cannot be read

```text
Problem: The list of paired devices cannot be read, so every device is refused.
```

The file `~/.local/share/switchboard/paired.json` on the home computer can't be opened, or was written by a
newer version. Switchboard refuses every device until the file reads again. This is because an unreadable list must
never be treated as an empty one that it could then overwrite.

### That code is wrong, or has expired

The helper says **That code is wrong. Check it and try again.** or **That code has expired or was already
used. Make a new one.** A code works once, for ten minutes, and five wrong tries end it. Run
`switchboard-node code` again on the home computer.

### A short code finds the home computer through a relay

```text
A short code finds the home computer through a relay, and none is set here. Use the link instead, or set the relay first: switchboard-node relay <url>
```

The computer you are pairing has no relay to look for the home computer at. Paste the link, which names the
relay, or set the same relay the home computer uses.
