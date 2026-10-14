<script lang="ts">
    import { execute, say } from "./commands";

    interface Props {
        accountId: string;
        username: string;
    }

    let { accountId, username }: Props = $props();

    let signature = $state<{ text: string; on_replies: boolean } | null>(null);
    let said = $state<string | null>(null);

    $effect(() => {
        const of = accountId;
        signature = null;
        said = null;
        execute<{ text: string; on_replies: boolean }>("mail.signature.get", { account_id: of })
            .then((got) => {
                if (of === accountId) signature = got;
            })
            .catch((failure) => (said = say(failure)));
    });

    async function save() {
        if (!signature) return;
        try {
            await execute("mail.signature.set", { account_id: accountId, text: signature.text, on_replies: signature.on_replies });
            said = "Signature saved. New messages will start with it.";
        } catch (failure) {
            said = say(failure);
        }
    }
</script>

{#if signature}
    <form class="signature" aria-label="Signature for {username}" onsubmit={(event) => (event.preventDefault(), save())}>
        <textarea bind:value={signature.text} aria-label="Signature" rows="6" placeholder="Your name, and whatever else should end your messages" oninput={() => (said = null)}></textarea>
        <label><input type="checkbox" bind:checked={signature.on_replies} /> Also on replies and forwards</label>
        <p class="muted">Kept on this computer. Plain text for now.</p>
        <div>
            <button type="submit" class="primary">Save</button>
        </div>
    </form>
{/if}
{#if said}
    <p class="said muted" role="status">{said}</p>
{/if}

<style>
    .signature {
        display: flex;
        flex-direction: column;
        gap: 0.6rem;
        max-width: 36rem;
    }

    textarea {
        font: inherit;
        padding: 0.5rem;
        resize: vertical;
    }

    p {
        margin: 0;
        font-size: 0.8rem;
    }

    .said {
        margin-top: 0.6rem;
    }
</style>
