import { execute } from "./commands";

export interface Folder {
    id: string;
    parent_id: string | null;
    name: string;
    /** inbox, drafts, sentitems and so on, for the folders every mailbox has. */
    well_known: string | null;
    unread: number;
    total: number;
    /** Pinned to the top of the list, on this computer only. */
    favourite: boolean;
    /** How far below the top of the tree. The list comes in tree order. */
    depth: number;
}

export interface Person {
    name: string | null;
    address: string | null;
}

export interface Message {
    id: string;
    folder_id: string;
    conversation_id: string | null;
    /** Only in a list of conversations: how many of the folder's messages are in this one's. */
    conversation?: { count: number; unread: number };
    subject: string;
    preview: string;
    from: Person;
    to: Person[];
    cc: Person[];
    received_at: string;
    is_read: boolean;
    has_attachments: boolean;
    importance: string;
    flag_status: string;
}

export interface Attachment {
    id: string;
    name: string;
    content_type: string;
    /** In bytes. */
    size: number;
    /** Shown within the body, as a picture in a signature is. */
    is_inline: boolean;
    kind: "file" | "item" | "reference";
}

export interface Opened {
    message: Message;
    body: { format: "html" | "text"; content: string; blocked_remote_content: number };
    /** The sender's remote pictures are shown without asking. */
    sender_trusted: boolean;
    /** Remote pictures were fetched for this. */
    remote_content_allowed: boolean;
}

export interface SyncResult {
    account_id: string;
    ok: boolean;
    changed: number;
    removed: number;
    error: string | null;
}

export const listFolders = async (account_id: string) =>
    (await execute<{ folders: Folder[] }>("mail.folders.list", { account_id })).folders;

export const setFavourite = (account_id: string, folder_id: string, favourite: boolean) =>
    execute<{ favourite: boolean }>("mail.folders.set_favourite", { account_id, folder_id, favourite });

export type Sort = "newest" | "oldest" | "sender" | "subject";

/** How a folder is laid out in the list. */
export interface View {
    sort: Sort;
    conversations: boolean;
    /** Read messages as plain words, with no formatting or pictures. */
    plain: boolean;
    /** Where the message being read goes: beside the list or under it. */
    pane: "right" | "bottom";
}

export const listMessages = (account_id: string, folder_id: string, offset: number, limit: number, view: View) =>
    execute<{ total: number; messages: Message[] }>("mail.messages.list", {
        account_id,
        folder_id,
        offset,
        limit,
        sort: view.sort,
        conversations: view.conversations,
    });

export const getConversation = async (account_id: string, conversation_id: string) =>
    (await execute<{ messages: Message[] }>("mail.conversations.get", { account_id, conversation_id })).messages;

export const openMessage = (account_id: string, message_id: string, allow_remote_content: boolean, plain: boolean) =>
    execute<Opened>("mail.messages.get", {
        account_id,
        message_id,
        format: plain ? "text" : "html",
        allow_remote_content,
        allow_for_trusted_senders: true,
    });

export const setTrustedSender = (account_id: string, address: string, trusted: boolean) =>
    execute<{ trusted: boolean }>("mail.trusted_senders.set", { account_id, address, trusted });

export const searchMail = async (account_id: string, query: string) =>
    (await execute<{ messages: Message[] }>("mail.search", { account_id, query, limit: 200 })).messages;

export const listAttachments = async (account_id: string, message_id: string) =>
    (await execute<{ attachments: Attachment[] }>("mail.attachments.list", { account_id, message_id })).attachments;

export const saveAttachment = (account_id: string, message_id: string, attachment_id: string, path?: string) =>
    execute<{ path: string; size: number }>("mail.attachments.save", { account_id, message_id, attachment_id, ...(path ? { path } : {}) });

