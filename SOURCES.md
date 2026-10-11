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
| message: delta | https://learn.microsoft.com/graph/api/message-delta | `/me/mailFolders/{id}/messages/delta`, `$select`, `$orderby=receivedDateTime desc`, `Prefer: odata.maxpagesize` |
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
| message: send | https://learn.microsoft.com/graph/api/message-send | `POST /me/messages/{id}/send` needs `Mail.Send`, takes no body, answers 202 and saves to Sent Items |
| Add attachment | https://learn.microsoft.com/graph/api/message-post-attachments | `POST /me/messages/{id}/attachments` with a fileAttachment, for files under 3 MB |
| attachment: createUploadSession | https://learn.microsoft.com/graph/api/attachment-createuploadsession | An upload session for a file of 3 MB to 150 MB, from an `AttachmentItem` of type, name and size |
| Attach large files to Outlook messages or events | https://learn.microsoft.com/graph/outlook-large-attachments | `PUT` to the session's `uploadUrl` in ranges of under 4 MB with `Content-Range`, in order, with no Authorization header; 200 until the last piece, then 201 |
| Get and update user mailbox settings | https://learn.microsoft.com/graph/api/user-update-mailboxsettings | `GET /me/mailboxSettings/automaticRepliesSetting` and `PATCH /me/mailboxSettings`, which need `MailboxSettings.ReadWrite` |
| automaticRepliesSetting resource type | https://learn.microsoft.com/graph/api/resources/automaticrepliessetting | Status, audience, the two messages and the scheduled times of automatic replies |
| message: permanentDelete | https://learn.microsoft.com/graph/api/message-permanentdelete | `POST …/messages/{id}/permanentDelete`, answered 204; the page shows it under `/users/{id}`, for which `/me` is the signed-in user |
| List masterCategories, and outlookCategory resource type | https://learn.microsoft.com/graph/api/outlookuser-list-mastercategories | `GET /me/outlook/masterCategories`, each a `displayName` and a `color` preset, and the colour Outlook gives each preset |
| followupFlag resource type | https://learn.microsoft.com/graph/api/resources/followupflag | A flag's `dueDateTime`, which must be set together with `startDateTime` |
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
| List place objects, room resource type | https://learn.microsoft.com/graph/api/place-list | `GET /places/microsoft.graph.room`, up to 100 a page; each room has `displayName`, `emailAddress`, `capacity`, `building`, `floorNumber`; needs `Place.Read.All`. Whether a room is free is asked of its own calendar with getSchedule; it is booked by inviting it as a `resource` |
| user: findMeetingTimes | https://learn.microsoft.com/graph/api/user-findmeetingtimes | `POST /me/findMeetingTimes` with `attendees`, `timeConstraint` (`activityDomain` `work` or `unrestricted`, `timeSlots`), `meetingDuration` as ISO 8601 and `maxCandidates`; gives `meetingTimeSuggestions` with a `confidence` each, or an `emptySuggestionsReason`; needs `Calendars.Read.Shared` |
| List contacts, List contactFolders, contact resource type | https://learn.microsoft.com/graph/api/user-list-contacts | `GET /me/contacts` for the main folder and `GET /me/contactFolders/{id}/contacts` for each other folder, paged; needs `Contacts.Read`. Each folder is read whole: delta is documented only for a folder named by id, and the main folder is not listed among them |
| Create, Update, Delete contact | https://learn.microsoft.com/graph/api/user-post-contacts | `POST /me/contacts` or `/me/contactFolders/{id}/contacts`, `201`; `PATCH /me/contacts/{id}` with only what changes; `DELETE /me/contacts/{id}`, `204`; need `Contacts.ReadWrite` |
| List people | https://learn.microsoft.com/graph/api/user-list-people | `GET /me/people?$search="typed"`: the people most to do with the owner, with title, department and phones; needs `People.Read` |
| List users | https://learn.microsoft.com/graph/api/user-list | `GET /users?$filter=startswith(displayName,'typed') or …&$select=…`: anyone in the organisation by the start of a name or address; `User.ReadBasic.All` gives names and addresses only. Neither directory answer is kept on this computer |
| Create event | https://learn.microsoft.com/graph/api/user-post-events | `POST /me/calendars/{id}/events` with an event; `201 Created` and the event; `start` and `end` are `dateTimeTimeZone`; a `transactionId` lets the server make one event however often the request arrives, so Brolga gives each its own and may retry |
| Update event | https://learn.microsoft.com/graph/api/event-update | `PATCH /me/events/{id}` with only what changes; `200 OK` and the event. The body is sent only when the words were changed, since rewriting it can remove an online meeting's joining details |
| Delete event | https://learn.microsoft.com/graph/api/event-delete | `DELETE /me/events/{id}`; `204`. Deleting a meeting one organised sends cancellations |
| dateTimeTimeZone resource type | https://learn.microsoft.com/graph/api/resources/datetimetimezone | `timeZone` takes Windows' zone names (and some others), so zones are sent by their Windows name, found from the CLDR table read backwards |
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
| OpenID Connect Core 1.0 | https://openid.net/specs/openid-connect-core-1_0.html | Section 3.1.3.7: an ID token received straight from the token endpoint over TLS need not have its signature checked |

## Data

| Data | URL | Used for |
|---|---|---|
| Unicode CLDR `windowsZones.xml` | https://github.com/unicode-org/cldr/blob/main/common/supplemental/windowsZones.xml | Which IANA time zone a Windows zone name means. Shipped unchanged in `crates/calendar/data`, under the Unicode License v3 |
| IANA Time Zone Database, through the `jiff` crate | https://www.iana.org/time-zones | The rules of every zone, built in so they are the same on every computer |

## Icons

The icons are Lucide's (https://lucide.dev), used under the ISC licence
through the `@lucide/svelte` package. None is taken from, or drawn after,
any Microsoft product.

## Libraries

Open source libraries are dependencies, listed in `Cargo.toml` and
`app/package.json`. Their licences are checked by `cargo deny` on every merge.
