# Brolga for administrators

What Brolga asks of a Microsoft 365 organisation, what it does with it, and
how to allow it, limit it or use your own app registration.

## What it is to Microsoft 365

Brolga is a desktop program. It signs a person in with their browser and
talks to Microsoft Graph as that person, with delegated permissions only. It
has no application permissions and no service of its own: there is no Brolga
server, and nothing it reads is sent anywhere but back to Microsoft 365.

- **App registration:** multi-tenant, a public client (no secret), redirecting
  to `http://localhost`. Client ID `3b0d6477-515b-429a-96b2-f8759528a5cc`.
- **Publisher:** Shane Poppleton, an individual, and not verified: Microsoft
  verifies publishers only through its partner programme, which is for
  organisations. If your organisation only lets people consent
  to apps from verified publishers, they cannot sign in until you approve the
  app or use your own registration (below).
- **Sign-in:** the authorisation code flow with PKCE, in the person's own
  browser, so your conditional access and multi-factor rules apply as they do
  anywhere else. On a computer with no browser, the device code flow.

## The permissions it asks for

All delegated. A person can consent to every one of these for themselves,
unless your organisation has turned user consent off.

| Permission | What it is for |
|---|---|
| `openid`, `profile`, `offline_access`, `User.Read` | Signing in, staying signed in, and the person's own name and address. |
| `Mail.ReadWrite` | Reading mail, and marking, flagging, moving, deleting and drafting it. |
| `Mail.Send` | Sending. |
| `MailboxSettings.ReadWrite` | Automatic replies, inbox rules and the list of categories. |
| `Calendars.ReadWrite` | The person's calendars and events. |
| `Calendars.ReadWrite.Shared` | Calendars other people have shared with them, and free and busy times. |
| `Contacts.ReadWrite` | Their personal contacts. |
| `Tasks.ReadWrite` | Their task lists and tasks in Microsoft To Do. Added in the version with Tasks; each person signs in again once to grant it. |
| `People.Read`, `User.ReadBasic.All` | Finding colleagues by name when addressing a message. |
| `Mail.ReadWrite.Shared`, `Mail.Send.Shared` | Shared mailboxes they have been given access to. Brolga cannot see a mailbox they have not. |

### One permission only you can give

| Permission | What it is for |
|---|---|
| `Place.Read.All` | Listing the organisation's meeting rooms, to find one that is free. |

Microsoft requires an administrator's consent for this one, so Brolga never
asks a person for it. Without it everything else works, and a room can still
be invited by its address. To turn room finding on, grant it for the
organisation: in the Microsoft Entra admin centre, under **Enterprise
applications**, open Brolga, then **Permissions**, then **Grant admin
consent**. People pick it up the next time they sign in.

## Approving it for everyone

If people are met with "Need admin approval" when they sign in, user consent
is restricted in your organisation. Open this address as an administrator,
with your own tenant's name or ID in place of `{tenant}`, and accept:

```
https://login.microsoftonline.com/{tenant}/adminconsent?client_id=3b0d6477-515b-429a-96b2-f8759528a5cc
```

Once you accept, the browser is sent on to `http://localhost` and may say the
page cannot be reached. That is expected: the approval has been recorded.

## Using your own app registration

You may prefer that Brolga runs under a registration your organisation owns.

1. In the Microsoft Entra admin centre, register an application: accounts in
   this organisational directory only.
2. Under **Authentication**, add the platform **Mobile and desktop
   applications** with the redirect URI `http://localhost`, and allow public
   client flows.
3. Under **API permissions**, add the delegated Microsoft Graph permissions
   in the tables above, and grant admin consent.
4. Give people the application (client) ID and your tenant ID. On Brolga's
   **Settings**, **Accounts**, under **Advanced**, they enter both before signing in.
   Brolga remembers them.

## Where a person's data is kept

- **Mail, calendar and contacts** are copied to the person's own computer, in
  SQLite databases under their user data folder
  (`~/.local/share/brolga` on Linux, the application data folder on Windows
  and macOS). They are not encrypted by Brolga: use full-disk encryption, as
  for any mail client that works offline.
- **Sign-in tokens:** the refresh token is kept only in the operating
  system's secret store (Secret Service, Credential Manager or Keychain).
  Access tokens are held in memory.
- **Nothing is sent to anyone else.** There is no telemetry and no crash
  reporting.

## Removing it

A person removes their account under **Settings**, **Accounts**, which forgets the
sign-in and deletes the local copy. To revoke it from your side, remove
Brolga under **Enterprise applications**, or revoke the person's sessions.

## AI agents

Brolga can be driven by an AI agent on the person's own computer, through a
local MCP server. An agent can only read unless the person starts it with
more, and sending asks the person first. [What an agent may do](mcp.md) has
the detail. An agent has no access of its own to Microsoft 365: it uses the
signed-in person's, through Brolga.
