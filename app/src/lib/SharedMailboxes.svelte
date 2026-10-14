<script lang="ts">
    import { Plus, Trash } from "@lucide/svelte";
    import { type SharedMailbox, addShared, listShared, removeShared } from "./accounts";
    import { say } from "./commands";

    interface Props {
        accountId: string;
        username: string;
        /** A mailbox was shared or unshared, so what there is to read has changed. */
        onShared?: () => void;
    }

    let { accountId, username, onShared }: Props = $props();

    /** The mailboxes shared with this account. */
    let sharing = $state<SharedMailbox[]>([]);
    let address = $state("");
    let adding = $state(false);
    let problem = $state<string | null>(null);

    $effect(() => {
        const of = accountId;
        listShared()
            .catch(() => [])
            .then((all) => {
                if (of === accountId) sharing = all.filter((each) => each.account_id === of);
            });
    });

    async function share(submitted: SubmitEvent) {
        submitted.preventDefault();
        if (!address.trim()) return;
        adding = true;
        problem = null;
        try {
            const added = await addShared(accountId, address.trim());
            sharing = [...sharing, added];
            address = "";
            onShared?.();
        } catch (failure) {
            problem = say(failure);
        } finally {
            adding = false;
        }
    }

    /** Stop reading a shared mailbox. It stays in the list if that was refused. */
    async function unshare(mailbox: SharedMailbox) {
        problem = null;
        try {
            await removeShared(mailbox.id);
        } catch (failure) {
            problem = say(failure);
            return;
        }
        sharing = sharing.filter((each) => each.id !== mailbox.id);
        onShared?.();
    }
</script>

<section class="shared" aria-label="Shared mailboxes of {username}">
    <p class="muted">A mailbox someone has given you access to, or a person who has shared only their calendar. Microsoft 365 will not list them, so add each by its address.</p>
    {#if sharing.length > 0}
        <ul>
            {#each sharing as each (each.id)}
                <li>
                    <span>{each.name ? `${each.name} · ` : ""}{each.address}{each.calendar_only ? " (calendar only)" : ""}</span>
                    <button type="button" class="icon" aria-label={`Remove ${each.address}`} title="Remove" onclick={() => unshare(each)}><Trash size={15} aria-hidden="true" /></button>
                </li>
            {/each}
        </ul>
    {/if}
    <form onsubmit={share}>
        <input type="email" aria-label="Address of a shared mailbox" placeholder="shared@example.com" bind:value={address} />
        <button type="submit" class="primary" disabled={adding || !address.trim()}><Plus size={15} aria-hidden="true" />{adding ? "Looking…" : "Add"}</button>
    </form>
    {#if problem}<p class="problem" role="alert">{problem}</p>{/if}
</section>

<style>
    .shared {
        display: flex;
        flex-direction: column;
        gap: 0.6rem;
        max-width: 36rem;
    }

    p,
    ul {
        margin: 0;
    }

    ul {
        padding: 0;
        list-style: none;
        border: 1px solid var(--line);
        border-radius: 0.5rem;
    }

    li {
        display: flex;
        align-items: center;
        justify-content: space-between;
        gap: 0.5rem;
        padding: 0.4rem 0.4rem 0.4rem 0.75rem;
        border-bottom: 1px solid var(--line);
    }

    li:last-child {
        border-bottom: 0;
    }

    form {
        display: flex;
        gap: 0.5rem;
    }

    input {
        flex: 1;
        font: inherit;
    }

    button {
        display: inline-flex;
        align-items: center;
        gap: 0.3rem;
    }

    .icon {
        border: 0;
        background: none;
        padding: 0.3rem;
    }

    .problem {
        color: var(--danger);
    }
</style>
