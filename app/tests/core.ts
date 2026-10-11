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

        const mail = [...(core.mail ?? [])];
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
            is_draft: false,
            has_attachments: m.has_attachments,
            importance: "normal",
            flag_status: "notFlagged",
            categories: [],
            focused: true,
        });
        const inFolder = (id: unknown) => mail.filter((m) => m.folder_id === id);
        const pinned = new Set<unknown>(core.pinned ?? []);
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
                const pictures = args.allow_remote_content === true && m.bodyWithPictures !== undefined;
                return {
                    message: shown(m),
                    body: { format: "html", content: pictures ? m.bodyWithPictures : m.body, blocked_remote_content: pictures ? 0 : (m.blocked ?? 0) },
                    note: "",
                };
            },
            "mail.search": (args) => {
                const words = String(args.query).toLowerCase().split(/\s+/).filter(Boolean);
                const found = mail.filter((m) => words.every((w) => `${m.subject} ${m.preview} ${m.from.name}`.toLowerCase().includes(w)));
                return { messages: found.map(shown), note: "" };
            },
            "app.open_link": () => ({ opened: true }),
            "accounts.list": () => ({ accounts }),
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
            ran,
            __TAURI_INTERNALS__: {
                invoke: async (name: string, payload: { command: string; args?: Record<string, unknown> }) => {
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
