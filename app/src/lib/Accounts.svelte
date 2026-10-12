<script lang="ts">
    import AccountRow from "./AccountRow.svelte";
    import Archives from "./Archives.svelte";
    import Desktop from "./Desktop.svelte";
    import {
        type Account,
        type CodeSignIn,
        beginWithCode,
        finishWithCode,
        listAccounts,
        removeAccount,
        signIn,
        signInAgain,
        signInOptions,
    } from "./accounts";
    import { say } from "./commands";

    interface Props {
        /** Told whenever the list of accounts may have changed. */
        onChanged: (accounts: Account[]) => void;
        /** Told whenever an archive is added, finishes or goes. */
        onArchives?: () => void;
    }

    let { onChanged, onArchives }: Props = $props();
    let accounts = $state<Account[]>([]);
    let loaded = $state(false);
    let needsClientId = $state(false);
    let showAdvanced = $state(false);
    let clientId = $state("");
    /** The client ID that worked last time, which is used again unless another is typed. */
    let remembered = $state<string | null>(null);
    let tenant = $state("");
    /** What is under way, if anything: nothing else can be started meanwhile. */
    let waiting = $state<null | "browser" | "code" | "removing">(null);
    let code = $state<CodeSignIn | null>(null);
    let problem = $state<string | null>(null);

    const under = () => ({
        ...(clientId.trim() ? { client_id: clientId.trim() } : {}),
        ...(tenant.trim() ? { tenant: tenant.trim() } : {}),
    });
    const missingClientId = $derived(needsClientId && !clientId.trim());

    /** Bumped with each thing started and each one given up on, so that what a sign-in no longer waited for comes to is not shown. */
    let attempts = 0;

    /** Run one thing at a time, show why it failed if it did, and reload the list after. `waited` is false once it has been given up on. */
    async function attempt(kind: "browser" | "code" | "removing", work: (waited: () => boolean) => Promise<unknown>) {
        const mine = ++attempts;
        const waited = () => mine === attempts;
        waiting = kind;
        problem = null;
        try {
            await work(waited);
        } catch (failure) {
            if (waited()) problem = say(failure);
        } finally {
            if (waited()) {
                waiting = null;
                code = null;
            }
            // Read again even so: a sign-in given up on here may still be finished in the browser.
            accounts = await listAccounts().catch(() => accounts);
            onChanged(accounts);
        }
    }

    /**
     * Stop waiting for a sign-in. The core has no command to call one off, so it
     * goes on waiting by itself until the browser or the code is finished with
     * or it gives up; the window is free meanwhile.
     */
    function stopWaiting() {
        attempts += 1;
        waiting = null;
        code = null;
    }

    const add = () => attempt("browser", () => signIn(under()));
    const again = (account: Account) => attempt("browser", () => signInAgain(account));
    const remove = (account: Account) => attempt("removing", () => removeAccount(account));
    const addWithCode = () =>
        attempt("code", async (waited) => {
            const begun = await beginWithCode(under());
            if (!waited()) return;
            code = begun;
            await finishWithCode(begun);
        });

    async function start() {
        try {
            const [listed, options] = await Promise.all([listAccounts(), signInOptions()]);
            accounts = listed;
            needsClientId = options.needs_client_id;
            remembered = options.remembered_client_id ?? null;
            showAdvanced = needsClientId;
        } catch (failure) {
            problem = say(failure);
        } finally {
            loaded = true;
        }
    }

    start();
</script>

