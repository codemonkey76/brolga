<script lang="ts">
    import { Plus, Star } from "@lucide/svelte";
    import { type Account, signInAgain } from "./accounts";
    import { execute, say } from "./commands";
    import { type Task, type TaskList, completeTask, createTask, createTaskList, deleteTask, deleteTaskList, linkedMessage, listTaskLists, listTasks, renameTaskList, undoTasks, updateTask } from "./tasks";

    interface Props {
        accounts: Account[];
        /** A task to show, asked for elsewhere: by its reminder, say. */
        reveal?: { account_id: string; task_id: string } | null;
        onRevealed?: () => void;
        /** Open the email a task was made from, in Mail. */
        onMessage: (account_id: string, message_id: string) => void;
    }

    let { accounts, reveal = null, onRevealed, onMessage }: Props = $props();
    let accountId = $state<string | null>(null);
    let lists = $state<TaskList[]>([]);
    let needsSignIn = $state(false);
    let waiting = $state(0);
    let refused = $state<{ change: string; id: string; error: string }[]>([]);
    let listId = $state<string | null>(null);
    let tasks = $state<Task[]>([]);
    let withDone = $state(false);
    let chosen = $state<Task | null>(null);
    let adding = $state("");
    let problem = $state<string | null>(null);
    /** What can be undone, and what it was, for a moment. */
    let undoable = $state<{ id: string; said: string } | null>(null);
    let naming = $state<{ id: string | null; name: string } | null>(null);
    /** The list about to be deleted, until it is confirmed. */
    let deletingList = $state<string | null>(null);

    const account = $derived(accounts.find((each) => each.id === accountId) ?? accounts[0] ?? null);
    const list = $derived(lists.find((each) => each.id === listId) ?? null);

    async function load() {
        if (!account) return;
        try {
            const got = await listTaskLists(account.id);
            lists = got.lists;
            needsSignIn = got.needs_sign_in;
            waiting = got.changes_waiting;
            refused = got.changes_refused;
            if (!lists.some((each) => each.id === listId)) listId = lists[0]?.id ?? null;
            await loadTasks();
        } catch (failure) {
            problem = say(failure);
        }
    }

    async function loadTasks() {
        if (!account || !listId) {
            tasks = [];
            return;
        }
        tasks = await listTasks(account.id, listId, withDone);
        if (chosen) chosen = tasks.find((each) => each.id === chosen?.id) ?? null;
    }

    $effect(() => {
        void account?.id;
        load();
        // Kept up to date as syncs bring changes down, and as changes made here go up.
        const timer = setInterval(load, 15_000);
        return () => clearInterval(timer);
    });

    $effect(() => {
        if (!reveal) return;
        const { account_id, task_id } = reveal;
        onRevealed?.();
        (async () => {
            accountId = account_id;
            const task = await execute<{ task: Task }>("tasks.get", { account_id, task_id }).catch(() => null);
            if (!task) return;
            listId = task.task.list_id;
            await load();
            chosen = tasks.find((each) => each.id === task_id) ?? task.task;
        })();
    });

    async function act(run: () => Promise<{ undo_id?: string } | object>, said: string) {
        problem = null;
        try {
            const done = await run();
            undoable = "undo_id" in done && typeof done.undo_id === "string" ? { id: done.undo_id, said } : null;
        } catch (failure) {
            problem = say(failure);
        }
        await load();
    }

    async function add(submitted: SubmitEvent) {
        submitted.preventDefault();
        if (!account || !listId || !adding.trim()) return;
        const title = adding.trim();
        adding = "";
        await act(() => createTask(account.id, listId as string, title), `Added “${title}”.`);
    }

    async function undo() {
        if (!account || !undoable) return;
        const id = undoable.id;
        undoable = null;
        await act(() => undoTasks(account.id, id), "Undone.");
        undoable = null;
    }

    /** Save a change to the task being looked at, as it is made. */
    function save(changes: Record<string, unknown>, said = "Changed.") {
        if (!account || !chosen) return;
        const id = chosen.id;
        return act(() => updateTask(account.id, id, changes), said);
    }

    const reminderLocal = (iso: string | null) => {
        if (!iso) return "";
        const at = new Date(iso);
        const pad = (n: number) => String(n).padStart(2, "0");
        return `${at.getFullYear()}-${pad(at.getMonth() + 1)}-${pad(at.getDate())}T${pad(at.getHours())}:${pad(at.getMinutes())}`;
    };

    async function saveList(submitted: SubmitEvent) {
        submitted.preventDefault();
        if (!account || !naming?.name.trim()) return;
        const { id, name } = naming;
        naming = null;
        if (id) await act(() => renameTaskList(account.id, id, name.trim()), "Renamed.");
        else {
            await act(() => createTaskList(account.id, name.trim()), `Made “${name.trim()}”.`);
        }
    }

    const dueSaid = (day: string) => {
        const due = new Date(`${day}T00:00`);
        const today = new Date();
        today.setHours(0, 0, 0, 0);
        const days = Math.round((due.getTime() - today.getTime()) / 86_400_000);
        if (days === 0) return "Today";
        if (days === 1) return "Tomorrow";
        if (days === -1) return "Yesterday";
        return due.toLocaleDateString(undefined, { weekday: "short", day: "numeric", month: "short" });
    };
    const overdue = (task: Task) => !task.done && !!task.due && task.due < new Date().toISOString().slice(0, 10);