/** A size in bytes as a person would say it. */
export function size(bytes: number): string {
    if (bytes < 1024) return `${bytes} B`;
    if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`;
    return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

export interface Draft {
    id: string;
    kind: "new" | "reply" | "reply_all" | "forward";
    message_id: string | null;
    to: Person[];
    cc: Person[];
    bcc: Person[];
    subject: string;
    /** The words as written: plain, or HTML the core has cleaned, as `format` says. */
    body: string;
    format: "text" | "html";
    /** queued: waiting to be sent. failed: Microsoft 365 would not send it. */
    state: "editing" | "queued" | "failed";
    error: string | null;
    files: DraftFile[];
}

export interface DraftFile {
    id: number;
    name: string;
    content_type: string;
    /** In bytes. */
    size: number;
}

export const attachFile = async (account_id: string, draft_id: string, path: string) =>
    (await execute<{ file: DraftFile }>("mail.drafts.files.add", { account_id, draft_id, path })).file;

export const removeFile = (account_id: string, draft_id: string, file_id: number) =>
    execute<{ removed: boolean }>("mail.drafts.files.remove", { account_id, draft_id, file_id });

export const createDraft = async (account_id: string, kind: Draft["kind"], format: Draft["format"], message_id?: string) =>
    (await execute<{ draft: Draft }>("mail.drafts.create", { account_id, kind, format, ...(message_id ? { message_id } : {}) })).draft;

export const editDraft = (account_id: string, draft_id: string, change: Partial<Pick<Draft, "to" | "cc" | "bcc" | "subject" | "body" | "format">>) =>
    execute<{ draft: Draft }>("mail.drafts.edit", { account_id, draft_id, ...change });

export const listDrafts = async (account_id: string) => (await execute<{ drafts: Draft[] }>("mail.drafts.list", { account_id })).drafts;

export const discardDraft = (account_id: string, draft_id: string) => execute("mail.drafts.discard", { account_id, draft_id });

export const sendDraft = (account_id: string, draft_id: string) => execute<{ queued: boolean }>("mail.drafts.send", { account_id, draft_id });

export const suggestPeople = async (account_id: string, typed: string) =>
    (await execute<{ people: Person[] }>("mail.people.suggest", { account_id, typed })).people;

export interface Edited {
    changed: number;
    /** Give this to `undo` to take the action back. */
    undo_id: string;
}

export const setRead = (account_id: string, message_ids: string[], is_read: boolean) =>
    execute<Edited>("mail.messages.set_read", { account_id, message_ids, is_read });

export type FlagStatus = "flagged" | "complete" | "notFlagged";

export const setFlag = (account_id: string, message_ids: string[], flag: FlagStatus) =>
    execute<Edited>("mail.messages.set_flag", { account_id, message_ids, flag });

/** Move messages to a folder, by its id or a well-known name. Deleting is a move to `deleteditems`. */
export const moveMessages = (account_id: string, message_ids: string[], folder_id: string) =>
    execute<Edited>("mail.messages.move", { account_id, message_ids, folder_id });

export const undo = (account_id: string, undo_id: string) => execute<{ undone: number }>("mail.undo", { account_id, undo_id });

export interface Refused {
    message_id: string;
    subject: string;
    change: string;
    error: string;
}

/** The changes Microsoft 365 refused, which have been taken back here. */
export const refusedChanges = async (account_id: string) =>
    (await execute<{ accounts: { account_id: string; refused: Refused[] }[] }>("app.outbox.status")).accounts.find((a) => a.account_id === account_id)
        ?.refused ?? [];

export const dismissRefused = (account_id: string) => execute("app.outbox.dismiss_refused", { account_id });

export const syncNow = async () => (await execute<{ accounts: SyncResult[] }>("mail.sync")).accounts;

export const openLink = (url: string) => execute<{ opened: boolean }>("app.open_link", { url });

export const who = (person: Person) => person.name?.trim() || person.address || "Unknown sender";

/** Today as a time, this year as a day and month, otherwise the full date. */
export function when(iso: string, now = new Date()): string {
    const date = new Date(iso);
    if (Number.isNaN(date.getTime())) return "";
    if (date.toDateString() === now.toDateString()) {
        return date.toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" });
    }
    return date.toLocaleDateString(undefined, {
        day: "numeric",
        month: "short",
        ...(date.getFullYear() === now.getFullYear() ? {} : { year: "numeric" }),
    });
}

/** Which stretch of time a message belongs to in a list by date: today, yesterday, this week, last week, then its month. */
export function dateGroup(iso: string, now = new Date()): string {
    const date = new Date(iso);
    if (Number.isNaN(date.getTime())) return "";
    const day = (d: Date) => new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
    const DAY = 86_400_000;
    const ago = Math.round((day(now) - day(date)) / DAY);
    // Days since this week's Monday.
    const intoWeek = (now.getDay() + 6) % 7;

    if (ago < 0) return "Later";
    if (ago === 0) return "Today";
    if (ago === 1) return "Yesterday";
    if (ago <= intoWeek) return "Earlier this week";
    if (ago <= intoWeek + 7) return "Last week";
    return date.toLocaleDateString(undefined, { month: "long", year: "numeric" });
}
