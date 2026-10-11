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
}

export const listMessages = (account_id: string, folder_id: string, offset: number, limit: number, view: View) =>
    execute<{ total: number; messages: Message[] }>("mail.messages.list", { account_id, folder_id, offset, limit, ...view });

export const getConversation = async (account_id: string, conversation_id: string) =>
    (await execute<{ messages: Message[] }>("mail.conversations.get", { account_id, conversation_id })).messages;

export const openMessage = (account_id: string, message_id: string, allow_remote_content: boolean) =>
    execute<Opened>("mail.messages.get", { account_id, message_id, format: "html", allow_remote_content });

export const searchMail = async (account_id: string, query: string) =>
    (await execute<{ messages: Message[] }>("mail.search", { account_id, query, limit: 200 })).messages;

export const listAttachments = async (account_id: string, message_id: string) =>
    (await execute<{ attachments: Attachment[] }>("mail.attachments.list", { account_id, message_id })).attachments;

export const saveAttachment = (account_id: string, message_id: string, attachment_id: string) =>
    execute<{ path: string; size: number }>("mail.attachments.save", { account_id, message_id, attachment_id });

/** A size in bytes as a person would say it. */
export function size(bytes: number): string {
    if (bytes < 1024) return `${bytes} B`;
    if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`;
    return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

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
