import type { Page } from "@playwright/test";

/** An account as the core's commands give it out. The people are made up. */
export interface FakeAccount {
    id: string;
    username: string;
    display_name: string | null;
    tenant_id: string;
    needs_sign_in: boolean;
    added_at: string;
}

export const robin: FakeAccount = {
    id: "tenant-1.robin",
    username: "robin@example.test",
    display_name: "Robin Example",
    tenant_id: "tenant-1",
    needs_sign_in: false,
    added_at: "2026-10-01T09:00:00Z",
};

export const sam: FakeAccount = {
    id: "tenant-2.sam",
    username: "sam.taylor@northwind.example",
    display_name: "Sam Taylor",
    tenant_id: "tenant-2",
    needs_sign_in: true,
    added_at: "2026-10-03T14:30:00Z",
};

/** A message as the core's commands give it out, with the body it opens to. */
export interface FakeMail {
    id: string;
    folder_id: string;
    subject: string;
    preview: string;
    from: { name: string | null; address: string | null };
    received_at: string;
    is_read: boolean;
    has_attachments: boolean;
    /** The conversation it belongs to. Left out, it is one of its own. */
    conversation?: string;
    /** A draft, in the mailbox's Drafts folder. */
    is_draft?: boolean;
    /** What it says of a meeting, if it is about one. */
    meeting?: Record<string, unknown>;
    attachments?: { id: string; name: string; content_type: string; size: number; is_inline: boolean; kind: string }[];
    /** HTML as the core would hand it over: already cleaned. */
    body: string;
    /** How many remote pictures the core left out of `body`. */
    blocked?: number;
    /** The body once the pictures have been asked for. */
    bodyWithPictures?: string;
}

const person = (name: string) => ({ name, address: `${name.toLowerCase().replace(/ /g, ".")}@example.test` });

/** A small, made-up mailbox. */
export const inbox: FakeMail[] = [
    {
        id: "m1",
        folder_id: "inbox",
        subject: "Budget review moved to Thursday",
        preview: "Hi Robin, the room is booked for 2 pm on Thursday instead. Can you bring the Q3 numbers?",
        from: person("Sam Taylor"),
        received_at: "2026-10-09T03:15:00Z",
        is_read: false,
        has_attachments: false,
        body: '<p>Hi Robin,</p><p>The room is booked for <strong>2 pm on Thursday</strong> instead. Can you bring the Q3 numbers?</p><p>The agenda is <a href="https://example.test/agenda" rel="noopener noreferrer">here</a>.</p><p>Sam</p>',
    },
    {
        id: "m2",
        folder_id: "inbox",
        subject: "Your October newsletter",
        preview: "This month: five ways to plan a quieter week, and the autumn timetable.",
        from: person("Northwind News"),
        received_at: "2026-10-08T22:40:00Z",
        is_read: false,
        has_attachments: false,
        body: "<h2>October</h2><p>This month: five ways to plan a quieter week, and the autumn timetable.</p>",
        blocked: 3,
        bodyWithPictures:
            '<h2>October</h2><p><img alt="Autumn" width="120" height="60" src="data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg=="></p><p>This month: five ways to plan a quieter week, and the autumn timetable.</p>',
    },
    {
        id: "m3",
        folder_id: "inbox",
        subject: "Invoice 2041 attached",
        preview: "Please find attached the invoice for September. Payment is due in 14 days.",
        from: person("Alex Chen"),
        received_at: "2026-10-08T05:02:00Z",
        is_read: true,
        has_attachments: true,
        attachments: [
            { id: "a1", name: "Invoice 2041.pdf", content_type: "application/pdf", size: 184_320, is_inline: false, kind: "file" },
            { id: "a2", name: "logo.png", content_type: "image/png", size: 2_048, is_inline: true, kind: "file" },
            { id: "a3", name: "Statement.xlsx", content_type: "application/octet-stream", size: 0, is_inline: false, kind: "reference" },
        ],
        body: "<p>Please find attached the invoice for September. Payment is due in 14 days.</p>",
    },
    {
        id: "m4",
        folder_id: "inbox",
        subject: "Action needed: confirm your password",
        preview: "We noticed a sign-in from a new device. Confirm your password to keep your account.",
        from: { name: "IT Support", address: "support@account-security.example" },
        received_at: "2026-10-07T11:30:00Z",
        is_read: true,
        has_attachments: false,
        body: '<p>We noticed a sign-in from a new device.</p><p><a href="https://account-security.example/login" rel="noopener noreferrer">https://www.example.test/security</a></p>',
    },
];

/** A conversation: a question in the inbox, the answer in Sent Items, and the reply to that. */
export const lunch: FakeMail[] = [
    {
        id: "c1",
        folder_id: "inbox",
        conversation: "lunch",
        subject: "Lunch on Friday?",
        preview: "Are you free on Friday? The place by the harbour has reopened.",
        from: person("Jo Nguyen"),
        received_at: "2026-10-09T00:10:00Z",
        is_read: true,
        has_attachments: false,
        body: "<p>Are you free on Friday? The place by the harbour has reopened.</p>",
    },
    {
        id: "c2",
        folder_id: "sent",
        conversation: "lunch",
        subject: "RE: Lunch on Friday?",
        preview: "Yes. Half past twelve?",
        from: person("Robin Example"),
        received_at: "2026-10-09T01:00:00Z",
        is_read: true,
        has_attachments: false,
        body: "<p>Yes. Half past twelve?</p>",
    },
    {
        id: "c3",
        folder_id: "inbox",
        conversation: "lunch",
        subject: "RE: Lunch on Friday?",
        preview: "Booked for four of us. See you there.",
        from: person("Jo Nguyen"),
        received_at: "2026-10-09T04:30:00Z",
        is_read: false,
        has_attachments: false,
        body: "<p>Booked for four of us. See you there.</p>",
    },
];

