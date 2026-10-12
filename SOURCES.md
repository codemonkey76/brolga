# Sources

Brolga is built only from public documentation and open standards. Every
outside document that informed the code is listed here, with what it was used
for. If you read something to write code, add it before the code is merged.

Nothing from Microsoft's own clients may be used: no decompiled or inspected
binaries, no captured traffic, no undocumented endpoints, no icons, sounds or
interface text. The full rules are in [AGENTS.md](AGENTS.md).

## Microsoft Graph (Microsoft Learn)

| Document | URL | Used for |
|---|---|---|
| Get a user | https://learn.microsoft.com/graph/api/user-get | `GET /me` and the properties returned by default, in the fake Graph server |
| Microsoft Graph throttling guidance | https://learn.microsoft.com/graph/throttling | The 429 status, the `Retry-After` header and the throttled error body, in the fake Graph server |
| Microsoft Graph error responses and resource types | https://learn.microsoft.com/graph/errors | The shape of an error body, and the meaning of 401, 404 and 503, in the fake Graph server |
| Use delta query to track changes in Microsoft Graph data | https://learn.microsoft.com/graph/delta-query-overview | nextLink and deltaLink, `@removed`, replays, the 410 reset and `syncStateNotFound`, in `crates/graph` and `crates/sync` |
| Get incremental changes to messages in a folder | https://learn.microsoft.com/graph/delta-query-messages | The message sync round, in `crates/sync` |
| message: delta | https://learn.microsoft.com/graph/api/message-delta | `/me/mailFolders/{id}/messages/delta`, `$select`, `$orderby=receivedDateTime desc`, `Prefer: odata.maxpagesize`. It says `$expand` is taken for messages; Microsoft 365 refused an extended property asked for that way (400), so none is |
| Outlook extended properties overview | https://learn.microsoft.com/graph/api/resources/extended-properties-overview | Naming a property Graph has no name for by its type and property tag: `Integer 0x1081` |
| Get singleValueLegacyExtendedProperty | https://learn.microsoft.com/graph/api/singlevaluelegacyextendedproperty-get | `$expand=singleValueExtendedProperties($filter=id eq '…')` on messages, the filter `singleValueExtendedProperties/Any(ep: ep/id eq '…' and cast(ep/value, Edm.Int32) ge …)` that finds the messages that have a property, and the `id` and `value` it answers with |
| mailFolder: delta | https://learn.microsoft.com/graph/api/mailfolder-delta | `/me/mailFolders/delta` |
| mailFolder resource type | https://learn.microsoft.com/graph/api/resources/mailfolder | Folder properties and the well-known folder names. v1.0 has no property naming a folder's role, so each is fetched by name |
| message resource type | https://learn.microsoft.com/graph/api/resources/message | Message properties, with followupFlag, recipient, emailAddress and itemBody |
| Get message | https://learn.microsoft.com/graph/api/message-get | Fetching a body with `$select=body` |
| List attachments | https://learn.microsoft.com/graph/api/message-list-attachments | `GET /me/messages/{id}/attachments`, with `$select` so the list comes without the files' contents |
| Get attachment | https://learn.microsoft.com/graph/api/attachment-get | `…/attachments/{id}` for a picture within a body, with its `contentId` and `contentBytes`; `…/attachments/{id}/$value` for the raw contents; MIME for an attached message, event or contact; 405 for a reference attachment |
| fileAttachment resource type | https://learn.microsoft.com/graph/api/resources/fileattachment | Attachment properties, and `size` being in bytes |
| Update message | https://learn.microsoft.com/graph/api/message-update | `PATCH /me/messages/{id}` needs `Mail.ReadWrite`; `isRead`, `flag` and `categories` may be changed on any message |
| message: move | https://learn.microsoft.com/graph/api/message-move | `POST /me/messages/{id}/move` with `destinationId` as a folder id or well-known name; 201 and the message. Deleting and archiving are moves |
| Create message | https://learn.microsoft.com/graph/api/user-post-messages | `POST /me/messages` makes a draft; 201 and the message |
| message: createReply, createReplyAll, createForward | https://learn.microsoft.com/graph/api/message-createreply | A draft answering or forwarding a message, with the original quoted in its body; 201 and the message |
| message: send | https://learn.microsoft.com/graph/api/message-send | `POST /me/messages/{id}/send` needs `Mail.Send`, takes no body, answers 202 and saves to Sent Items. It says nothing of sending twice, of a way to name a request so a repeat is known, or of what a draft looks like between being accepted and being sent |
| Add attachment | https://learn.microsoft.com/graph/api/message-post-attachments | `POST /me/messages/{id}/attachments` with a fileAttachment, for files under 3 MB; `201` and the attachment, whose `id` is kept so the file can be taken off again. Its example lists a `size` larger than the file's bytes |
| Delete attachment | https://learn.microsoft.com/graph/api/attachment-delete | `DELETE /me/messages/{id}/attachments/{id}` needs `Mail.ReadWrite`, takes no body and answers `204`: a file taken off a draft here is taken off the server's copy before it is sent |
| attachment: createUploadSession | https://learn.microsoft.com/graph/api/attachment-createuploadsession | An upload session for a file of 3 MB to 150 MB, from an `AttachmentItem` of type, name and size |
| Attach large files to Outlook messages or events | https://learn.microsoft.com/graph/outlook-large-attachments | `PUT` to the session's `uploadUrl` in ranges of under 4 MB with `Content-Range`, in order, with no Authorization header; 200 until the last piece, then 201 with a `Location` header whose address ends `Attachments('{id}')`, the id to use with Graph afterwards. Its example lists a file of 3,483,322 bytes with a `size` of 3,640,066, so a file is not looked for on a draft by a size equal to its own |
| Get and update user mailbox settings | https://learn.microsoft.com/graph/api/user-update-mailboxsettings | `GET /me/mailboxSettings/automaticRepliesSetting` and `PATCH /me/mailboxSettings`, which need `MailboxSettings.ReadWrite` |
| automaticRepliesSetting resource type | https://learn.microsoft.com/graph/api/resources/automaticrepliessetting | Status, audience, the two messages and the scheduled times of automatic replies |
| message: permanentDelete | https://learn.microsoft.com/graph/api/message-permanentdelete | `POST …/messages/{id}/permanentDelete`, answered 204; the page shows it under `/users/{id}`, for which `/me` is the signed-in user |
| List masterCategories, and outlookCategory resource type | https://learn.microsoft.com/graph/api/outlookuser-list-mastercategories | `GET /me/outlook/masterCategories`, each a `displayName` and a `color` preset, and the colour Outlook gives each preset |
| Create Outlook category, Update outlookCategory, Delete outlookCategory | https://learn.microsoft.com/graph/api/outlookuser-post-mastercategories | `POST /me/outlook/masterCategories` with `displayName` and `color`, `201`; `PATCH /me/outlook/masterCategories/{id}` changes `color` only, a name being fixed once made; `DELETE`, `204`; need `MailboxSettings.ReadWrite`. What becomes of a deleted category on the items that carry it is not documented, and nothing is assumed about it |
| List rules, Create rule, Update rule, Delete rule; messageRule, messageRulePredicates, messageRuleActions and sizeRange resource types | https://learn.microsoft.com/graph/api/mailfolder-list-messagerules | `GET` and `POST /me/mailFolders/inbox/messageRules` (`201`; `displayName`, `sequence` and `actions` required), `PATCH` and `DELETE …/messageRules/{id}` (`200`, `204`); need `MailboxSettings.Read` to read and `MailboxSettings.ReadWrite` to change. Every condition and action the commands take is one named on the resource pages; a rule the server marks `isReadOnly` is shown and not changed. Whether a `PATCH` of `conditions` or `actions` replaces the part or merges into it is not said, so the whole part is always sent |
| followupFlag resource type | https://learn.microsoft.com/graph/api/resources/followupflag | A flag's `dueDateTime`, which must be set together with `startDateTime`; both are `dateTimeTimeZone`, so a day is sent as its first moment in the writer's zone. It does not say how a date is taken off a flag: the outbox sends both as `null` |
| Get MIME content of a message | https://learn.microsoft.com/graph/outlook-get-mime-message | `GET /me/messages/{id}/$value`, the message whole as a `.eml` file holds it, for forwarding as an attachment |
| Get immutable identifiers for Outlook resources | https://learn.microsoft.com/graph/outlook-immutable-id | `Prefer: IdType="ImmutableId"` on every request, so an id survives a move between folders |
| event: accept, event: tentativelyAccept, event: decline | https://learn.microsoft.com/graph/api/event-accept | `POST /me/events/{id}/accept`, `/tentativelyAccept` and `/decline` with `{comment, sendResponse}`; `202 Accepted` and no body; `sendResponse` defaults to true; needs `Calendars.ReadWrite`. Brolga does not repeat the request blindly: it reads the event's `responseStatus` back first |
| Get event | https://learn.microsoft.com/graph/api/event-get | `GET /me/events/{id}` of a series master gives its `recurrence`; read before a series is changed from one occurrence on |
| Series, changed from one occurrence on | https://learn.microsoft.com/graph/api/resources/recurrencerange | Graph has no request for "this and following". Brolga ends the old series the day before (`range.type` `endDate`) and makes a new series from that day, the new one first so nothing is lost if a step fails; a `numbered` range carries on with the count it had left. `range.startDate` must be the day the event starts |
| event: cancel | https://learn.microsoft.com/graph/api/event-cancel | `POST /me/events/{id}/cancel` with `{comment}`; `202 Accepted`; for the organiser only (400 otherwise); the event leaves the calendar and those invited are told. Not repeated blindly: the event is looked for first |
| event and attendee resource types | https://learn.microsoft.com/graph/api/resources/event | `attendees` are `{emailAddress, type}` with `type` `required`, `optional` or `resource`, and carry each one's `status.response`; an update of `attendees` alone is told only to those it changes; `isOnlineMeeting` with `onlineMeetingProvider` `teamsForBusiness` makes a Teams meeting and cannot be turned off again |
| calendar: getSchedule | https://learn.microsoft.com/graph/api/calendar-getschedule | `POST /me/calendar/getSchedule` with `{schedules, startTime, endTime, availabilityViewInterval}`; for each address an `availabilityView` of one digit a slot (0 free or working elsewhere, 1 tentative, 2 busy, 3 out of office), its `scheduleItems`, or an `error`; allowed by `Calendars.ReadWrite`. Times everyone is free are worked out here from the views: `findMeetingTimes` needs `Calendars.Read.Shared`, which is not asked for yet |
| Create calendar, Delete calendar | https://learn.microsoft.com/graph/api/user-post-calendars | `POST /me/calendars` with `{name}`, `201` and the calendar; `DELETE /me/calendars/{id}`, `204` |
| event: dismissReminder, event: snoozeReminder | https://learn.microsoft.com/graph/api/event-dismissreminder | `POST /me/events/{id}/dismissReminder` with no body, and `/snoozeReminder` with `{newReminderTime}`; `200`. Brolga tells the server when a reminder is dealt with here. Nothing documented says a reminder dismissed on another device can be read back, so that direction is not built |
| Propose new meeting times | https://learn.microsoft.com/graph/outlook-calendar-meeting-proposals | `proposedNewTime` `{start, end}` goes with `tentativelyAccept` or `decline`, needs `sendResponse` true and the event's `allowNewTimeProposals`, else 400; an organiser reads each attendee's `proposedNewTime` and takes one by changing the event's time |
| List place objects, room resource type | https://learn.microsoft.com/graph/api/place-list | `GET /places/microsoft.graph.room`, up to 100 a page; each room has `displayName`, `emailAddress`, `capacity`, `building`, `floorNumber`; needs `Place.Read.All`, which the permissions reference marks as needing admin consent when delegated, so it is not asked for at sign-in: rooms are listed only where an administrator has approved it. Whether a room is free is asked of its own calendar with getSchedule; it is booked by inviting it as a `resource` |
| Microsoft Graph permissions reference | https://learn.microsoft.com/graph/permissions-reference | Which delegated permissions a person can consent to and which need an administrator: of those Brolga uses, only `Place.Read.All` needs one. It lists no `GroupMember.ReadBasic.All`, and reading a group's members needs admin consent, so distribution lists are not expanded |
| Grant tenant-wide admin consent to an application | https://learn.microsoft.com/entra/identity/enterprise-apps/grant-admin-consent | The address an administrator opens to approve an app for the whole organisation, `https://login.microsoftonline.com/{organization}/adminconsent?client_id={client-id}`, given in `docs/admins.md` |
| Publisher verification | https://learn.microsoft.com/entra/identity-platform/publisher-verification-overview | A verified publisher needs a verified Microsoft AI Cloud Partner Program account and a custom publisher domain, so an app published by an individual stays unverified; said in `docs/admins.md` |
| user: findMeetingTimes | https://learn.microsoft.com/graph/api/user-findmeetingtimes | `POST /me/findMeetingTimes` with `attendees`, `timeConstraint` (`activityDomain` `work` or `unrestricted`, `timeSlots`), `meetingDuration` as ISO 8601 and `maxCandidates`; gives `meetingTimeSuggestions` with a `confidence` each, or an `emptySuggestionsReason`; needs `Calendars.Read.Shared` |
| List contacts, List contactFolders, contact resource type | https://learn.microsoft.com/graph/api/user-list-contacts | `GET /me/contacts` for the main folder and `GET /me/contactFolders/{id}/contacts` for each other folder, paged; needs `Contacts.Read`. Each folder is read whole: delta is documented only for a folder named by id, and the main folder is not listed among them |
| Create, Update, Delete contact | https://learn.microsoft.com/graph/api/user-post-contacts | `POST /me/contacts` or `/me/contactFolders/{id}/contacts`, `201`; `PATCH /me/contacts/{id}` with only what changes; `DELETE /me/contacts/{id}`, `204`; need `Contacts.ReadWrite` |
| List people | https://learn.microsoft.com/graph/api/user-list-people | `GET /me/people?$search="typed"`: the people most to do with the owner, with title, department and phones; needs `People.Read`. `$search` matches a name or an alias, so a person is looked up by what comes before the @ of their address to learn their title and department |
| List users | https://learn.microsoft.com/graph/api/user-list | `GET /users?$filter=startswith(displayName,'typed') or …&$select=…`: anyone in the organisation by the start of a name or address; `User.ReadBasic.All` gives names and addresses only. Neither directory answer is kept on this computer |
| Another user's mailbox | https://learn.microsoft.com/graph/api/user-list-mailfolders | Every mail request has a `/users/{id or userPrincipalName}/…` form beside `/me/…`; with `Mail.Read.Shared` or `Mail.ReadWrite.Shared` it reaches a mailbox the signed-in person has been given, and with `Mail.Send.Shared` sends from it. Graph does not list the mailboxes someone has been given, so each is added by address |
| Get shared or delegated Outlook calendar and its events; Create Outlook events in a shared or delegated calendar | https://learn.microsoft.com/graph/outlook-get-shared-events-calendars | A calendar someone has shared or delegated, once accepted, is among `GET /me/calendars` with its `owner` and `canEdit`, and is read and written there like one's own; reading needs `Calendars.Read.Shared` and writing `Calendars.ReadWrite.Shared`. The calendar of a mailbox one has been given is reached directly at `/users/{address}/calendar…`, with ids that belong to that mailbox only. Change notifications are not available on either, which this client does not use |
| List, Create, Update and Delete calendarPermission; calendarPermission resource type | https://learn.microsoft.com/graph/api/calendar-list-calendarpermissions | `GET` and `POST /me/calendar/calendarPermissions` (`200` and the permission), `PATCH …/{id}` with `role` alone, `DELETE …/{id}` (`204`); `Calendars.ReadWrite`. Only the owner may read or change them, only the main calendar's are documented, and each says in `allowedRoles` what its person may be given. Whether the person is told of a new share is not said, and nothing is claimed about it |
| Send from another mailbox | https://learn.microsoft.com/graph/outlook-send-mail-from-other-user | A draft's `from` may be set to a mailbox the sender may send as or on behalf of; which of the two it goes as is the server's, by what was granted, and without either the send is refused. A message sent this way from one's own mailbox is kept in one's own Sent Items; one written in the shared mailbox (`/users/{address}/messages`) is kept in that mailbox's. Whether a copy also goes to the shared mailbox's Sent Items is a setting of the tenant |
| Create event | https://learn.microsoft.com/graph/api/user-post-events | `POST /me/calendars/{id}/events` with an event; `201 Created` and the event; `start` and `end` are `dateTimeTimeZone`; a `transactionId` lets the server make one event however often the request arrives, so Brolga gives each its own and may retry |
| Update event | https://learn.microsoft.com/graph/api/event-update | `PATCH /me/events/{id}` with only what changes; `200 OK` and the event. The body is sent only when the words were changed, since rewriting it can remove an online meeting's joining details |
| Delete event | https://learn.microsoft.com/graph/api/event-delete | `DELETE /me/events/{id}`; `204`. Deleting a meeting one organised sends cancellations |
| dateTimeTimeZone resource type | https://learn.microsoft.com/graph/api/resources/datetimetimezone | `timeZone` takes Windows' zone names (and some others), so zones are sent by their Windows name, found from the CLDR table read backwards, for events and for the day a flag is due |
| event: delta | https://learn.microsoft.com/graph/api/event-delta | `GET /me/calendarView/delta?startDateTime=…&endDateTime=…` for the default calendar: needs `Calendars.Read`, takes no `$select`, and names as removed events outside the window as well as deleted ones |
| List calendars, and calendar resource type | https://learn.microsoft.com/graph/api/user-list-calendars | `GET /me/calendars`, and a calendar's name, colour, `isDefaultCalendar`, `canEdit`, owner and its `calendarView` |
| event resource type | https://learn.microsoft.com/graph/api/resources/event | An event's properties: start and end in UTC by default, `isAllDay`, `type`, `seriesMasterId`, `showAs`, reminder, `onlineMeeting`, `changeKey` |
| eventMessage resource type, and Get eventMessage | https://learn.microsoft.com/graph/api/eventmessage-get | A message about a meeting is an `eventMessage`, named by `@odata.type`; `meetingMessageType` says which kind; `GET /me/messages/{id}?$expand=microsoft.graph.eventMessage/event` gives the event in the calendar it is about |
| recurrencePattern and recurrenceRange resource types | https://learn.microsoft.com/graph/api/resources/recurrencepattern | The six ways an event repeats, what each needs, and the three ways a series ends, in `crates/calendar` |

