<script lang="ts">
    import { Check, CircleHelp, X } from "@lucide/svelte";
    import type { Answer } from "./calendar";

    interface Props {
        /** The answer given so far, as Microsoft 365 names it. */
        response?: string | null;
        /** Answer the invitation. Resolves once it has been taken, and throws if it was not. */
        onAnswer: (answer: Answer, comment: string, tell: boolean) => Promise<void>;
    }

    let { response = null, onAnswer }: Props = $props();
    let comment = $state("");
    let tell = $state(true);
    let sending = $state<Answer | null>(null);
    let problem = $state<string | null>(null);

    const CHOICES: { answer: Answer; label: string; given: string; icon: typeof Check }[] = [
        { answer: "accept", label: "Accept", given: "accepted", icon: Check },
        { answer: "tentative", label: "Tentative", given: "tentativelyAccepted", icon: CircleHelp },
        { answer: "decline", label: "Decline", given: "declined", icon: X },
    ];

    async function give(answer: Answer) {
        sending = answer;
        problem = null;
        try {
            await onAnswer(answer, tell ? comment.trim() : "", tell);
            comment = "";
        } catch (failure) {
            problem = failure && typeof failure === "object" && "message" in failure ? String(failure.message) : String(failure);
        } finally {
            sending = null;
        }
    }
</script>

<div class="rsvp" role="group" aria-label="Answer the invitation">
    <div class="choices">
        {#each CHOICES as each (each.answer)}
            {@const Icon = each.icon}
            <button type="button" class:given={response === each.given} aria-pressed={response === each.given} disabled={sending !== null} onclick={() => give(each.answer)}>
                <Icon size={15} aria-hidden="true" />{sending === each.answer ? "Sending…" : each.label}
            </button>
        {/each}
        <details>
            <summary>Options</summary>
            <label class="tell"><input type="checkbox" bind:checked={tell} /> Tell the organiser</label>
            <textarea aria-label="Note to the organiser" placeholder="A note to the organiser (optional)" rows="2" maxlength="2000" disabled={!tell} bind:value={comment}></textarea>
        </details>
    </div>
    {#if problem}<p class="problem" role="alert">{problem}</p>{/if}
</div>

<style>
    .rsvp {
        margin-top: 0.4rem;
    }

    .choices {
        display: flex;
        flex-wrap: wrap;
        align-items: flex-start;
        gap: 0.35rem;
    }

    button {
        display: inline-flex;
        align-items: center;
        gap: 0.3rem;
        font-size: 0.85rem;
    }

    button.given {
        background: var(--accent);
        border-color: var(--accent);
        color: Canvas;
    }

    details {
        font-size: 0.85rem;
    }

    summary {
        padding: 0.3rem 0.4rem;
        cursor: default;
    }

    .tell {
        display: block;
        margin: 0.2rem 0;
    }

    textarea {
        width: 16rem;
        max-width: 100%;
        font: inherit;
    }

    .problem {
        margin: 0.3rem 0 0;
        font-size: 0.85rem;
        color: var(--danger);
    }
</style>