/** A long folder, for scrolling. */
export const bulk = (count: number): FakeMail[] =>
    Array.from({ length: count }, (_, n) => ({
        id: `bulk-${n}`,
        folder_id: "archive",
        subject: `Archived message ${n + 1}`,
        preview: `This is the preview of archived message ${n + 1}.`,
        from: person(`Sender ${n % 50}`),
        received_at: new Date(Date.UTC(2026, 8, 30) - n * 3_600_000).toISOString(),
        is_read: true,
        has_attachments: false,
        body: `<p>Archived message ${n + 1}.</p>`,
    }));

export interface FakeCore {
    accounts?: FakeAccount[];
    mail?: FakeMail[];
    /** Mail that turns up at the second sync, as new mail does. */
    arrives?: FakeMail[];
    /** The words the next sync fails with. */
    syncFails?: string;
    /** What is in the calendar. */
    events?: FakeEvent[];
    /** What agents are waiting for the user's say-so on. */
    asking?: { id: number; client: string; command: string; args: Record<string, unknown> }[];
    /** Microsoft 365 cannot be reached. */
    offline?: boolean;
    /** Mailboxes other people have shared with an account. */
    shared?: { id: string; account_id: string; address: string; name: string }[];
    /** Personal contacts. */
    contacts?: Record<string, unknown>[];
    /** People of the organisation, for looking up. */
    directory?: { name: string; address: string; job_title?: string; department?: string }[];
    /** When people are busy, for the free and busy lookup: an address, and a digit for each half hour of the day from midnight. */
    busy?: Record<string, string>;
    /** What agents have asked for, the latest first. */
    agentLog?: { at: string; client: string; command: string; args: unknown; outcome: string; detail: string }[];
    /** Why automatic replies cannot be read, if they cannot. */
    repliesRefused?: string;
    /** The files the file dialog says were chosen. Left out, it is dismissed. */
    pickFiles?: string[];
    /** The sync is still going for as long as the test runs. */
    syncNeverEnds?: boolean;
    /** Why Microsoft 365 will not send anything, if it will not. */
    sendRefused?: string;
    /** Changes Microsoft 365 refused, to be told of after the next sync. */
    refused?: { message_id: string; subject: string; change: string; error: string }[];
    /** Where the save dialog says to save. Left out, the dialog is dismissed. */
    savePath?: string;
    /** Folders already pinned as favourites. */
    pinned?: string[];
    /** True when Brolga has no app registration of its own, as now. */
    needsClientId?: boolean;
    /** The account a sign-in adds, or the words it fails with. */
    signIn?: FakeAccount | { fails: string };
    /** Never finish a sign-in, so the waiting screen can be looked at. */
    signInHangs?: boolean;
}

/**
 * Stand in for the Rust core. The frontend reaches it through one Tauri
 * command, `execute_command`, so that is all there is to answer. Each command
 * run is kept on `window.ran` for a test to read back.
 */
