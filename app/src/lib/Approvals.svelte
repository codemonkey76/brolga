<script lang="ts">
    import { execute, say } from "./commands";

    interface Props {
        accountId: string;
        /** The computer that makes approved calls, and when it was last seen. */
        processor: string | null;
        lastActive: string | null;
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
        server: string | null;
        tool: string | null;
        inputs: Record<string, unknown>;
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
    const shown = (value: unknown) => (typeof value === "string" ? value : JSON.stringify(value));
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
                <li aria-label={`${each.server} ${each.tool} for ${each.subject ?? "a message"}`}>
                    <span><strong>{each.server} {each.tool}</strong> for “{each.subject ?? ""}”{each.from ? `, from ${each.from}` : ""}</span>
                    <span class="muted">{each.instruction}{each.reason ? `: ${each.reason}` : ""}</span>
                    {#if each.note}<span class="warning">{each.note}</span>{/if}
                    <dl>
                        {#each Object.entries(each.inputs ?? {}) as [name, value] (name)}
                            <dt>{name}</dt>
                            <dd>{shown(value)}</dd>
                        {/each}
                    </dl>
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
                    <span>{each.server} {each.tool} for “{each.subject ?? ""}”</span>
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
