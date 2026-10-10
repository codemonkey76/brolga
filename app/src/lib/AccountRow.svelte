<script lang="ts">
    import type { Account } from "./accounts";

    interface Props {
        account: Account;
        busy: boolean;
        onSignInAgain: (account: Account) => void;
        onRemove: (account: Account) => void;
    }

    let { account, busy, onSignInAgain, onRemove }: Props = $props();
    let confirming = $state(false);

    const initials = $derived(
        (account.display_name ?? account.username)
            .split(/[\s.@]+/)
            .filter(Boolean)
            .slice(0, 2)
            .map((word) => word[0]?.toUpperCase() ?? "")
            .join(""),
    );
</script>

<li>
    <span class="avatar" aria-hidden="true">{initials}</span>
    <span class="who">
        <span class="name">{account.display_name ?? account.username}</span>
        <span class="muted">{account.username}</span>
    </span>

    {#if account.needs_sign_in}
        <span class="warning" role="status">Needs sign-in</span>
        <button type="button" class="primary" disabled={busy} onclick={() => onSignInAgain(account)}>Sign in again</button>
    {:else}
        <span class="ok">Signed in</span>
    {/if}

    {#if confirming}
        <button type="button" class="danger" disabled={busy} onclick={() => onRemove(account)}>Remove it?</button>
        <button type="button" onclick={() => (confirming = false)}>Keep</button>
    {:else}
        <button type="button" disabled={busy} onclick={() => (confirming = true)}>Remove</button>
    {/if}
</li>

<style>
    li {
        display: flex;
        align-items: center;
        gap: 0.75rem;
        padding: 0.75rem 1rem;
        border-bottom: 1px solid var(--line);
    }

    li:last-child {
        border-bottom: 0;
    }

    .avatar {
        flex: none;
        display: grid;
        place-items: center;
        width: 2.25rem;
        height: 2.25rem;
        border-radius: 50%;
        background: color-mix(in srgb, var(--accent) 22%, var(--surface));
        font-size: 0.85rem;
        font-weight: 600;
    }

    .who {
        flex: 1;
        min-width: 0;
        display: flex;
        flex-direction: column;
    }

    .who span {
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    }

    .name {
        font-weight: 600;
    }

    .ok,
    .warning {
        flex: none;
        font-size: 0.85rem;
    }

    .ok {
        color: var(--muted);
    }

    .warning {
        padding: 0.15rem 0.5rem;
        border-radius: 999px;
        background: color-mix(in srgb, var(--warn) 22%, var(--surface));
    }
</style>
