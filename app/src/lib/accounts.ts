import { execute } from "./commands";

export interface Account {
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
