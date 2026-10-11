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

export const listMessages = (account_id: string, folder_id: string, offset: number, limit: number) =>
    execute<{ total: number; messages: Message[] }>("mail.messages.list", { account_id, folder_id, offset, limit });

export const openMessage = (account_id: string, message_id: string, allow_remote_content: boolean) =>
    execute<Opened>("mail.messages.get", { account_id, message_id, format: "html", allow_remote_content });

export const searchMail = async (account_id: string, query: string) =>
    (await execute<{ messages: Message[] }>("mail.search", { account_id, query, limit: 200 })).messages;

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