## Microsoft identity platform (Microsoft Learn)

| Document | URL | Used for |
|---|---|---|
| Microsoft identity platform and OAuth 2.0 authorization code flow | https://learn.microsoft.com/entra/identity-platform/v2-oauth2-auth-code-flow | The authorize and token endpoints, their parameters, PKCE, refresh, and the error body, in `crates/auth` |
| Microsoft identity platform and the OAuth 2.0 device authorization grant flow | https://learn.microsoft.com/entra/identity-platform/v2-oauth2-device-code | The devicecode endpoint, polling and its errors, in `crates/auth` |
| Redirect URI (reply URL) outline and restrictions | https://learn.microsoft.com/entra/identity-platform/reply-url | `http://localhost` as the redirect, and the port being ignored when it is matched |
| Microsoft Entra authentication and authorization error codes | https://learn.microsoft.com/entra/identity-platform/reference-error-codes | Which AADSTS code means which plain-language problem, in `crates/auth/src/error.rs` |
| ID token claims reference | https://learn.microsoft.com/entra/identity-platform/id-token-claims-reference | `tid`, `oid`, `preferred_username` and `name`, to label an account |
| Claims challenges, claims requests, and client capabilities | https://learn.microsoft.com/entra/identity-platform/claims-challenge | Reading a claims challenge from a 401's `WWW-Authenticate` header |

