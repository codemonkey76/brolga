<script lang="ts">
    import { untrack } from "svelte";
    import { execute, say } from "./commands";
    import { type Folder, listFolders } from "./mail";

    interface Props {
        accountId: string;
    }

    interface Estimate {
        messages: number;
        estimated_cost_usd: number | null;
        estimated_minutes: number | null;
        not_synced: string[];
    }

    interface Run {
        id: string;
        state: "previewing" | "previewed" | "applying" | "applied" | "cancelled" | "undone";
        total: number;
        counts: Record<string, number>;
        actions: Record<string, number>;
        undo_until: string | null;
        error: string | null;
    }

    interface Item {
        message_id: string;
        subject: string;
        from: { name: string; address: string };
        received_at: string;
        state: string;
        decisions: { action: string; folder?: string; categories?: string[]; due?: string | null; read?: boolean; title?: string; list?: string; instruction: string; reason: string }[];
        error: string | null;
    }

    const PAGE = 50;

    let { accountId }: Props = $props();
    let folders = $state<Folder[]>([]);
    let scope = $state({ folder: "", subfolders: false, from: "", to: "", tasks: false });
    let estimate = $state<Estimate | null>(null);
    let current = $state<Run | null>(null);
    let items = $state<Item[]>([]);
    let more = $state(false);
    let problem = $state<string | null>(null);
    let busy = $state(false);
    /** Applying or undoing, until it is confirmed. */
    let confirming = $state<"apply" | "undo" | null>(null);

    // Read afresh for each mailbox, and for nothing else: what is read here
    // is not what the effect follows.
    $effect(() => {
        const mailbox = accountId;
        untrack(() => {
            listFolders(mailbox)
                .then((found) => {
                    folders = found;
                    scope.folder = found.find((each) => each.well_known === "inbox")?.id ?? found[0]?.id ?? "";
                })
                .catch(() => {});
            latest();
        });
    });

    /** While a run is under way it is read again every few seconds. */
    $effect(() => {
        if (current?.state !== "previewing" && current?.state !== "applying") return;
        const timer = setInterval(latest, 2000);
        return () => clearInterval(timer);
    });

    async function latest() {
        try {
            const was = current?.state;
            current = (await execute<{ run: Run | null }>("mail.instructions.run.status", { account_id: accountId })).run;
            if (current && current.state !== was && current.state !== "previewing") await page(0);
        } catch (failure) {
            problem = say(failure);
        }
    }

    async function page(offset: number) {
        if (!current) return;
        const found = (await execute<{ items: Item[] }>("mail.instructions.run.items", { account_id: accountId, run_id: current.id, filter: "changes", offset, limit: PAGE })).items;
        items = offset === 0 ? found : [...items, ...found];
        more = found.length === PAGE;
    }

    const scoped = () => {
        const args: Record<string, unknown> = { account_id: accountId, folder_id: scope.folder, subfolders: scope.subfolders };
        if (scope.from) args.received_from = scope.from;
        if (scope.to) args.received_to = scope.to;
        if (scope.tasks) args.include_tasks = true;
        return args;
    };

    async function estimateIt() {
        problem = null;
        try {
            estimate = await execute<Estimate>("mail.instructions.run.estimate", scoped());
        } catch (failure) {
            problem = say(failure);
        }
    }

    async function act(command: string, args: Record<string, unknown>) {
        busy = true;
        problem = null;
        confirming = null;
        try {
            await execute(command, { account_id: accountId, ...args });
            estimate = null;
        } catch (failure) {
            problem = say(failure);
        } finally {
            busy = false;
            await latest();
        }
    }

    const done = (run: Run) => run.total - (run.counts.pending ?? 0) - (run.state === "applying" ? (run.counts.decided ?? 0) : 0);
    const when = (iso: string) => new Date(iso).toLocaleString(undefined, { weekday: "short", day: "numeric", month: "short", hour: "numeric", minute: "2-digit" });
    const said = (actions: Record<string, number>) =>
        Object.entries(actions)
            .map(([action, n]) => `${({ move: "move", set_categories: "categorise", flag: "flag", set_read: "mark read or unread", create_task: "make a task for" } as Record<string, string>)[action] ?? action} ${n}`)
            .join(", ") || "change nothing";
    const decision = (each: Item["decisions"][number]) =>
        each.action === "create_task"
            ? `Make a task in ${each.list}: ${each.title}${each.due ? `, due ${each.due}` : ""}`
            : each.action === "move"
            ? `Move to ${each.folder}`
            : each.action === "set_categories"
              ? `Categorise ${each.categories?.join(", ")}`
              : each.action === "flag"
                ? `Flag${each.due ? `, due ${each.due}` : ""}`
                : each.read
                  ? "Mark read"
                  : "Mark unread";
    const minutes = (n: number) => (n < 90 ? `${n} minutes` : `${Math.round(n / 60)} hours`);
