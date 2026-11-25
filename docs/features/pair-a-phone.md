---
description: Pair an iPhone or an Android phone with your home computer by scanning a QR code, typing a short code or opening a link. What the Home computer page shows, and how to leave.
---

# Pair a phone

<p class="sb-meta">Last validated: Nov 22, 2026</p>

!!! note ""
    Device keys arrived with Switchboard 0.9.0 and are off until you turn them on. The iPhone and
    Android apps that pair are not released yet.

Pairing lets the Switchboard app on your phone connect to your home computer, the one of your computers that runs
Switchboard itself, by the phone's own device key. You scan a code on the home computer's screen once, and the
phone finds the home computer from any network.

## Benefits

- **No address and no VPN**: a paired phone finds the home computer by its key, at home and away.
- **One scan**: the camera reads the code, and the phone opens the board.
- **The last board stays with you**: when the home computer is out of reach, the phone shows the board as it
  last saw it and says how old it is.
- **One place to cut a phone off**: remove the phone on the home computer and its key stops working at once.

## Use cases

- **A phone on mobile data**: it reaches the home computer through the relay, with nothing to turn on first.
- **A reply written with no signal**: the phone keeps it and sends it when the home computer answers again.
- **A phone you lost or sold**: remove it from the list on the home computer, and it can't connect again.

## How it works

A phone pairs the way a computer does. The app makes a device key for the phone, and a code or a link from the
home computer, good for ten minutes and for one use, puts that key on the home computer's list of paired devices.

A computer leaves its key to the helper. A phone has no helper, so the same parts run inside the app. The app
connects directly when it can and through the relay when it can't.

For more information, refer to [Device keys and pairing](../concepts/device-keys-and-pairing.md).

## Pair a phone with the home computer

To pair a phone:

1. Get a code on the home computer.
2. Join on the phone.
3. Check the connection.

### Prerequisites

Before you can pair a phone, you must:

- Have keys turned on, on the home computer. Refer to [Pair a computer](pair-a-computer.md#turn-keys-on).
- Have a relay set on the home computer. This is because a phone finds the home computer through the relay named
  in the code's link. Refer to [Run your own relay](run-your-own-relay.md).
- Have a build of the app that pairs. Refer to [The apps](../apps.md#iphone).

### Get a code on the home computer

1. On the home computer, open the desktop app.
2. Open **Settings** and select **Devices**.
3. Select **Add a device**.

The page shows a code such as `K7PM-2XQ9`, a link, and the link as a QR code. All three work once, and the page
counts down the ten minutes.

Without the desktop app, run `switchboard-node code` on the home computer. It prints the same three in the
terminal.

### Join on the phone

A new install of the app opens on **Join your home computer**. In an app that already connects by address, open
**Settings**, select **Home computer**, then **Join your home computer**.

To scan the code:

1. Select **Turn on the camera**, and allow the camera when the phone asks.
2. Point the camera at the QR code on the home computer's screen. The app says **Looking for a code**, then
   **Found it**.

To type the code:

1. Select **Type the code instead**.
2. Enter the code in **Code or link**, or select **Paste** to paste the link.
3. Select **Join**.

The app says "Finding your home computer. This can take a few seconds." and then "Paired with example-home.
Opening the board."

You can also open the link itself on the phone, for example from a message you sent yourself. The app opens on
**Join your home computer** with the link filled in. Nothing is sent until you select **Join**.

### Check the connection

On the phone, open **Settings** and select **Home computer**. The page shows the home computer's name, and under
it one of these:

| The page says | What it means |
|---|---|
| **Direct**, with a time such as 12 ms | The phone talks straight to the home computer |
| **Through the relay** | The connection passes through the relay because no direct path exists |
| **Not connected** | The phone isn't connected to the home computer right now |

**Listed as** is the name the home computer shows for this phone in **Settings**, **Devices**. **Id** is the
start of the phone's id, and a tap copies all of it.

On the home computer, the phone appears in **Settings**, **Devices** as a **Phone** with **Connected** beside it.

## Use the app when the home computer is out of reach

A paired phone keeps the last board it saw. When the home computer doesn't answer, the app shows that board with
its age, such as **Updated 12 min ago**, and keeps trying by itself.

A reply you send in that time waits on the phone, and the board says so: "1 reply is waiting to send to" and the
chat's name. The app sends it when the home computer answers again. A reply that waited more than a day is not
sent. The app says **Reply expired** and puts the words back in that chat's message box.

If the chat went cold while your reply waited, the app asks first and offers **Resume**, **Start fresh** and
**Not now**. For more information, refer to [Warm and cold](../concepts/warm-and-cold.md).

## Join another home computer

A phone is joined to one home computer at a time.

1. On the phone, open **Settings** and select **Home computer**.
2. Select **Join another home computer**.
3. Scan or type a code from the other home computer.

## Leave the home computer

1. On the phone, open **Settings** and select **Home computer**.
2. Select **Leave this home computer**, then **Leave**.

The phone forgets the home computer. The home computer still lists the phone. To take it off that list too, open
**Settings**, select **Devices** on the home computer, and select **Remove** beside the phone's name.

## Connect by address instead

**Connect by address** stays in both apps, at the bottom of **Join your home computer** and of **Settings**,
**Home computer**. It is for a home computer that doesn't use codes yet. The address works on the same network,
or over Tailscale.

## Limitations

- **The apps that pair are not released yet.** The iPhone app isn't on the App Store or TestFlight, and there is
  no Android app to download.
- **A phone needs a relay on the home computer.** A link with no relay in it can't find the home computer from a
  phone. Connect by address in that case.
- **Widgets still use addresses.** The iPhone widgets read the board from the home computer's address, so they
  need an address that reaches.
- **Notifications don't come through the gateway yet.** The phone apps aren't wired to it, so iPhone
  notifications still need a push key on the home computer. Refer to [The apps](../apps.md#iphone-notifications).
- **One home computer at a time.** Joining another one replaces the first.

## Troubleshooting

Refer to [A phone can't join or can't reach home](../troubleshooting/phone-cant-join.md).
