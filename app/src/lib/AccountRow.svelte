<script lang="ts">
    import type { Account } from "./accounts";
    import { execute, say } from "./commands";

    interface Props {
        account: Account;
        busy: boolean;
        onSignInAgain: (account: Account) => void;
        onRemove: (account: Account) => void;
    }

    let { account, busy, onSignInAgain, onRemove }: Props = $props();
    let confirming = $state(false);

    /** The signature being edited, while its editor is open. */
    let signature = $state<{ text: string; on_replies: boolean } | null>(null);
    let signatureSaid = $state<string | null>(null);

    async function editSignature() {
        if (signature) {
            signature = null;
            return;
        }
        signatureSaid = null;
        try {
            signature = await execute("mail.signature.get", { account_id: account.id });
        } catch (failure) {
            signatureSaid = say(failure);
        }
    }

    async function saveSignature() {
        if (!signature) return;
        try {
            await execute("mail.signature.set", { account_id: account.id, text: signature.text, on_replies: signature.on_replies });
            signature = null;
            signatureSaid = "Signature saved. New messages will start with it.";
        } catch (failure) {
            signatureSaid = say(failure);
        }
    }

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
    <div class="row">
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
        <button type="button" aria-expanded={signature !== null} onclick={editSignature}>Signature</button>
        <button type="button" disabled={busy} onclick={() => (confirming = true)}>Remove</button>
    {/if}
    </div>

    {#if signature}
        <form class="signature" aria-label="Signature for {account.username}" onsubmit={(event) => (event.preventDefault(), saveSignature())}>
            <textarea bind:value={signature.text} aria-label="Signature" rows="5" placeholder="Your name, and whatever else should end your messages"></textarea>
            <label><input type="checkbox" bind:checked={signature.on_replies} /> Also on replies and forwards</label>
            <p class="muted">Kept on this computer. Plain text for now.</p>
            <div>
                <button type="submit" class="primary">Save</button>
                <button type="button" onclick={() => (signature = null)}>Cancel</button>
            </div>
        </form>
    {/if}
    {#if signatureSaid}
        <p class="said muted" role="status">{signatureSaid}</p>
    {/if}
</li>

<style>
    li {
        padding: 0.75rem 1rem;
        border-bottom: 1px solid var(--line);
    }

    .row {
        display: flex;
        align-items: center;
        gap: 0.75rem;
    }

    .signature {
        margin: 0.75rem 0 0 3rem;
        display: flex;
        flex-direction: column;
        gap: 0.5rem;
    }

    .signature textarea {
        font: inherit;
        padding: 0.5rem;
        resize: vertical;
    }

    .signature p,
    .said {
        margin: 0;
        font-size: 0.8rem;
    }

    .signature div {
        display: flex;
        gap: 0.5rem;
    }

    .said {
        margin: 0.5rem 0 0 3rem;
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