<div class="page">
    <header>
        <h1>Accounts</h1>
        <p class="muted">Microsoft 365 work and school accounts in Brolga.</p>
    </header>

    {#if problem}
        <p class="problem" role="alert">{problem}</p>
    {/if}

    {#if !loaded}
        <p class="muted" role="status">Starting…</p>
    {:else}
        {#if accounts.length > 0}
            <ul class="card" aria-label="Accounts">
                {#each accounts as account (account.id)}
                    <AccountRow {account} busy={waiting !== null} onSignInAgain={again} onRemove={remove} onShared={() => onChanged(accounts)} />
                {/each}
            </ul>
        {:else}
            <p class="card empty">No account yet. Add one to get started.</p>
        {/if}

        <section class="card add" aria-labelledby="add-heading">
            <h2 id="add-heading">Add an account</h2>

            {#if waiting === "browser"}
                <p role="status">Finish signing in in your browser. This page will update when you are done.</p>
                <div class="actions"><button type="button" onclick={stopWaiting}>Cancel</button></div>
            {:else if waiting === "code" && code}
                <p role="status">
                    On any device, open <strong>{code.verification_uri}</strong> and enter this code:
                </p>
                <p class="code" aria-label="Sign-in code">{code.user_code}</p>
                <p class="muted">Waiting for you to finish…</p>
                <div class="actions"><button type="button" onclick={stopWaiting}>Cancel</button></div>
            {:else if waiting === "code"}
                <p role="status">Getting a code…</p>
                <div class="actions"><button type="button" onclick={stopWaiting}>Cancel</button></div>
            {:else}
                <div class="actions">
                    <button type="button" class="primary" disabled={waiting !== null || missingClientId} onclick={add}>
                        Sign in with your browser
                    </button>
                    <button type="button" disabled={waiting !== null || missingClientId} onclick={addWithCode}>
                        Sign in with a code instead
                    </button>
                </div>

                <button type="button" class="link" aria-expanded={showAdvanced} onclick={() => (showAdvanced = !showAdvanced)}>
                    {showAdvanced ? "Hide advanced" : "Advanced"}
                </button>

                {#if showAdvanced}
                    <div class="advanced">
                        {#if needsClientId}
                            <p>
                                Brolga does not have its own app registration with Microsoft yet. Enter the client ID your
                                organisation has registered for it.
                            </p>
                        {:else}
                            <p class="muted">
                                If your organisation blocks apps it has not approved, enter the client ID it has registered for
                                Brolga.
                            </p>
                        {/if}
                        <label>
                            Client ID
                            <input bind:value={clientId} autocomplete="off" spellcheck="false" placeholder={remembered ?? "00000000-0000-0000-0000-000000000000"} />
                            {#if remembered}<span class="muted">Left empty, the one used last time is used again.</span>{/if}
                        </label>
                        <label>
                            Tenant ID or domain
                            <input bind:value={tenant} autocomplete="off" spellcheck="false" placeholder="Any organisation" />
                        </label>
                    </div>
                {/if}
            {/if}
        </section>

        <Archives onChanged={onArchives} />
        <Desktop />
    {/if}
</div>

<style>
    .page {
        max-width: 40rem;
        margin: 0 auto;
        padding: 2rem 1rem;
        display: flex;
        flex-direction: column;
        gap: 1rem;
    }

    h1,
    h2,
    p,
    ul {
        margin: 0;
    }

    h1 {
        font-size: 1.5rem;
        font-weight: 600;
    }

    h2 {
        font-size: 1rem;
        font-weight: 600;
    }

    ul {
        padding: 0;
        list-style: none;
    }

    .card {
        border: 1px solid var(--line);
        border-radius: 0.5rem;
        background: var(--raised);
    }

    .empty {
        padding: 1.5rem 1rem;
        text-align: center;
        color: var(--muted);
    }

    .add {
        padding: 1rem;
        display: flex;
        flex-direction: column;
        gap: 0.75rem;
    }

    .actions {
        display: flex;
        flex-wrap: wrap;
        gap: 0.5rem;
    }

    .advanced {
        display: flex;
        flex-direction: column;
        gap: 0.75rem;
    }

    label {
        display: flex;
        flex-direction: column;
        gap: 0.25rem;
        font-size: 0.85rem;
        font-weight: 600;
    }

    .code {
        align-self: flex-start;
        padding: 0.5rem 1rem;
        border-radius: 0.5rem;
        border: 1px solid var(--line);
        font-family: ui-monospace, monospace;
        font-size: 1.75rem;
        letter-spacing: 0.2em;
        user-select: all;
    }

    .problem {
        padding: 0.75rem 1rem;
        border-radius: 0.5rem;
        border: 1px solid color-mix(in srgb, var(--danger) 60%, var(--surface));
        background: color-mix(in srgb, var(--danger) 12%, var(--surface));
    }
</style>