## Model Context Protocol

| Document | URL | Used for |
|---|---|---|
| MCP Rust SDK (`rmcp` 3.5), source and docs | https://docs.rs/rmcp | The MCP server: tool listing, tool calls, the stdio transport |

## Open standards

| Document | URL | Used for |
|---|---|---|
| RFC 7636, Proof Key for Code Exchange | https://www.rfc-editor.org/rfc/rfc7636 | The code verifier and its S256 challenge, with the appendix B example as a test |
| RFC 8252, OAuth 2.0 for Native Apps | https://www.rfc-editor.org/rfc/rfc8252 | Signing in through the system browser with a loopback redirect |
| RFC 8628, OAuth 2.0 Device Authorization Grant | https://www.rfc-editor.org/rfc/rfc8628 | Waiting five seconds longer after `slow_down` (section 3.5) |
| RFC 6350, vCard Format Specification | https://www.rfc-editor.org/rfc/rfc6350 | Which properties of a card become which fields of a contact (`FN`, `N`, `ORG`, `TITLE`, `EMAIL`, `TEL` and its `TYPE`, `BDAY`, `NOTE`, `CATEGORIES`), read in any version and written as 3.0 |
| MS-PST, Outlook Personal Folders (.pst) File Format | https://learn.microsoft.com/openspecs/office_file_formats/ms-pst/141923d5-15ab-4ef1-a524-6dce75aae546 | The format a PST or OST file is in. It is read by Microsoft's own open-source parser written from this document (the `outlook-pst` crate, MIT), through a file opened for reading alone |
| MS-OXPROPS, Exchange Server Protocols Master Property List | https://learn.microsoft.com/openspecs/exchange_server_protocols/ms-oxprops/f6ab1613-aefe-447d-a49c-18217230b148 | Which numbered property of a message, recipient or attachment in a PST holds what: subject, sender, times, flags, importance, the plain and HTML bodies and their code page, attachment name, type, content id and data |
| MS-OXOMSG, Email Object Protocol: PidTagLastVerbExecuted | https://learn.microsoft.com/openspecs/exchange_server_protocols/ms-oxomsg/87a8b6b8-59a4-4859-9dcd-8b0f36e3d729 | What was last done with a message, property 0x1081: 102 a reply to its sender, 103 a reply to all, 104 forwarding it |
| OpenID Connect Core 1.0 | https://openid.net/specs/openid-connect-core-1_0.html | Section 3.1.3.7: an ID token received straight from the token endpoint over TLS need not have its signature checked |

