<script lang="ts">
    import { execute, say } from "./commands";

    interface Props {
        accountId: string;
        /** The computer that makes approved calls, and when it was last seen. */
        processor: string | null;
        lastActive: string | null;
    }

    interface Step {
        server: string;
        tool: string;
        inputs: Record<string, unknown>;
    }

    interface Looked {
        server: string;
        tool: string;
        inputs: Record<string, unknown>;
        ok: boolean;
        result: string;
    }

    interface Approval {
        id: string;
        state: "waiting" | "approved" | "rejected";
        note: string | null;
        decided_by: string | null;
        decided_at: string | null;
        subject: string | null;
        from: string | null;
        instruction: string | null;
        steps: Step[];
        looked: Looked[] | null;
        reason: string | null;
        outcome: { state: string; text: string } | null;
    }

    let { accountId, processor, lastActive }: Props = $props();
    let approvals = $state<Approval[]>([]);
    let problem = $state<string | null>(null);
    let deciding = $state<string | null>(null);

    async function read() {
        try {
            approvals = (await execute<{ approvals: Approval[] }>("mail.instructions.approvals.list", { account_id: accountId })).approvals;
        } catch (failure) {
            problem = say(failure);
        }
    }

    $effect(() => {
        read();
    });

    async function decide(some: Approval[], approve: boolean) {
        deciding = some.length === 1 ? some[0].id : "all";
        problem = null;
        const troubles: string[] = [];
        for (const each of some) {
            try {
                await execute("mail.instructions.approvals.decide", { account_id: accountId, approval_id: each.id, approve });
            } catch (failure) {
                troubles.push(say(failure));
            }
        }
        if (troubles.length) problem = troubles.join(" ");
        deciding = null;
        await read();
    }

    const waiting = $derived(approvals.filter((a) => a.state === "waiting"));
    const decided = $derived(approvals.filter((a) => a.state !== "waiting"));
    /** An input that stands for what an earlier step returns. */
    const fromStep = (value: unknown): { from_step: number; field: string } | null =>
        value && typeof value === "object" && "from_step" in value && "field" in value ? (value as { from_step: number; field: string }) : null;
    const shown = (value: unknown) => {
        const from = fromStep(value);
        if (from) return `what step ${from.from_step} returns (${from.field})`;
        return typeof value === "string" ? value : JSON.stringify(value);
    };
    const called = (a: Approval) => a.steps.map((s) => `${s.server} ${s.tool}`).join(", then ");
    const seen = (iso: string | null) => (iso ? new Date(iso).toLocaleString(undefined, { day: "numeric", month: "short", hour: "numeric", minute: "2-digit" }) : "never");
    const came = (a: Approval) =>
        a.state === "rejected"
            ? "Rejected"
            : !a.outcome
              ? `Approved, waiting for ${processor ?? "the processing computer"} (last active ${seen(lastActive)})`
              : a.outcome.state === "done" ? `Made: ${a.outcome.text}` : a.outcome.state === "changed" ? "Not made: it was changed after it was put up" : `Did not work: ${a.outcome.text}`;
</script>

{#if problem}
    <p class="problem" role="alert">{problem}</p>
{/if}
{#if waiting.length > 0}
    <div class="approvals" aria-label="Calls waiting for approval">
        <p><strong>{waiting.length === 1 ? "A call waits" : `${waiting.length} calls wait`} for your approval.</strong> What a call holds was filled in from the email: check it before approving.</p>
        <ul>
            {#each waiting as each (each.id)}
                <li aria-label={`${called(each)} for ${each.subject ?? "a message"}`}>
                    <span><strong>{called(each)}</strong> for “{each.subject ?? ""}”{each.from ? `, from ${each.from}` : ""}</span>
                    <span class="muted">{each.instruction}{each.reason ? `: ${each.reason}` : ""}</span>
                    {#if each.note}<span class="warning">{each.note}</span>{/if}
                    <ol class="steps" aria-label="Steps">
                        {#each each.steps as step, n (n)}
                            <li>
                                <span>{each.steps.length > 1 ? `${n + 1}. ` : ""}{step.server} {step.tool}</span>
                                <dl>
                                    {#each Object.entries(step.inputs ?? {}) as [name, value] (name)}
                                        <dt>{name}</dt>
                                        <dd class:from={fromStep(value)}>{shown(value)}</dd>
                                    {/each}
                                </dl>
                            </li>
                        {/each}
                    </ol>
                    {#if each.looked?.length}
                        <details>
                            <summary>Looked up first ({each.looked.length})</summary>
                            <ul class="looked" aria-label="Looked up first">
                                {#each each.looked as l, n (n)}
                                    <li>
                                        <span>{l.server} {l.tool} {JSON.stringify(l.inputs)}{l.ok ? "" : " (did not work)"}</span>
                                        <pre>{l.result}</pre>
                                    </li>
                                {/each}
                            </ul>
                        </details>
                    {/if}
                    <div class="buttons">
                        <button type="button" class="primary" disabled={deciding !== null} onclick={() => decide([each], true)}>Approve</button>
                        <button type="button" disabled={deciding !== null} onclick={() => decide([each], false)}>Reject</button>
                    </div>
                </li>
            {/each}
        </ul>
        {#if waiting.length > 1}
            <div class="buttons">
                <button type="button" disabled={deciding !== null} onclick={() => decide(waiting, true)}>Approve all</button>
                <button type="button" disabled={deciding !== null} onclick={() => decide(waiting, false)}>Reject all</button>
            </div>
        {/if}
    </div>
{/if}
{#if decided.length > 0}
    <details class="log">
        <summary>Calls decided</summary>
        <ul aria-label="Calls decided">
            {#each decided as each (each.id)}
                <li>
                    <span>{called(each)} for “{each.subject ?? ""}”</span>
                    <span class="muted">{came(each)}{each.decided_by ? ` · decided on ${each.decided_by}` : ""}</span>
                </li>
            {/each}
        </ul>
    </details>
{/if}

<style>
    .approvals {
        padding: 0.5rem 0.75rem;
        border: 1px solid var(--line);
        background: color-mix(in srgb, var(--accent, #3b82f6) 8%, var(--surface));
        display: flex;
        flex-direction: column;
        gap: 0.4rem;
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

    li {
        display: flex;
        flex-direction: column;
        gap: 0.25rem;
        padding: 0.4rem 0;
        border-bottom: 1px solid var(--line);
        font-size: 0.85rem;
    }

    dl {
        margin: 0;
        display: grid;
        grid-template-columns: max-content 1fr;
        gap: 0.1rem 0.6rem;
        font-size: 0.8rem;
    }

    .steps,
    .looked {
        margin: 0;
        padding: 0;
        list-style: none;
    }

    .steps li,
    .looked li {
        border: 0;
        padding: 0.2rem 0;
    }

    dd.from {
        font-style: italic;
    }

    pre {
        margin: 0;
        font-size: 0.75rem;
        white-space: pre-wrap;
        overflow-wrap: anywhere;
        max-height: 8rem;
        overflow: auto;
    }

    dt {
        color: var(--muted, inherit);
    }

    dd {
        margin: 0;
        white-space: pre-wrap;
        overflow-wrap: anywhere;
    }

    .buttons {
        display: flex;
        gap: 0.5rem;
    }

    .warning {
        font-size: 0.8rem;
    }

    .problem {
        padding: 0.5rem 0.75rem;
        background: color-mix(in srgb, var(--danger) 14%, var(--surface));
    }
</style>