</script>

<section class="run" aria-label="Run over existing mail">
    <h3>Run over mail already here</h3>
    {#if problem}
        <p class="problem" role="alert">{problem}</p>
    {/if}

    {#if current && (current.state === "previewing" || current.state === "applying")}
        <p role="status">
            {current.state === "previewing" ? "Previewing" : "Applying"}: {done(current)} of {current.total} messages.
            {#if current.error}<span class="warning">Waiting: {current.error}</span>{/if}
        </p>
        <progress max={current.total} value={done(current)}></progress>
        <div class="buttons"><button type="button" disabled={busy} onclick={() => act("mail.instructions.run.cancel", { run_id: current?.id })}>Cancel</button></div>
    {:else}
        <form class="scope" aria-label="What to run over" onsubmit={(event) => (event.preventDefault(), estimateIt())}>
            <label class="stacked">
                Folder
                <select aria-label="Folder" bind:value={scope.folder} onchange={() => (estimate = null)}>
                    {#each folders as each (each.id)}
                        <option value={each.id}>{"  ".repeat(each.depth)}{each.name}</option>
                    {/each}
                </select>
            </label>
            <label><input type="checkbox" bind:checked={scope.subfolders} onchange={() => (estimate = null)} /> And the folders inside it</label>
            <label><input type="checkbox" bind:checked={scope.tasks} /> Let instructions make To Do tasks too</label>
            <div class="days">
                <label class="stacked">Received from <input type="date" bind:value={scope.from} onchange={() => (estimate = null)} /></label>
                <label class="stacked">to <input type="date" bind:value={scope.to} onchange={() => (estimate = null)} /></label>
            </div>
            <div class="buttons"><button type="submit" disabled={!scope.folder}>See how many</button></div>
        </form>

        {#if estimate}
            <p role="status">
                {estimate.messages} message{estimate.messages === 1 ? "" : "s"}{estimate.estimated_cost_usd !== null ? `, about US$${estimate.estimated_cost_usd.toFixed(2)} at Anthropic` : ""}{estimate.estimated_minutes !== null ? `, about ${minutes(estimate.estimated_minutes)}` : ""}.
                Previewing changes nothing.
            </p>
            {#if estimate.not_synced.length}
                <p class="warning">Not every message in {estimate.not_synced.join(", ")} has been fetched yet. Let the sync finish first.</p>
            {/if}
            <div class="buttons">
                <button type="button" class="primary" disabled={busy || estimate.messages === 0 || estimate.not_synced.length > 0} onclick={() => act("mail.instructions.run.preview", scoped())}>Preview</button>
            </div>
        {/if}
    {/if}

    {#if current && current.state === "previewed"}
        <p>The preview would {said(current.actions)}; {current.counts.nothing ?? 0} left as they are{current.counts.failed ? `, ${current.counts.failed} could not be asked about` : ""}.</p>
        {#if confirming === "apply"}
            <p>Do this to {current.counts.decided ?? 0} messages? It can be undone for three days.</p>
            <div class="buttons">
                <button type="button" class="primary" disabled={busy} onclick={() => act("mail.instructions.run.apply", { run_id: current?.id })}>Yes, apply it</button>
                <button type="button" onclick={() => (confirming = null)}>Not yet</button>
            </div>
        {:else}
            <div class="buttons">
                <button type="button" class="primary" disabled={busy || !current.counts.decided} onclick={() => (confirming = "apply")}>Apply</button>
                <button type="button" disabled={busy} onclick={() => act("mail.instructions.run.cancel", { run_id: current?.id })}>Set it aside</button>
            </div>
        {/if}
    {:else if current && current.state === "applied"}
        <p>
            Applied to {current.counts.applied ?? 0} messages{current.counts.skipped ? `; ${current.counts.skipped} skipped, changed since the preview` : ""}{current.counts.unapplied ? `; ${current.counts.unapplied} not done in full` : ""}.
        </p>
        {#if current.undo_until}
            {#if confirming === "undo"}
                <p>Undo everything this run did? Messages changed since are left as they are.</p>
                <div class="buttons">
                    <button type="button" class="danger" disabled={busy} onclick={() => act("mail.instructions.run.undo", { run_id: current?.id })}>Undo the run</button>
                    <button type="button" onclick={() => (confirming = null)}>Keep it</button>
                </div>
            {:else}
                <div class="buttons"><button type="button" onclick={() => (confirming = "undo")}>Undo the run</button></div>
                <p class="muted">It can be undone until {when(current.undo_until)}.</p>
            {/if}
        {/if}
    {:else if current && current.state === "undone"}
        <p class="muted">The last run was undone.</p>
    {/if}

    {#if current && current.state !== "previewing" && items.length > 0}
        <ul aria-label="What the run does">
            {#each items as item (item.message_id)}
                <li>
                    <span class="what">
                        <span class="name">{item.subject || "(no subject)"}</span>
                        <span class="muted">{item.from.name || item.from.address} · {new Date(item.received_at).toLocaleDateString()}</span>
                        {#each item.decisions as each, n (n)}
                            <span>{decision(each)} <span class="muted">({each.instruction}: {each.reason})</span></span>
                        {/each}
                        {#if item.state === "skipped" || item.state === "unapplied"}
                            <span class="warning">{item.error}</span>
                        {/if}
                    </span>
                </li>
            {/each}
        </ul>
        {#if more}
            <div class="buttons"><button type="button" onclick={() => page(items.length)}>Show more</button></div>
        {/if}
    {/if}
</section>

<style>
    .run {
        display: flex;
        flex-direction: column;
        gap: 0.5rem;
        padding: 0.6rem 0.75rem;
        border: 1px solid var(--line);
        border-radius: 6px;
        font-size: 0.9rem;
    }

    h3 {
        margin: 0;
        font-size: 0.95rem;
    }

    p {
        margin: 0;
    }

    form,
    .stacked {
        display: flex;
        flex-direction: column;
        gap: 0.3rem;
    }

    .days,
    .buttons {
        display: flex;
        flex-wrap: wrap;
        gap: 0.5rem;
    }

    select,
    input[type="date"] {
        font: inherit;
        padding: 0.3rem;
    }

    progress {
        width: 100%;
    }

    ul {
        margin: 0;
        padding: 0;
        list-style: none;
        max-height: 24rem;
        overflow-y: auto;
    }

    li {
        padding: 0.35rem 0;
        border-bottom: 1px solid var(--line);
    }

    .what {
        display: flex;
        flex-direction: column;
        font-size: 0.85rem;
    }

    .name {
        font-weight: 600;
    }

    .warning {
        color: var(--danger);
    }

    .problem {
        padding: 0.5rem 0.75rem;
        background: color-mix(in srgb, var(--danger) 14%, var(--surface));
    }
</style>