export async function withCore(page: Page, core: FakeCore = {}) {
    await page.addInitScript((core) => {
        const accounts = [...(core.accounts ?? [])];
        const ran: { command: string; args: Record<string, unknown> }[] = [];
        const never = new Promise<never>(() => {});

        const signedIn = () => {
            const outcome = core.signIn;
            if (!outcome) throw { code: "failed", message: "This test did not say what signing in gives." };
            if ("fails" in outcome) throw { code: "failed", message: outcome.fails };
            const at = accounts.findIndex((a) => a.id === outcome.id);
            if (at >= 0) accounts[at] = outcome;
            else accounts.push(outcome);
            return { account: outcome };
        };

        const mail = (core.mail ?? []).map((m) => ({ ...m }));
        const flags = new Map<string, string>();
        const tags = new Map<string, string[]>();
        const dues = new Map<string, string | null>();
        /** What each action changed, by the id that undoes it. */
        const actions = new Map<string, () => void>();
        const refused = [...(core.refused ?? [])];
        /** Drafts as the core keeps them, and what has been sent. */
        const drafts: Record<string, unknown>[] = [];
        const sent: Record<string, unknown>[] = [];
        const checked = (people: unknown) => {
            for (const each of (people as { address: string }[] | undefined) ?? []) {
                if (!/^[^@\s<>,;]+@[^@\s<>,;]+\.[^@\s<>,;]+$/.test(each.address)) throw { code: "failed", message: `\`${each.address}\` is not an email address.` };
            }
            return people;
        };
        const draft = (id: unknown) => drafts.find((each) => each.id === id);
        let signature = { text: "", on_replies: false };
        const notified: { title: string; body: string }[] = [];
        /** What has been done about each reminder: dismissed, or put off until a time. */
        const done = new Map<unknown, number>();
        const diaries = [
            { id: "main", name: "Calendar", color: "auto", hex_color: "", is_default: true, can_edit: true, shown: true },
            { id: "family", name: "Family", color: "auto", hex_color: "#b3305f", is_default: false, can_edit: true, shown: true },
        ];
        const event = (e: NonNullable<typeof core.events>[number]) => ({
            id: e.id,
            calendar_id: e.calendar_id ?? "main",
            subject: e.subject,
            preview: (e.description ?? "").slice(0, 40),
            location: e.location ?? "",
            start: e.start,
            end: e.end,
            is_all_day: e.is_all_day ?? false,
            time_zone: e.time_zone ?? "Australia/Brisbane",
            show_as: e.show_as ?? "busy",
            sensitivity: e.sensitivity ?? "normal",
            is_cancelled: e.is_cancelled ?? false,
            is_organizer: e.mine ?? false,
            organizer: e.mine ? { name: "Robin Example", address: "robin@example.test" } : { name: "Sam Taylor", address: "sam.taylor@example.test" },
            response: e.mine ? "organizer" : (e.response ?? "accepted"),
            kind: e.series_id ? "occurrence" : "singleInstance",
            series_id: e.series_id ?? null,
            categories: [],
            reminder_minutes: e.reminder_minutes === undefined ? 15 : e.reminder_minutes,
            join_url: e.join_url ?? null,
            attendees: e.attendees ?? [],
        });
        /** Write what a command gives into an event, as the core does: what is left out is left. */
        const written = (e: NonNullable<typeof core.events>[number], args: Record<string, unknown>) => {
            if (typeof args.subject === "string") e.subject = args.subject;
            if (typeof args.calendar_id === "string") e.calendar_id = args.calendar_id;
            if (typeof args.location === "string") e.location = args.location;
            if (typeof args.description === "string") e.description = args.description;
            if (typeof args.show_as === "string") e.show_as = args.show_as;
            if (typeof args.is_private === "boolean") e.sensitivity = args.is_private ? "private" : "normal";
            if ("reminder_minutes" in args) e.reminder_minutes = args.reminder_minutes as number | null;
            if (Array.isArray(args.attendees)) {
                e.attendees = (args.attendees as { address: string; name?: string; kind?: string }[]).map((each) => ({
                    name: each.name ?? each.address,
                    address: each.address,
                    kind: each.kind ?? "required",
                    response: e.attendees?.find((had) => had.address === each.address)?.response ?? "none",
                }));
            }
            if (args.online_meeting === true) e.join_url = "https://teams.example.test/meet/1";
            if (typeof args.start === "string" && typeof args.end === "string") {
                e.is_all_day = args.is_all_day === true;
                // The page is in the zone the times are given in: a time by the clock is read as its own.
                e.start = e.is_all_day ? `${args.start}T00:00:00Z` : new Date(args.start).toISOString();
                e.end = e.is_all_day ? `${args.end}T00:00:00Z` : new Date(args.end).toISOString();
                if (typeof args.time_zone === "string") e.time_zone = args.time_zone;
            }
        };
        /** A contact with what a command gives written into it, as the core does. */
        const contactOf = (was: Record<string, unknown>, args: Record<string, unknown>) => {
            const all: Record<string, unknown> = {
                given_name: "", surname: "", nickname: "", company: "", job_title: "", department: "", office: "",
                emails: [], business_phones: [], home_phones: [], mobile: "", notes: "", birthday: null, ...was,
            };
            for (const name of ["given_name", "surname", "company", "job_title", "department", "mobile", "notes", "business_phones", "home_phones"]) {
                if (name in args) all[name] = args[name];
            }
            if (Array.isArray(args.emails)) all.emails = (args.emails as string[]).map((address) => ({ address, name: address }));
            all.name = `${all.given_name} ${all.surname}`.trim() || String(all.company) || ((all.emails as { address: string }[])[0]?.address ?? "");
            return all;
        };
        const asking = [...(core.asking ?? [])];
        let replies: Record<string, unknown> = {
            status: "off",
            internal_message: "",
            external_message: "",
            external_audience: "nobody",
            start: null,
            end: null,
        };
        const when = (time: unknown) => (time ? { date_time: String(time).replace("Z", ""), time_zone: "UTC" } : null);
        const acted = (ids: unknown, apply: (m: (typeof mail)[number]) => (() => void) | null) => {
            const undo: (() => void)[] = [];
            for (const m of mail.filter((each) => (ids as string[]).includes(each.id))) {
                const back = apply(m);
                if (back) undo.push(back);
            }
            const undo_id = `action-${actions.size + 1}`;
            actions.set(undo_id, () => undo.forEach((back) => back()));
            return { changed: undo.length, undo_id };
        };
        let syncs = 0;
        const shown = (m: (typeof mail)[number]) => ({
            id: m.id,
            folder_id: m.folder_id,
            conversation_id: m.conversation ?? `conversation-${m.id}`,
            subject: m.subject,
            preview: m.preview,
            from: m.from,
            to: [{ name: "Robin Example", address: "robin@example.test" }],
            cc: [],
            received_at: m.received_at,
            is_read: m.is_read,
            is_draft: m.is_draft ?? false,
            has_attachments: m.has_attachments,
            importance: "normal",
            flag_status: flags.get(m.id) ?? "notFlagged",
            flag_due: dues.get(m.id) ?? null,
            categories: tags.get(m.id) ?? [],
            focused: true,
        });
        const inFolder = (id: unknown) => mail.filter((m) => m.folder_id === id);
        const pinned = new Set<unknown>(core.pinned ?? []);
        const trusted = new Set<string | null>();
        const folder = (id: string, name: string, well_known: string | null, parent_id = "root", depth = 0) => ({
            id,
            parent_id,
            name,
            well_known,
            favourite: pinned.has(id),
            depth,
            unread: inFolder(id).filter((m) => !m.is_read).length,
            total: inFolder(id).length,
            hidden: false,
        });

        const sync = () => ({
            accounts: accounts.map((a) => ({
                account_id: a.id,
                ok: !core.syncFails,
                changed: 0,
                removed: 0,
                error: core.syncFails ?? null,
            })),
        });

        const commands: Record<string, (args: Record<string, unknown>) => unknown> = {
            "mail.sync": () => {
                syncs += 1;
                if (syncs === 2) mail.unshift(...(core.arrives ?? []));
                // A first sync that is still going: mail is on this computer, and the sync has not ended.
                if (core.syncNeverEnds) return never;
                return sync();
            },
            "mail.folders.list": (args) => ({
                account_id: args.account_id,
                folders: [
                    folder("inbox", "Inbox", "inbox"),
                    folder("drafts", "Drafts", "drafts"),
                    folder("sent", "Sent Items", "sentitems"),
                    folder("deleted", "Deleted Items", "deleteditems"),
                    folder("archive", "Archive", "archive"),
                    folder("junk", "Junk Email", "junkemail"),
                    folder("projects", "Projects", null),
                    folder("harbour", "Harbour upgrade", null, "projects", 1),
                    folder("surveys", "Surveys", null, "harbour", 2),
                    folder("website", "Website", null, "projects", 1),
                ],
            }),
            "mail.folders.set_favourite": (args) => {
                if (args.favourite) pinned.add(args.folder_id);
                else pinned.delete(args.folder_id);
                return { account_id: args.account_id, folder_id: args.folder_id, favourite: args.favourite };
            },
            "mail.messages.list": (args) => {
                const newest = (a: (typeof mail)[number], b: (typeof mail)[number]) => b.received_at.localeCompare(a.received_at);
                const orders: Record<string, (a: (typeof mail)[number], b: (typeof mail)[number]) => number> = {
                    newest,
                    oldest: (a, b) => newest(b, a),
                    sender: (a, b) => (a.from.name ?? "").localeCompare(b.from.name ?? "") || newest(a, b),
                    subject: (a, b) => a.subject.localeCompare(b.subject) || newest(a, b),
                };
                const all = inFolder(args.folder_id).sort(orders[(args.sort as string) ?? "newest"]);
                const offset = (args.offset as number) ?? 0;
                const limit = (args.limit as number) ?? 50;
                if (args.conversations === true) {
                    const of = (m: (typeof mail)[number]) => shown(m).conversation_id;
                    const lines = all
                        .filter((m, at) => all.findIndex((each) => of(each) === of(m)) === at)
                        .map((m) => {
                            const same = all.filter((each) => of(each) === of(m));
                            return { ...shown(m), conversation: { count: same.length, unread: same.filter((each) => !each.is_read).length } };
                        });
                    return { total: lines.length, messages: lines.slice(offset, offset + limit), note: "" };
                }
                return { total: all.length, messages: all.slice(offset, offset + limit).map(shown), note: "" };
            },
            "mail.conversations.get": (args) => ({
                messages: mail
                    .filter((m) => shown(m).conversation_id === args.conversation_id)
                    .sort((a, b) => a.received_at.localeCompare(b.received_at))
                    .map(shown),
                note: "",
            }),
            "mail.messages.get": (args) => {
                const m = mail.find((each) => each.id === args.message_id);
                if (!m) throw { code: "failed", message: "There is no such message on this computer." };
                const sender_trusted = trusted.has(m.from.address);
                const allowed = args.allow_remote_content === true || (sender_trusted && args.allow_for_trusted_senders === true);
                const pictures = allowed && m.bodyWithPictures !== undefined;
                const body =
                    args.format === "html"
                        ? { format: "html", content: pictures ? m.bodyWithPictures : m.body, blocked_remote_content: pictures ? 0 : (m.blocked ?? 0) }
                        : { format: "text", content: m.preview, blocked_remote_content: 0 };
                return { message: shown(m), meeting: m.meeting ?? null, body, sender_trusted, remote_content_allowed: allowed, note: "" };
            },
            "mail.trusted_senders.set": (args) => {
                if (args.trusted) trusted.add(args.address as string);
                else trusted.delete(args.address as string);
                return { address: args.address, trusted: args.trusted };
            },
            "mail.drafts.create": (args) => {
                const source = mail.find((each) => each.id === args.message_id);
                const asked = (args.kind as string) ?? "new";
                const attached = asked === "forward_as_attachment";
                const kind = attached ? "new" : asked;
                const made = {
                    id: `draft-${drafts.length + sent.length + 1}`,
                    kind,
                    message_id: attached ? null : (source?.id ?? null),
                    to: kind === "reply" || kind === "reply_all" ? [source?.from] : [],
                    cc: [],
                    bcc: [],
                    subject: source ? `${kind === "forward" || attached ? "FW:" : "RE:"} ${source.subject}` : "",
                    body: !(signature.text && (kind === "new" || signature.on_replies))
                        ? ""
                        : args.format === "html"
                          ? `<p><br></p><p>${signature.text.replace(/\n/g, "<br>")}</p>`
                          : `\n\n${signature.text}`,
                    format: (args.format as string) ?? "text",
                    state: "editing",
                    error: null,
                    files: (attached ? [{ id: 1, name: `${source?.subject}.eml`, content_type: "message/rfc822", size: 4_096 }] : []) as Record<string, unknown>[],
                };
                drafts.unshift(made);
                return { draft: made };
            },
            "mail.automatic_replies.get": () => {
                if (core.repliesRefused) throw { code: "failed", message: core.repliesRefused };
                return replies;
            },
            "mail.automatic_replies.set": (args) => {
                replies = { ...replies, ...args, start: when(args.start) ?? replies.start, end: when(args.end) ?? replies.end };
                delete replies.account_id;
                return replies;
            },
            "mail.people.suggest": (args) => {
                const typed = String(args.typed).toLowerCase();
                const known = [...new Map(mail.map((m) => [m.from.address, m.from])).values()];
                return {
                    people: known.filter((p) => `${p.name} ${p.address}`.toLowerCase().split(/[\s.@]+/).some((word) => word.startsWith(typed))),
                    note: "",
                };
            },
            "app.agent_log.list": () => ({ entries: core.agentLog ?? [] }),
            "calendar.calendars.list": (args) => ({ account_id: args.account_id, calendars: diaries }),
            "calendar.calendars.create": (args) => {
                const id = `made-${diaries.length + 1}`;
                diaries.push({ id, name: args.name as string, color: "auto", hex_color: "", is_default: false, can_edit: true, shown: true });
                return { calendar_id: id, name: args.name };
            },
            "calendar.calendars.delete": (args) => {
                diaries.splice(diaries.findIndex((each) => each.id === args.calendar_id), 1);
                core.events = (core.events ?? []).filter((each) => (each.calendar_id ?? "main") !== args.calendar_id);
                return { deleted: true };
            },
            "calendar.calendars.set_shown": (args) => {
                const found = diaries.find((each) => each.id === args.calendar_id);
                if (found) found.shown = args.shown as boolean;
                return { calendar_id: args.calendar_id, shown: args.shown };
            },
            "calendar.events.list": (args) => ({
                events: (core.events ?? [])
                    .map(event)
                    .filter((e) => e.start < new Date(args.end as string).toISOString() && e.end > new Date(args.start as string).toISOString())
                    .filter((e) => diaries.find((d) => d.id === e.calendar_id)?.shown)
                    .sort((a, b) => a.start.localeCompare(b.start)),
                more: false,
                note: "",
            }),
            "calendar.reminders.due": () => ({
                reminders: (core.events ?? [])
                    .filter((e) => !e.is_cancelled)
                    .map((e) => ({ event: event(e), starts_in_minutes: Math.round((+new Date(e.start) - Date.now()) / 60_000) }))
                    .filter((r) => r.starts_in_minutes <= 15 && r.starts_in_minutes > -24 * 60 && (done.get(r.event.id) ?? 0) <= Date.now()),
                note: "",
            }),
            "calendar.reminders.dismiss": (args) => {
                done.set(args.event_id, Infinity);
                return { dismissed: true };
            },
            "calendar.reminders.snooze": (args) => {
                done.set(args.event_id, Date.now() + (args.minutes as number) * 60_000);
                return { snoozed_until: "" };
            },
            "calendar.events.create": (args) => {
                if (core.offline) throw { code: "failed", message: "Microsoft 365 could not be reached, so the calendar was not changed. Sync before trying again, in case it was." };
                core.events ??= [];
                const made = { id: `made-${core.events.length + 1}`, subject: "", start: "", end: "", mine: true };
                core.events.push(made);
                written(made, args);
                return { event: event(made), note: "" };
            },
            "calendar.events.update": (args) => {
                if (core.offline) throw { code: "failed", message: "Microsoft 365 could not be reached, so the calendar was not changed. Sync before trying again, in case it was." };
                const found = (core.events ?? []).find((each) => each.id === args.event_id);
                if (!found) throw { code: "failed", message: "There is no such event on this computer." };
                // Of a series, what is said of it goes to as much of the series as is asked.
                const { start: _start, end: _end, ...said } = args;
                for (const each of core.events ?? []) {
                    const also = found.series_id && each.series_id === found.series_id && (args.scope === "series" || (args.scope === "following" && each.start >= found.start));
                    if (each === found) written(each, args);
                    else if (also) written(each, said);
                }
                return { event: args.scope && args.scope !== "occurrence" ? { ...event(found), kind: "seriesMaster" } : event(found), note: "" };
            },
            "calendar.events.delete": (args) => {
                if (core.offline) throw { code: "failed", message: "Microsoft 365 could not be reached, so the calendar was not changed. Sync before trying again, in case it was." };
                const found = (core.events ?? []).find((each) => each.id === args.event_id);
                core.events = (core.events ?? []).filter((each) => {
                    if (each === found) return false;
                    const also = found?.series_id && each.series_id === found.series_id && (args.scope === "series" || (args.scope === "following" && each.start >= found.start));
                    return !also;
                });
                return { deleted: true };
            },
            "contacts.folders.list": () => ({ folders: [{ id: "", name: "Contacts", is_default: true }, { id: "suppliers", name: "Suppliers", is_default: false }] }),
            "contacts.list": (args) => {
                const words = String(args.query ?? "").toLowerCase().split(/\s+/).filter(Boolean);
                return {
                    contacts: (core.contacts ?? [])
                        .filter((each) => args.folder_id === undefined || each.folder_id === args.folder_id)
                        .filter((each) => words.every((word) => JSON.stringify(each).toLowerCase().includes(word)))
                        .sort((a, b) => String(a.name).localeCompare(String(b.name))),
                    more: false,
                    note: "",
                };
            },
            "contacts.create": (args) => {
                if (core.offline) throw { code: "failed", message: "Microsoft 365 could not be reached, so your contacts were not changed. Sync before trying again, in case they were." };
                core.contacts ??= [];
                const made = contactOf({ id: `made-${core.contacts.length + 1}`, folder_id: args.folder_id ?? "" }, args);
                core.contacts.push(made);
                return { contact: made, note: "" };
            },
            "contacts.update": (args) => {
                const at = (core.contacts ?? []).findIndex((each) => each.id === args.contact_id);
                const changed = contactOf((core.contacts ?? [])[at], args);
                (core.contacts ?? [])[at] = changed;
                return { contact: changed, note: "" };
            },
            "contacts.delete": (args) => {
                core.contacts = (core.contacts ?? []).filter((each) => each.id !== args.contact_id);
                return { deleted: true };
            },
            "people.search": (args) => {
                const typed = String(args.query).toLowerCase();
                const directory = core.offline ? [] : (core.directory ?? []).filter((each) => each.name.toLowerCase().split(" ").some((word) => word.startsWith(typed)));
                return {
                    people: directory.map((each) => ({ job_title: "", department: "", company: "", phones: [], ...each, source: "directory" })),
                    directory_searched: !core.offline,
                    note: "",
                };
            },
            "calendar.rooms.list": () => {
                if (core.offline) throw { code: "failed", message: "Microsoft 365 could not be reached, so there is no way to list meeting rooms just now." };
                return {
                    rooms: [
                        { name: "Annex", address: "annex@example.test", capacity: 4, building: "Annex", free: true },
                        { name: "Harbour Room", address: "harbour@example.test", capacity: 12, building: "Head office", free: true },
                        { name: "Boardroom", address: "board@example.test", capacity: 20, building: "Head office", free: false },
                    ],
                };
            },
            "calendar.free_busy": (args) => {
                if (core.offline) throw { code: "failed", message: "Microsoft 365 could not be reached. Free and busy times are only known while it can be." };
                // A day in half hours. Whoever has nothing said of them is free all day.
                const people = (args.people as string[]).map((address) =>
                    address.startsWith("nobody")
                        ? { address, availability: "", busy: [], error: "The recipient was not found." }
                        : { address, availability: (core.busy?.[address] ?? "").padEnd(48, "0"), busy: [], error: null },
                );
                const from = +new Date(args.start as string);
                const needed = Math.ceil(((args.duration_minutes as number) ?? 30) / 30);
                const free = (slot: number) => people.every((each) => each.error || each.availability[slot] === "0");
                const free_slots = [];
                for (let first = 0; first + needed <= 48; first++) {
                    if (Array.from({ length: needed }, (_, i) => first + i).every(free)) {
                        free_slots.push({ start: new Date(from + first * 1_800_000).toISOString(), end: new Date(from + (first + needed) * 1_800_000).toISOString() });
                    }
                }
                return { start: args.start, end: args.end, interval_minutes: 30, people, free_slots };
            },
            "calendar.events.cancel": (args) => {
                if (core.offline) throw { code: "failed", message: "Microsoft 365 could not be reached, and the meeting was not cancelled. Nothing was sent; try again." };
                const found = (core.events ?? []).find((each) => each.id === args.event_id);
                core.events = (core.events ?? []).filter((each) => each !== found && !(args.scope === "series" && found?.series_id && each.series_id === found.series_id));
                return { cancelled: true };
            },
            "calendar.events.respond": (args) => {
                if (core.offline) throw { code: "failed", message: "Microsoft 365 could not be reached, and the answer was not taken. Nothing was sent; try again." };
                const given = { accept: "accepted", tentative: "tentativelyAccepted", decline: "declined" }[args.response as string];
                const at = (core.events ?? []).findIndex((each) => each.id === args.event_id);
                if (at < 0) throw { code: "failed", message: "There is no such event on this computer." };
                if (args.response === "decline") core.events?.splice(at, 1);
                else (core.events ?? [])[at].response = given;
                for (const m of mail) if (m.meeting && m.meeting.event_id === args.event_id) m.meeting.response = given;
                return { response: given, organizer_told: args.send_response !== false };
            },
            "calendar.events.get": (args) => {
                const found = (core.events ?? []).find((each) => each.id === args.event_id);
                if (!found) throw { code: "failed", message: "There is no such event on this computer." };
                return { event: event(found), description: found.description ?? "", description_complete: !core.offline, repeat: found.repeat ?? null, note: "" };
            },
            "app.confirmations.list": () => ({ questions: asking }),
            "app.confirmations.answer": (args) => {
                const at = asking.findIndex((each) => each.id === args.id);
                if (at >= 0) asking.splice(at, 1);
                return { answered: at >= 0 };
            },
            "mail.drafts.get": (args) => ({ draft: draft(args.draft_id) ?? null }),
            "mail.signature.get": () => signature,
            "mail.signature.set": (args) => {
                signature = { text: args.text as string, on_replies: (args.on_replies as boolean) ?? signature.on_replies };
                return signature;
            },
            "mail.drafts.edit": (args) => {
                const found = draft(args.draft_id);
                if (!found) throw { code: "failed", message: "There is no such draft." };
                for (const line of ["to", "cc", "bcc"]) if (args[line] !== undefined) found[line] = checked(args[line]);
                for (const part of ["subject", "body", "format"]) if (args[part] !== undefined) found[part] = args[part];
                if (args.from !== undefined) found.from = args.from || null;
                found.state = "editing";
                found.error = null;
                return { draft: found };
            },
            "mail.drafts.files.add": (args) => {
                const found = draft(args.draft_id);
                const name = args.path ? (String(args.path).split("/").pop() ?? "") : String(args.name);
                if (name.startsWith("huge")) throw { code: "failed", message: `\`${name}\` is over 25 MB, which is more than most mail systems accept. Share it as a link instead.` };
                const id = (found?.files as unknown[]).length + 1;
                const content = (args.content_base64 as string) ?? "";
                const file = {
                    id,
                    name,
                    content_type: args.inline ? "image/png" : "application/pdf",
                    size: args.path ? 184_320 : Math.floor((content.length * 3) / 4),
                    content_id: args.inline ? `picture-${id}@brolga` : null,
                    content_base64: content,
                };
                (found?.files as unknown[]).push(file);
                return { file };
            },
            "mail.drafts.files.get": (args) => ({
                file: (draft(args.draft_id)?.files as { id: number }[]).find((each) => each.id === args.file_id),
            }),
            "mail.drafts.files.remove": (args) => {
                const found = draft(args.draft_id);
                if (found) found.files = (found.files as { id: number }[]).filter((each) => each.id !== args.file_id);
                return { removed: true };
            },
            "mail.drafts.open": (args) => {
                const here = drafts.find((each) => each.server_id === args.message_id);
                if (here) return { draft: here };
                const m = mail.find((each) => each.id === args.message_id);
                if (!m) throw { code: "failed", message: "This draft is not on this computer yet, and could not be fetched now." };
                const taken = {
                    id: `draft-${drafts.length + sent.length + 1}`,
                    server_id: m.id,
                    kind: "new",
                    message_id: null,
                    to: [{ name: "Jo Nguyen", address: "jo.nguyen@example.test" }],
                    cc: [],
                    bcc: [],
                    subject: m.subject,
                    body: m.body,
                    format: "html",
                    state: "editing",
                    error: null,
                    on_server: true,
                    files: [],
                };
                drafts.unshift(taken);
                return { draft: taken };
            },
            "app.outbox.flush": () => ({ sent: 0, waiting: 0, refused: 0, drafts_saved: 0 }),
            "mail.drafts.list": () => ({ drafts }),
            "mail.drafts.discard": (args) => {
                const at = drafts.findIndex((each) => each.id === args.draft_id);
                if (at >= 0) drafts.splice(at, 1);
                return { discarded: at >= 0 };
            },
            "mail.drafts.send": (args) => {
                const found = draft(args.draft_id);
                if (!found) throw { code: "failed", message: "There is no such draft." };
                if ([found.to, found.cc, found.bcc].every((line) => (line as unknown[]).length === 0)) {
                    throw { code: "failed", message: "Add at least one recipient before sending." };
                }
                if (core.sendRefused) {
                    found.state = "failed";
                    found.error = core.sendRefused;
                } else {
                    drafts.splice(drafts.indexOf(found), 1);
                    sent.push(found);
                }
                return { queued: true };
            },
            "mail.messages.set_read": (args) =>
                acted(args.message_ids, (m) => {
                    if (m.is_read === args.is_read) return null;
                    const was = m.is_read;
                    m.is_read = args.is_read as boolean;
                    return () => (m.is_read = was);
                }),
            "mail.messages.set_flag": (args) =>
                acted(args.message_ids, (m) => {
                    const was = flags.get(m.id) ?? "notFlagged";
                    const wasDue = dues.get(m.id) ?? null;
                    const due = args.flag !== "flagged" ? null : args.due === undefined ? wasDue : (args.due as string) || null;
                    if (was === args.flag && due === wasDue) return null;
                    flags.set(m.id, args.flag as string);
                    dues.set(m.id, due);
                    return () => {
                        flags.set(m.id, was);
                        dues.set(m.id, wasDue);
                    };
                }),
            "mail.categories.list": () => ({
                categories: [
                    { name: "Clients", color: "preset7" },
                    { name: "Red category", color: "preset0" },
                    { name: "Plain", color: "none" },
                ],
            }),
            "mail.messages.set_categories": (args) =>
                acted(args.message_ids, (m) => {
                    const was = tags.get(m.id) ?? [];
                    tags.set(m.id, args.categories as string[]);
                    return () => tags.set(m.id, was);
                }),
            "mail.messages.move": (args) => {
                const named: Record<string, string> = { deleteditems: "deleted", sentitems: "sent", junkemail: "junk" };
                const to = named[args.folder_id as string] ?? (args.folder_id as string);
                return acted(args.message_ids, (m) => {
                    if (m.folder_id === to) return null;
                    const was = m.folder_id;
                    m.folder_id = to;
                    return () => (m.folder_id = was);
                });
            },
            "mail.messages.delete_permanently": (args) => {
                const ids = args.message_ids as string[];
                if (core.offline) throw { code: "failed", message: "0 of 1 were deleted before this went wrong, and the rest are untouched. Microsoft 365 could not be reached." };
                for (const id of ids) {
                    const at = mail.findIndex((each) => each.id === id);
                    if (at >= 0) mail.splice(at, 1);
                }
                return { deleted: ids.length };
            },
            "mail.undo": (args) => {
                const back = actions.get(args.undo_id as string);
                back?.();
                return { undone: back ? 1 : 0 };
            },
            "app.outbox.status": () => ({ accounts: accounts.map((a) => ({ account_id: a.id, queued: 0, refused })) }),
            "app.outbox.dismiss_refused": () => {
                refused.length = 0;
                return { dismissed: true };
            },
            "mail.attachments.list": (args) => ({
                attachments: mail.find((each) => each.id === args.message_id)?.attachments ?? [],
                note: "",
            }),
            "mail.attachments.save": (args) => {
                const found = mail.find((each) => each.id === args.message_id)?.attachments?.find((each) => each.id === args.attachment_id);
                if (!found) throw { code: "failed", message: "This message has no such attachment." };
                return { path: args.path ?? `/home/robin/Downloads/${found.name}`, size: found.size };
            },
            "mail.search": (args) => {
                const words = String(args.query).toLowerCase().split(/\s+/).filter(Boolean);
                const found = mail
                    .filter((m) => !args.folder_id || m.folder_id === args.folder_id)
                    .filter((m) => words.every((w) => `${m.subject} ${m.preview} ${m.from.name}`.toLowerCase().includes(w)));
                return { messages: found.map(shown), note: "" };
            },
            "app.open_link": () => ({ opened: true }),
            "accounts.list": () => ({ accounts }),
            "accounts.shared.list": () => ({ mailboxes: core.shared ?? [] }),
            "accounts.shared.add": (args) => {
                if (!String(args.address).startsWith("support")) {
                    throw { code: "failed", message: `There is no mailbox \`${args.address}\`, or you have not been given access to it.` };
                }
                const added = { id: `${args.account_id}~${args.address}`, account_id: args.account_id as string, address: args.address as string, name: "Support" };
                core.shared = [...(core.shared ?? []), added];
                return { mailbox: added };
            },
            "accounts.shared.remove": (args) => {
                core.shared = (core.shared ?? []).filter((each) => each.id !== args.mailbox_id);
                return { removed: true };
            },
            "accounts.sign_in_options": () => ({ needs_client_id: core.needsClientId ?? false }),
            "accounts.sign_in": () => (core.signInHangs ? never : signedIn()),
            "accounts.sign_in_with_code.begin": () => ({
                flow_id: "1",
                user_code: "ABCD1234",
                verification_uri: "https://login.example.test/device",
                message: "To sign in, open https://login.example.test/device and enter the code ABCD1234.",
                expires_in_seconds: 900,
            }),
            "accounts.sign_in_with_code.finish": () => (core.signInHangs ? never : signedIn()),
            "accounts.remove": (args) => {
                const at = accounts.findIndex((a) => a.id === args.id);
                if (at >= 0) accounts.splice(at, 1);
                return { removed: at >= 0 };
            },
        };

        Object.assign(window, {
            notified,
            arrive: () =>
                mail.push({
                    id: "arrived",
                    folder_id: "inbox",
                    subject: "Just in",
                    preview: "",
                    from: { name: "Sam Taylor", address: "sam.taylor@example.test" },
                    received_at: "2026-10-09T05:00:00Z",
                    is_read: false,
                    has_attachments: false,
                    body: "<p>Just in.</p>",
                }),
            ran,
            sent,
            __TAURI_INTERNALS__: {
                invoke: async (name: string, payload: { command: string; args?: Record<string, unknown> }) => {
                    // The save dialog: the place this test says was chosen, or dismissed.
                    if (name === "choose_save_path") return core.savePath ?? null;
                    // The file dialog: the files this test says were chosen.
                    if (name === "choose_files") return core.pickFiles ?? [];
                    // A notification on the desktop: written down, for a test to read.
                    if (name === "notify") {
                        notified.push(payload as unknown as { title: string; body: string });
                        return null;
                    }
                    if (name !== "execute_command") throw new Error(`The frontend called ${name}, which is not a command.`);
                    const run = commands[payload.command];
                    if (!run) throw { code: "unknown_command", message: `there is no command called \`${payload.command}\`` };
                    ran.push({ command: payload.command, args: payload.args ?? {} });
                    return run(payload.args ?? {});
                },
            },
        });
    }, core);

    await page.goto("/");
}

