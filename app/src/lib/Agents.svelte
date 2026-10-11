<script lang="ts">
    import { execute, say } from "./commands";

    interface Entry {
        at: string;
        client: string;
        command: string;
        args: unknown;
        outcome: "ok" | "refused" | "failed";
        detail: string;
    }

    let entries = $state<Entry[] | null>(null);
    let problem = $state<string | null>(null);

    async function load() {
        try {
            entries = (await execute<{ entries: Entry[] }>("app.agent_log.list", { limit: 200 })).entries;
            problem = null;
        } catch (failure) {
            problem = say(failure);
        }
    }

    $effect(() => {
        load();
    });

    const when = (at: string) => new Date(at).toLocaleString(undefined, { dateStyle: "medium", timeStyle: "medium" });
    const outcomes = { ok: "Ran", refused: "Not allowed", failed: "Did not work" };
</script>

<section class="agents">
    <header>
        <h1>Agents</h1>
        <button type="button" onclick={load}>Refresh</button>
    </header>
    <p class="muted">
        Everything an AI agent has asked Brolga to do through its MCP server, whether or not it was allowed. An agent can only read unless you start the
        server with more: <code>--access draft</code> or <code>--access full</code>, and <code>--allow-consequential</code> before it may send.
    </p>

    {#if problem}
        <p class="problem" role="alert">{problem}</p>
    {:else if entries === null}
        <p class="muted" role="status">Loading…</p>
    {:else if entries.length === 0}
        <p class="muted">No agent has asked for anything yet.</p>
    {:else}
        <table>
            <thead>
                <tr><th>When</th><th>Agent</th><th>Command</th><th>Outcome</th><th>Asked with</th></tr>
            </thead>
            <tbody>
                {#each entries as each, index (index)}
                    <tr class={each.outcome}>
                        <td>{when(each.at)}</td>
                        <td>{each.client}</td>
                        <td><code>{each.command}</code></td>
                        <td title={each.detail}>{outcomes[each.outcome]}{#if each.detail}<span class="muted why">{each.detail}</span>{/if}</td>
                        <td><code class="args">{JSON.stringify(each.args)}</code></td>
                    </tr>
                {/each}
            </tbody>
        </table>
    {/if}
</section>

<style>
    .agents {
        padding: 1.5rem;
        max-width: 72rem;
    }

    header {
        display: flex;
        align-items: center;
        justify-content: space-between;
    }

    h1 {
        margin: 0;
        font-size: 1.3rem;
    }

    table {
        width: 100%;
        border-collapse: collapse;
        font-size: 0.85rem;
        table-layout: fixed;
    }

    th,
    td {
        text-align: left;
        vertical-align: top;
        padding: 0.4rem 0.6rem 0.4rem 0;
        border-bottom: 1px solid var(--line);
        overflow-wrap: anywhere;
    }

    th:nth-child(1) {
        width: 11rem;
    }

    th:nth-child(2) {
        width: 9rem;
    }

    th:nth-child(4) {
        width: 16rem;
    }

    tr.refused td:nth-child(4),
    tr.failed td:nth-child(4) {
        color: var(--danger);
    }

    .why {
        display: block;
        font-size: 0.75rem;
    }

    .args {
        font-size: 0.75rem;
    }

    .problem {
        padding: 0.5rem 0.75rem;
        background: color-mix(in srgb, var(--danger) 14%, var(--surface));
    }
</style>
