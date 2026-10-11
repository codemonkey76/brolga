<script lang="ts">
    import { execute } from "./commands";
    import type { Draft } from "./mail";

    interface Question {
        id: number;
        client: string;
        command: string;
        args: Record<string, unknown>;
    }

    /** What an agent is waiting on, the oldest first. One is shown at a time. */
    let questions = $state<Question[]>([]);
    /** For a message to be sent: who to and about what, so the answer is given knowing. */
    let draft = $state<Draft | null>(null);
    let looked = 0;

    const current = $derived(questions[0] ?? null);

    // Looking for questions every two seconds is also how the client knows
    // there is a window to ask in. With none looking, an agent is told no.
    $effect(() => {
        const look = async () => {
            try {
                questions = (await execute<{ questions: Question[] }>("app.confirmations.list")).questions;
            } catch {
                // Nothing to ask until the client can be reached again.
            }
        };
        look();
        const timer = setInterval(look, 2_000);
        return () => clearInterval(timer);
    });

    $effect(() => {
        const question = current;
        draft = null;
        if (!question || question.command !== "mail.drafts.send" || typeof question.args.draft_id !== "string") return;
        const mine = ++looked;
        execute<{ draft: Draft | null }>("mail.drafts.get", { draft_id: question.args.draft_id })
            .then((got) => mine === looked && (draft = got.draft))
            .catch(() => {});
    });

    async function answer(allow: boolean) {
        const question = current;
        if (!question) return;
        questions = questions.slice(1);
        await execute("app.confirmations.answer", { id: question.id, allow }).catch(() => {});
    }

    const SAID: Record<string, string> = {
        "mail.drafts.send": "send a message",
        "mail.messages.delete_permanently": "delete messages for good",
        "mail.automatic_replies.set": "change your automatic replies",
    };
    const people = (list: Draft["to"]) => list.map((p) => p.name || p.address).join(", ");
</script>

{#if current}
    <div class="veil">
        <div class="ask" role="alertdialog" aria-modal="true" aria-labelledby="ask-title">
            <h2 id="ask-title"><span class="who">{current.client}</span> wants to {SAID[current.command] ?? "do something that cannot be undone"}</h2>

            {#if draft}
                <dl>
                    <dt>To</dt>
                    <dd>{people([...draft.to, ...draft.cc, ...draft.bcc]) || "Nobody yet"}</dd>
                    <dt>Subject</dt>
                    <dd>{draft.subject || "(No subject)"}</dd>
                    {#if draft.files.length > 0}
                        <dt>Files</dt>
                        <dd>{draft.files.map((file) => file.name).join(", ")}</dd>
                    {/if}
                </dl>
            {/if}
            <p class="muted">
                The command is <code>{current.command}</code>, asked with <code>{JSON.stringify(current.args)}</code>. An AI agent is asking; nothing in an
                email it has read gives it the right to. If you did not expect this, say no.
            </p>

            <div class="buttons">
                <!-- svelte-ignore a11y_autofocus -->
                <button type="button" autofocus onclick={() => answer(false)}>Don't allow</button>
                <button type="button" class="primary" onclick={() => answer(true)}>Allow this once</button>
            </div>
            {#if questions.length > 1}
                <p class="muted more">{questions.length - 1} more waiting.</p>
            {/if}
        </div>
    </div>
{/if}

<style>
    .veil {
        position: fixed;
        inset: 0;
        z-index: 50;
        display: grid;
        place-items: center;
        background: color-mix(in srgb, CanvasText 35%, transparent);
    }

    .ask {
        width: min(34rem, calc(100vw - 2rem));
        padding: 1.25rem 1.5rem;
        background: var(--surface);
        border: 1px solid var(--line);
        border-radius: 0.6rem;
        box-shadow: 0 12px 40px color-mix(in srgb, CanvasText 30%, transparent);
    }

    h2 {
        margin: 0 0 0.75rem;
        font-size: 1.1rem;
    }

    .who {
        font-family: ui-monospace, monospace;
    }

    dl {
        display: grid;
        grid-template-columns: auto 1fr;
        gap: 0.25rem 0.75rem;
        margin: 0 0 0.75rem;
        padding: 0.6rem 0.75rem;
        background: var(--raised);
        border-radius: 0.4rem;
    }

    dt {
        color: var(--muted);
    }

    dd {
        margin: 0;
        overflow-wrap: anywhere;
    }

    p {
        margin: 0 0 1rem;
        font-size: 0.85rem;
        overflow-wrap: anywhere;
    }

    .buttons {
        display: flex;
        justify-content: flex-end;
        gap: 0.5rem;
    }

    .more {
        margin: 0.75rem 0 0;
        text-align: right;
    }
</style>