</script>

<div class="tasks">
    <aside aria-label="Task lists">
        {#if accounts.length > 1}
            <select aria-label="Account" value={account?.id} onchange={(event) => ((accountId = event.currentTarget.value), (listId = null), (chosen = null))}>
                {#each accounts as each (each.id)}
                    <option value={each.id}>{each.username}</option>
                {/each}
            </select>
        {/if}
        <ul>
            {#each lists as each (each.id)}
                <li>
                    <button type="button" class:current={each.id === listId} aria-current={each.id === listId} onclick={() => ((listId = each.id), (chosen = null), loadTasks())}>{each.name}</button>
                </li>
            {/each}
        </ul>
        {#if naming && naming.id === null}
            <form aria-label="New list" onsubmit={saveList}>
                <input type="text" aria-label="List name" bind:value={naming.name} maxlength="255" />
            </form>
        {:else}
            <button type="button" class="new" onclick={() => (naming = { id: null, name: "" })}><Plus size={15} aria-hidden="true" /> New list</button>
        {/if}
        {#if waiting > 0}
            <p class="muted" role="status">{waiting} change{waiting === 1 ? "" : "s"} waiting to reach To Do.</p>
        {/if}
    </aside>

    <section class="list" aria-label={list ? `Tasks in ${list.name}` : "Tasks"}>
        {#if needsSignIn && account}
            <div class="problem" role="alert">
                <p>This account's sign-in is from before Tasks, so Brolga may not read its To Do yet.</p>
                <button type="button" onclick={() => account && signInAgain(account).then(load).catch((failure) => (problem = say(failure)))}>Sign in again to use Tasks</button>
            </div>
        {/if}
        {#if problem}
            <p class="problem" role="alert">{problem}</p>
        {/if}
        {#each refused as each, n (n)}
            <p class="problem" role="alert">To Do would not take a change: {each.error}</p>
        {/each}

        {#if list}
            <header>
                {#if naming && naming.id === list.id}
                    <form aria-label="Rename list" onsubmit={saveList}>
                        <input type="text" aria-label="List name" bind:value={naming.name} maxlength="255" />
                    </form>
                {:else}
                    <h2>{list.name}</h2>
                    {#if !list.built_in}
                        {#if deletingList === list.id}
                            <span>Delete “{list.name}” and its tasks? It cannot be undone.</span>
                            <button type="button" class="danger" onclick={() => account && ((deletingList = null), act(() => deleteTaskList(account.id, list.id), "Deleted."))}>Delete it</button>
                            <button type="button" onclick={() => (deletingList = null)}>Keep</button>
                        {:else}
                            <button type="button" onclick={() => (naming = { id: list.id, name: list.name })}>Rename</button>
                            <button type="button" onclick={() => (deletingList = list.id)}>Delete list</button>
                        {/if}
                    {/if}
                {/if}
                <label class="done"><input type="checkbox" bind:checked={withDone} onchange={loadTasks} /> Show done</label>
            </header>
            {#if list.read_only}
                <p class="muted">These are your flagged messages. Flag, unflag or complete the message in Mail to change one.</p>
            {:else}
                <form class="add" aria-label="Add a task" onsubmit={add}>
                    <input type="text" aria-label="Add a task" placeholder="Add a task" bind:value={adding} maxlength="255" />
                </form>
            {/if}
            <ul aria-label="Tasks">
                {#each tasks as task (task.id)}
                    <li class:chosen={chosen?.id === task.id}>
                        <input
                            type="checkbox"
                            aria-label={`${task.title} is done`}
                            checked={task.done}
                            disabled={list.read_only}
                            onchange={(event) => account && act(() => completeTask(account.id, task.id, event.currentTarget.checked), event.currentTarget.checked ? `Done: “${task.title}”.` : "Not done.")}
                        />
                        <button type="button" class="what" onclick={() => (chosen = task)}>
                            <span class:struck={task.done}>{task.title}</span>
                            <span class="muted" class:late={overdue(task)}>
                                {task.due ? dueSaid(task.due) : ""}{task.reminder_at ? " · reminder" : ""}{task.links.length ? " · from an email" : ""}{task.on_server ? "" : " · not in To Do yet"}
                            </span>
                        </button>
                        {#if task.importance === "high"}<Star size={15} aria-label="Important" />{/if}
                    </li>
                {:else}
                    <li class="muted">Nothing to do{withDone ? "" : " that is not done"}.</li>
                {/each}
            </ul>
        {/if}
        {#if undoable}
            <p class="undo" role="status">{undoable.said} <button type="button" onclick={undo}>Undo</button></p>
        {/if}
    </section>

    {#if chosen && list}
        {@const task = chosen}
        <section class="detail" aria-label="The task">
            <input type="text" aria-label="Title" value={task.title} disabled={list.read_only} onchange={(event) => event.currentTarget.value.trim() && save({ title: event.currentTarget.value.trim() })} />
            <label class="stacked">Due <input type="date" value={task.due ?? ""} disabled={list.read_only} onchange={(event) => save({ due: event.currentTarget.value })} /></label>
            <label class="stacked">
                Remind me
                <input type="datetime-local" value={reminderLocal(task.reminder_at)} disabled={list.read_only} onchange={(event) => save({ reminder_at: event.currentTarget.value ? new Date(event.currentTarget.value).toISOString() : "" })} />
            </label>
            <label class="stacked">
                Importance
                <select value={task.importance} disabled={list.read_only} onchange={(event) => save({ importance: event.currentTarget.value })}>
                    <option value="low">Low</option>
                    <option value="normal">Normal</option>
                    <option value="high">High</option>
                </select>
            </label>
            <label class="stacked">Notes <textarea rows="6" value={task.notes} disabled={list.read_only} onchange={(event) => save({ notes: event.currentTarget.value })}></textarea></label>
            {#if linkedMessage(task) && account}
                {@const link = linkedMessage(task)}
                <button type="button" onclick={() => account && onMessage(account.id, (link?.message_id ?? link?.external_id) as string)}>Open the email: {link?.name}</button>
            {/if}
            {#if !list.read_only}
                <button type="button" class="danger" onclick={() => account && act(() => deleteTask(account.id, task.id), `Deleted “${task.title}”.`).then(() => (chosen = null))}>Delete</button>
            {/if}
        </section>
    {/if}
</div>

<style>
    .tasks {
        height: 100%;
        display: grid;
        grid-template-columns: 14rem minmax(0, 1fr) 20rem;
    }

    aside {
        padding: 0.75rem 0.5rem;
        border-right: 1px solid var(--line);
        background: var(--raised);
        display: flex;
        flex-direction: column;
        gap: 0.5rem;
        overflow-y: auto;
    }

    ul {
        margin: 0;
        padding: 0;
        list-style: none;
    }

    aside li button {
        width: 100%;
        text-align: left;
        padding: 0.35rem 0.5rem;
        border: 0;
        background: none;
        font: inherit;
        border-radius: 4px;
    }

    aside li button.current {
        background: var(--accent-fill);
        color: var(--accent-text);
    }

    .new {
        display: flex;
        align-items: center;
        gap: 0.3rem;
    }

    .list {
        padding: 1rem 1.25rem;
        overflow-y: auto;
        display: flex;
        flex-direction: column;
        gap: 0.6rem;
    }

    header {
        display: flex;
        align-items: center;
        gap: 0.5rem;
    }

    h2 {
        margin: 0;
        flex: 1;
        font-size: 1.2rem;
    }

    .done {
        font-size: 0.85rem;
    }

    .add input,
    .detail input[type="text"] {
        width: 100%;
        font: inherit;
        padding: 0.45rem;
    }

    .list li {
        display: flex;
        align-items: center;
        gap: 0.5rem;
        padding: 0.35rem 0;
        border-bottom: 1px solid var(--line);
    }

    .list li.chosen {
        background: color-mix(in srgb, var(--accent) 8%, transparent);
    }

    .what {
        flex: 1;
        min-width: 0;
        display: flex;
        flex-direction: column;
        align-items: flex-start;
        border: 0;
        background: none;
        font: inherit;
        text-align: left;
        padding: 0;
    }

    .struck {
        text-decoration: line-through;
        color: var(--muted);
    }

    .late {
        color: var(--danger);
    }

    .detail {
        border-left: 1px solid var(--line);
        padding: 1rem;
        display: flex;
        flex-direction: column;
        gap: 0.6rem;
        overflow-y: auto;
    }

    .stacked {
        display: flex;
        flex-direction: column;
        gap: 0.2rem;
        font-size: 0.9rem;
    }

    textarea,
    select,
    input[type="date"],
    input[type="datetime-local"] {
        font: inherit;
        padding: 0.35rem;
    }

    .problem {
        margin: 0;
        padding: 0.5rem 0.75rem;
        background: color-mix(in srgb, var(--danger) 14%, var(--surface));
        font-size: 0.9rem;
    }

    .problem p {
        margin: 0 0 0.4rem;
    }

    .undo {
        margin: 0;
        font-size: 0.9rem;
    }

    p {
        margin: 0;
    }
</style>
