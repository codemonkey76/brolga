# Using Brolga

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
| Search mail | Ctrl+E or F3 |
| Undo the last change | Ctrl+Z |
| Sync now | F9 |
| In the calendar: new event, today | N, T |

On macOS, Cmd takes the place of Ctrl.

## What Brolga does to keep you safe

- **Remote pictures stay out** until you ask for them, so a sender cannot
  tell that you opened their message. You can let a sender's pictures load
  every time.
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
shared with you appears in the calendar list once you have accepted the
invitation, labelled with whose it is.

## Old mail from Outlook

**Import a PST file…** on the Accounts screen reads an Outlook data file into
an archive kept on this computer. The file itself is only read, never
changed. The archive is read and searched like a mailbox, but nothing can be
sent from it and it is not uploaded to Microsoft 365.

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
