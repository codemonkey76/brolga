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

export interface FakeCore {
    accounts?: FakeAccount[];
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

        const commands: Record<string, (args: Record<string, unknown>) => unknown> = {
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