export const ran = (page: Page) =>
    page.evaluate(() => (window as unknown as { ran: { command: string; args: Record<string, unknown> }[] }).ran);

/** An event as the core's commands give it out. */
export interface FakeEvent {
    id: string;
    calendar_id?: string;
    subject: string;
    /** UTC. */
    start: string;
    end: string;
    is_all_day?: boolean;
    location?: string;
    time_zone?: string;
    show_as?: string;
    is_cancelled?: boolean;
    join_url?: string;
    attendees?: { name: string; address: string; kind: string; response: string; proposed_start?: string; proposed_end?: string }[];
    description?: string;
    /** The owner's own answer. Left out, the meeting has been accepted. */
    response?: string;
    /** The owner's own event, not one they were invited to. */
    mine?: boolean;
    /** The series it is one of. */
    series_id?: string;
    /** How that series repeats. */
    repeat?: Record<string, unknown>;
    sensitivity?: string;
    reminder_minutes?: number | null;
}

/**
 * A made-up week, about Wednesday 14 October 2026 in Brisbane (UTC+10):
 * a timed event, two that overlap, one all day over two days, one on another
 * calendar, one cancelled, and one made in another zone.
 */
export const week: FakeEvent[] = [
    { id: "e1", subject: "Stand-up", start: "2026-10-13T23:00:00Z", end: "2026-10-13T23:15:00Z", location: "Room 2" },
    {
        id: "e2",
        subject: "Budget review",
        start: "2026-10-14T04:00:00Z",
        end: "2026-10-14T05:00:00Z",
        location: "Harbour Room",
        join_url: "https://meet.example.test/budget",
        attendees: [
            { name: "Sam Taylor", address: "sam.taylor@example.test", kind: "required", response: "accepted" },
            { name: "Alex Chen", address: "alex.chen@example.test", kind: "optional", response: "notResponded" },
        ],
        description: "Bring the Q3 numbers.\nRoom is booked for an hour.",
    },
    { id: "e3", subject: "Supplier call", start: "2026-10-14T04:30:00Z", end: "2026-10-14T05:30:00Z", show_as: "tentative" },
    { id: "e4", subject: "Conference", start: "2026-10-15T00:00:00Z", end: "2026-10-17T00:00:00Z", is_all_day: true },
    { id: "e5", subject: "School pickup", calendar_id: "family", start: "2026-10-16T05:00:00Z", end: "2026-10-16T05:30:00Z" },
    { id: "e6", subject: "Old plan", start: "2026-10-12T01:00:00Z", end: "2026-10-12T02:00:00Z", is_cancelled: true },
    { id: "e7", subject: "London sync", start: "2026-10-14T07:00:00Z", end: "2026-10-14T07:30:00Z", time_zone: "Europe/London" },
    { id: "e8", subject: "Next month", start: "2026-11-03T01:00:00Z", end: "2026-11-03T02:00:00Z" },
];

/** What the stand-in core has sent. */
export const sent = (page: Page) => page.evaluate(() => (window as unknown as { sent: Record<string, unknown>[] }).sent);

/** What the desktop has been asked to show as notifications. */
export const notified = (page: Page) => page.evaluate(() => (window as unknown as { notified: { title: string; body: string }[] }).notified);
