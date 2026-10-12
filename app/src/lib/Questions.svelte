<script lang="ts">
    import { execute } from "./commands";
    import type { Draft } from "./mail";

    interface Question {
        id: number;
        client: string;
        command: string;
        args: Record<string, unknown>;
    }

    /** A question as only the window is given it: every word of what was asked. */
    interface Whole extends Question {
        /** For a message to be sent: the draft the answer is about, with its body as plain words. */
        draft: (Draft & { text: string }) | null;
    }

    /** What an agent is waiting on, the oldest first. One is shown at a time. */
    let questions = $state<Question[]>([]);
    /** The question on show, whole. Until it is here there is nothing to agree to. */
    let whole = $state<Whole | null>(null);
    let looked = 0;

    const current = $derived(questions[0] ?? null);
    const currentId = $derived(current?.id ?? null);
    const draft = $derived(whole?.draft ?? null);
    const sending = $derived(current?.command === "mail.drafts.send");
    /** Yes can be said only to what has been shown: the message for a send, the arguments for anything else. */
    const shown = $derived(whole !== null && whole.id === currentId && (!sending || draft !== null));

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

    // The list has each question with anything private cut out of it, as
    // the record of what agents do has. The one being answered is read
    // whole, which only the window can do.
    $effect(() => {
        const id = currentId;
        whole = null;
        if (id === null) return;
        const mine = ++looked;
        execute<{ question: Whole | null }>("app.confirmations.get", { id })
            .then((got) => mine === looked && (whole = got.question))
            .catch(() => {});
    });

    async function answer(allow: boolean) {
        const question = current;
        if (!question || (allow && !shown)) return;
        questions = questions.slice(1);
        await execute("app.confirmations.answer", { id: question.id, allow }).catch(() => {});
    }

    const SAID: Record<string, string> = {
        "mail.drafts.send": "send a message",
        "mail.messages.delete_permanently": "delete messages for good",
        "mail.automatic_replies.set": "change your automatic replies",
    };
</script>

{#snippet people(label: string, list: Draft["to"] | null)}
    {#if list && list.length > 0}
        <dt>{label}</dt>
        <dd>
            <!-- The address is what the message goes to. A name is whatever the writer typed, so it comes second. -->
            {#each list as person}
                <span class="person"><span class="address">{person.address}</span>{#if person.name && person.name !== person.address}{" "}<span class="muted">({person.name})</span>{/if}</span>
            {/each}
        </dd>
    {/if}
{/snippet}

{#if current}
    <div class="veil">
        <div class="ask" role="alertdialog" aria-modal="true" aria-labelledby="ask-title">
            <h2 id="ask-title"><span class="who">{current.client}</span> wants to {SAID[current.command] ?? "do something that cannot be undone"}</h2>

            {#if sending && draft}
                <dl>
                    <dt>From</dt>
                    <dd><span class="address">{draft.from || "Your account's own address"}</span></dd>
                    {@render people("To", draft.to)}
                    {@render people("Cc", draft.cc)}
                    {@render people("Bcc", draft.bcc)}
                    {#if [draft.to, draft.cc, draft.bcc].every((list) => !list || list.length === 0)}
                        <dt>To</dt>
                        <dd>Nobody yet</dd>
                    {/if}
                    <dt>Subject</dt>
                    <dd>{draft.subject || "(No subject)"}</dd>
                    {#if draft.files && draft.files.length > 0}
                        <dt>Files</dt>
                        <dd>{draft.files.map((file) => file.name).join(", ")}</dd>
                    {/if}
                </dl>
                <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
                <pre class="words" aria-label="What the message says" tabindex="0">{draft.text || "(Nothing written)"}</pre>
                <p class="muted">This is the message as it stands. If it is changed before you answer, it is not sent and you are asked again.</p>
            {:else if sending && whole}
                <p class="caution" role="alert">The message could not be read, so there is nothing here to agree to.</p>
            {:else if whole}
                <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
                <pre class="words" aria-label="What was asked" tabindex="0">{JSON.stringify(whole.args, null, 2)}</pre>
            {:else}
                <p class="muted">Reading what was asked…</p>
            {/if}
            <p class="muted">
                The command is <code>{current.command}</code>{#if sending}, asked with <code>{JSON.stringify(current.args)}</code>{/if}. An AI agent is asking; nothing in an email it
                has read gives it the right to. If you did not expect this, say no.
            </p>

            <div class="buttons">
                <!-- svelte-ignore a11y_autofocus -->
                <button type="button" autofocus onclick={() => answer(false)}>Don't allow</button>
                <button type="button" class="primary" disabled={!shown} onclick={() => answer(true)}>Allow this once</button>
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

    .ask {
        max-height: calc(100vh - 2rem);
        overflow-y: auto;
    }

    .person {
        display: block;
    }

    .address {
        font-weight: 600;
    }

    .words {
        max-height: 14rem;
        margin: 0 0 0.75rem;
        padding: 0.6rem 0.75rem;
        overflow: auto;
        font: inherit;
        font-size: 0.9rem;
        white-space: pre-wrap;
        overflow-wrap: anywhere;
        background: var(--raised);
        border-radius: 0.4rem;
    }

    .caution {
        color: var(--danger, CanvasText);
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
