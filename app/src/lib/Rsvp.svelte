<script lang="ts">
    import { Check, CircleHelp, X } from "@lucide/svelte";
    import type { Answer, Proposed } from "./calendar";

    interface Props {
        /** The answer given so far, as Microsoft 365 names it. */
        response?: string | null;
        /** Answer the invitation. Resolves once it has been taken, and throws if it was not. */
        onAnswer: (answer: Answer, comment: string, tell: boolean, proposed?: Proposed) => Promise<void>;
        /** When the meeting is, in UTC: where another time proposed starts from. */
        start?: string | null;
        end?: string | null;
    }

    let { response = null, onAnswer, start = null, end = null }: Props = $props();

    const pad = (n: number) => String(n).padStart(2, "0");
    const dayOf = (date: Date) => `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}`;
    const timeOf = (date: Date) => `${pad(date.getHours())}:${pad(date.getMinutes())}`;
    /** Another time is being proposed with the answer. */
    let proposing = $state(false);
    // svelte-ignore state_referenced_locally
    let day = $state(start ? dayOf(new Date(start)) : "");
    // svelte-ignore state_referenced_locally
    let from = $state(start ? timeOf(new Date(start)) : "09:00");
    // svelte-ignore state_referenced_locally
    let to = $state(end ? timeOf(new Date(end)) : "10:00");
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
        problem = null;
        let proposed: Proposed | undefined;
        if (proposing) {
            proposed = { start: new Date(`${day}T${from}`), end: new Date(`${day}T${to}`) };
            if (Number.isNaN(+proposed.start) || Number.isNaN(+proposed.end) || proposed.end <= proposed.start) {
                problem = "Say the day, and a start and an end after it, for the time you propose.";
                return;
            }
        }
        sending = answer;
        try {
            // Another time goes to the organiser with the answer, so they are told.
            await onAnswer(answer, tell || proposing ? comment.trim() : "", tell || proposing, proposed);
            comment = "";
            proposing = false;
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
            <button type="button" class:given={response === each.given} aria-pressed={response === each.given} disabled={sending !== null || (proposing && each.answer === "accept")} onclick={() => give(each.answer)}>
                <Icon size={15} aria-hidden="true" />{sending === each.answer ? "Sending…" : each.label}
            </button>
        {/each}
        <details>
            <summary>Options</summary>
            <label class="tell"><input type="checkbox" bind:checked={tell} disabled={proposing} /> Tell the organiser</label>
            <label class="tell"><input type="checkbox" bind:checked={proposing} /> Propose another time</label>
            {#if proposing}
                <p class="when">
                    <input type="date" aria-label="Proposed day" bind:value={day} />
                    <input type="time" aria-label="Proposed start" bind:value={from} /> to
                    <input type="time" aria-label="Proposed end" bind:value={to} />
                </p>
                <p class="muted">Sent with Tentative or Decline.</p>
            {/if}
            <textarea aria-label="Note to the organiser" placeholder="A note to the organiser (optional)" rows="2" maxlength="2000" disabled={!tell && !proposing} bind:value={comment}></textarea>
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

    .when {
        display: flex;
        flex-wrap: wrap;
        align-items: center;
        gap: 0.3rem;
        margin: 0.2rem 0;
    }

    .when input {
        font: inherit;
    }

    .muted {
        margin: 0 0 0.3rem;
    }

    .problem {
        margin: 0.3rem 0 0;
        font-size: 0.85rem;
        color: var(--danger);
    }
</style>
