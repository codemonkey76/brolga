import { execute } from "./commands";

export interface Account {
    /** The account this mailbox was shared with, where it is a shared mailbox and not an account. */
    shared_with?: string;
    /** It is mail read in from a file, kept on this computer, with no server behind it. */
    archive?: boolean;
    id: string;
    username: string;
    display_name: string | null;
    tenant_id: string;
    /** The saved sign-in no longer works and the person must sign in again. */
    needs_sign_in: boolean;
    added_at: string;
}

/** Which app registration to sign in under. Both blank means Brolga's own. */
export interface Under {
    client_id?: string;
    tenant?: string;
}

export interface CodeSignIn {
    flow_id: string;
    user_code: string;
    verification_uri: string;
    message: string;
    expires_in_seconds: number;
}

export const listAccounts = async () => (await execute<{ accounts: Account[] }>("accounts.list")).accounts;

export const signInOptions = () => execute<{ needs_client_id: boolean; remembered_client_id?: string | null }>("accounts.sign_in_options");

export const signIn = async (under: Under) =>
    (await execute<{ account: Account }>("accounts.sign_in", { ...under })).account;

export const signInAgain = async (account: Account) =>
    (await execute<{ account: Account }>("accounts.sign_in", { account_id: account.id })).account;

export const beginWithCode = (under: Under) => execute<CodeSignIn>("accounts.sign_in_with_code.begin", { ...under });

export const finishWithCode = async (flow: CodeSignIn) =>
    (await execute<{ account: Account }>("accounts.sign_in_with_code.finish", { flow_id: flow.flow_id })).account;

export const removeAccount = (account: Account) => execute<{ removed: boolean }>("accounts.remove", { id: account.id });

/** A mailbox someone has shared with an account. Its id goes wherever an account id does. */
export interface SharedMailbox {
    id: string;
    account_id: string;
    address: string;
    name: string;
    /** Only its calendar was shared: there is no mail of it to read. */
    calendar_only?: boolean;
}

export const listShared = async () => (await execute<{ mailboxes: SharedMailbox[] }>("accounts.shared.list")).mailboxes;

export const addShared = async (account_id: string, address: string) =>
    (await execute<{ mailbox: SharedMailbox }>("accounts.shared.add", { account_id, address })).mailbox;

export const removeShared = (mailbox_id: string) => execute("accounts.shared.remove", { mailbox_id });

/** Mail read in from a PST or OST file. */
export interface Archive {
    id: string;
    name: string;
    source: string;
    state: "importing" | "ready" | "cancelled" | "failed";
    /** Messages kept so far. */
    messages: number;
    messages_done: number | null;
    messages_total: number | null;
    skipped: number;
    error: string | null;
}

export const listArchives = async () => (await execute<{ archives: Archive[] }>("archives.list")).archives;

export const importArchive = async (path: string) => (await execute<{ archive: Archive }>("archives.import", { path })).archive;

export const cancelArchive = (archive_id: string) => execute("archives.cancel", { archive_id });

export const removeArchive = (archive_id: string) => execute("archives.remove", { archive_id });

/**
 * Every mailbox there is to read mail in: each account, followed by those
 * shared with it, and then the archives. For calendars, those of which only
 * the calendar was shared are among them, and archives are not.
 */
export function mailboxes(accounts: Account[], shared: SharedMailbox[], calendars = false, archives: Archive[] = []): Account[] {
    return [
        ...accounts.flatMap((account) => [
            account,
            ...shared
                .filter((each) => each.account_id === account.id && (calendars || !each.calendar_only))
                .map((each) => ({ ...account, id: each.id, username: each.address, display_name: each.name || null, shared_with: account.id })),
        ]),
        ...(calendars ? [] : archives).map((each) => ({ id: each.id, username: each.name, display_name: each.name, tenant_id: "", needs_sign_in: false, added_at: "", archive: true })),
    ];
}
