# Using Brolga

## Installing

- **Linux:** `Brolga.flatpak` from the releases page, installed with
  `flatpak install --user Brolga.flatpak`; or the `.deb`, `.rpm` or
  `.AppImage`. The Flatpak can reach the network, the secret store,
  notifications and your Downloads folder, and no other file except one you
  pick in a file dialog. In the Flatpak, starting at sign-in does not work
  yet.
- **Windows and macOS:** the installers are not signed, so the system will
  warn you. On Windows, choose **More info** and then **Run anyway**. On
  macOS, open the app once, then allow it under **System Settings**,
  **Privacy & Security**, **Open Anyway**. Only do this for a file you
  downloaded from Brolga's own releases page, and check it against the
  `SHA256SUMS` file published beside it.

## Getting started

Open Brolga and choose **Sign in** on the Accounts screen. Your browser opens
on Microsoft's own sign-in page; Brolga never sees your password. If you are
told "Need admin approval", your organisation limits which apps people can
use: send your administrator [the administrators' guide](admins.md).

The first sync of a large mailbox takes a while. Mail shows as it arrives,
newest first, and you can read and write while it carries on.

## Finding your way

The rail on the left switches between **Mail**, **Calendar**, **People**,
**Accounts** and **Agents**. Press **Ctrl+K** anywhere for a list of
everything the window does, with its keys: type a few letters and press
Enter.

| To | Press |
|---|---|
| Go to Mail, Calendar, People | Ctrl+1, Ctrl+2, Ctrl+3 |
| Open the address book | Ctrl+Shift+B |
| Write a new message | Ctrl+N |
| Start a new meeting | Ctrl+Shift+Q |
| Reply, reply to all, forward | Ctrl+R, Ctrl+Shift+R, Ctrl+F |
| Send | Ctrl+Enter |
| Mark as read, as unread | Ctrl+Q, Ctrl+U |
| Flag or clear a flag | Insert |
| Delete, archive | Delete, Backspace |
| Move to a folder | Ctrl+Shift+V |
| Next, previous message | Ctrl+. and Ctrl+, |
| Choose several messages | Shift+Up, Shift+Down; Ctrl+A for all; Escape for one again |
| Search mail | Ctrl+E or F3 |
| Undo the last change | Ctrl+Z |
| Sync now | F9 |
| In the calendar: new event, today | N, T |

On macOS, Cmd takes the place of Ctrl.

## What Brolga does to keep you safe

- **Remote pictures stay out** until you ask for them, so a sender cannot
  tell that you opened their message. **Show blocked content** downloads
  them for that message once; **Trust sender** downloads them for every
  message from that address.
- **A message looks as it was laid out**, with its own colours, type and
  tables. On a dark desktop its colours are darkened to suit, keeping their
  hue; **Show it as written** puts back the sender's own. What is taken out is anything that could fetch
  from the internet or place one thing over another.
- **Nothing in a message runs.** Scripts, forms and frames are taken out, and
  a message's own styling cannot draw over the window.
- **Links open in your browser**, never inside Brolga, and a link that says
  one address while going to another is called out before you click.
- **Attachments are never opened for you.** They are saved where you say.

## Working offline

Everything already synced can be read and searched with no connection.
Changes you make offline, and messages you send, wait in the outbox and go
when the connection is back. If Microsoft 365 refuses one, Brolga says which
and why, and nothing is lost: an unsent message stays in Drafts.

## Shared mailboxes and calendars

Microsoft 365 does not list the mailboxes you have been given access to, so
add each by its address under your account on the Accounts screen. It then
sits beside your own mail, and you can send from it. A calendar someone has
shared with you appears in the calendar list once it is among your calendars
in Microsoft 365, labelled with whose it is.

## Old mail from Outlook

**Import a PST file…** on the Accounts screen reads an Outlook data file into
an archive kept on this computer. The file itself is only read, never
changed. The archive is read and searched like a mailbox, but nothing can be
sent from it and it is not uploaded to Microsoft 365.

## Keeping Brolga running

Under **This computer** on the Accounts screen:

- **Keep running when the window is closed** leaves Brolga fetching mail and
  giving reminders, with an icon in the system tray to open it again or quit.
  On a desktop with no system tray, closing the window still quits.
- **Start when I sign in to this computer** starts it for you, with no window
  if it is also kept running.

Both are off until you turn them on. Starting Brolga a second time always
brings the window you already have to the front.

## Opening email links

Installed from a package, Brolga offers itself as the program for `mailto:`
links. Choose it as your default mail program in your desktop's settings, and
a clicked address starts a new message in the window you already have open.

## AI agents

An agent on your computer can work with your mail through Brolga, with what
you allow and no more. See [Brolga for agents](mcp.md).

## What Brolga does not do

- Expand a distribution list into its members.
- Show an online archive mailbox or public folders.
- Share your signature with other mail programs: it is kept on this computer.
- Find rooms, unless your administrator has allowed it.
