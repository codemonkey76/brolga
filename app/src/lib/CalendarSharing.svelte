<script lang="ts">
    import { execute, say } from "./commands";

    interface Props {
        accountId: string;
        username: string;
    }

    interface Share {
        id: string;
        name: string;
        /** Null for the organisation as a whole. */
        address: string | null;
        role: string;
        allowed_roles: string[];
        removable: boolean;
    }

    /** What each role lets someone do, in the order of how much. */
    const ROLES: Record<string, string> = {
        none: "Nothing",
        freeBusyRead: "When I am busy",
        limitedRead: "Titles and places",
        read: "All details",
        write: "Can change it",
        delegateWithoutPrivateEventAccess: "Delegate",
        delegateWithPrivateEventAccess: "Delegate, private events too",
        custom: "Custom",
    };

    let { accountId, username }: Props = $props();
    let shares = $state<Share[] | null>(null);
    let problem = $state<string | null>(null);
    let address = $state("");
    let role = $state("read");
    let busy = $state(false);

    async function read() {
        try {
            shares = (await execute<{ shares: Share[] }>("calendar.sharing.list", { account_id: accountId })).shares;
        } catch (failure) {
            problem = say(failure);
        }
    }

    $effect(() => {
        read();
    });

    async function run(command: string, args: Record<string, unknown>) {
        busy = true;
        problem = null;
        try {
            await execute(command, { account_id: accountId, ...args });
            return true;
        } catch (failure) {
            problem = say(failure);
            return false;
        } finally {
            busy = false;
            await read();
        }
    }

    async function add(submitted: SubmitEvent) {
        submitted.preventDefault();
        if (await run("calendar.sharing.set", { address: address.trim(), role })) address = "";
    }
</script>

<section class="sharing" aria-label="Calendar sharing for {username}">
    <p class="muted">Who can see your main calendar, and how much of it. Kept by Microsoft 365; a change takes effect at once.</p>
    {#if problem}
        <p class="problem" role="alert">{problem}</p>
    {/if}
    {#if shares}
        <ul>
            {#each shares as each (each.id)}
                {@const who = each.address ?? "Everyone in your organisation"}
                <li>
                    <span class="who">{each.address && each.name && each.name !== each.address ? `${each.name} · ` : ""}{who}</span>
                    <select aria-label={`What ${who} can see`} value={each.role} disabled={busy} onchange={(event) => run("calendar.sharing.set", { share_id: each.id, role: event.currentTarget.value })}>
                        {#each [...new Set([...each.allowed_roles, each.role])] as allowed (allowed)}
                            <option value={allowed}>{ROLES[allowed] ?? allowed}</option>
                        {/each}
                    </select>
                    {#if each.removable}
                        <button type="button" aria-label={`Stop sharing with ${who}`} disabled={busy} onclick={() => run("calendar.sharing.remove", { share_id: each.id })}>Remove</button>
                    {/if}
                </li>
            {/each}
        </ul>
        <form onsubmit={add}>
            <input type="email" aria-label="Address of someone to share with" placeholder="someone@example.com" bind:value={address} />
            <select aria-label="What they can see" bind:value={role}>
                {#each ["freeBusyRead", "limitedRead", "read", "write"] as each (each)}
                    <option value={each}>{ROLES[each]}</option>
                {/each}
            </select>
            <button type="submit" class="primary" disabled={busy || !address.trim()}>Share</button>
        </form>
    {:else if !problem}
        <p class="muted" role="status">Reading from Microsoft 365…</p>
    {/if}
</section>

<style>
    .sharing {
        margin: 0.75rem 0 0 3rem;
        display: flex;
        flex-direction: column;
        gap: 0.5rem;
        max-width: 38rem;
    }

    p {
        margin: 0;
        font-size: 0.8rem;
    }

    ul {
        margin: 0;
        padding: 0;
        list-style: none;
    }

    li,
    form {
        display: flex;
        align-items: center;
        gap: 0.5rem;
        padding: 0.15rem 0;
    }

    .who,
    input {
        flex: 1;
        min-width: 0;
    }

    input,
    select {
        font: inherit;
        padding: 0.3rem;
    }

    .problem {
        padding: 0.5rem 0.75rem;
        background: color-mix(in srgb, var(--danger) 14%, var(--surface));
        font-size: 0.85rem;
    }
</style>