## Tauri

| Document | URL | Used for |
|---|---|---|
| Updater plugin | https://v2.tauri.app/plugin/updater/ | The signing key and its environment variables, `createUpdaterArtifacts`, the `latest.json` an installed copy reads and its platform names, and checking for, fetching and installing an update |

## Data

| Data | URL | Used for |
|---|---|---|
| Unicode CLDR `windowsZones.xml` | https://github.com/unicode-org/cldr/blob/main/common/supplemental/windowsZones.xml | Which IANA time zone a Windows zone name means. Shipped unchanged in `crates/calendar/data`, under the Unicode License v3 |
| IANA Time Zone Database, through the `jiff` crate | https://www.iana.org/time-zones | The rules of every zone, built in so they are the same on every computer |

## Icons

The icons are Lucide's (https://lucide.dev), used under the ISC licence
through the `@lucide/svelte` package. None is taken from, or drawn after,
any Microsoft product.

## Data

| Source | URL | Used for |
| --- | --- | --- |
| `holidays` (vacanza), MIT | https://github.com/vacanza/holidays | The public holidays of each country and its regions for a span of years, worked out by release 0.106 and written to `crates/calendar/data/holidays.json` by `tools/holidays/generate.py`. The library is not a dependency and is not shipped |
| Nager.Holidays terms of service | https://nagerholidays.com/legal/termsofservice | Looked at as a source of holidays and not used: its web service is for private or non-profit projects, and its days for Australia's states were less complete |

## Libraries

Open source libraries are dependencies, listed in `Cargo.toml` and
`app/package.json`. Their licences are checked by `cargo deny` on every merge.
