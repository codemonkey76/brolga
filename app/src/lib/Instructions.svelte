<script lang="ts">
    import { execute, say } from "./commands";
    import InstructionRun from "./InstructionRun.svelte";

    interface Props {
        accountId: string;
        username: string;
    }

    type Action = "move" | "set_categories" | "flag" | "set_read" | "create_task";

    interface Instruction {
        id: string;
        name: string;
        text: string;
        enabled: boolean;
        actions: Action[];
    }

    interface Device {
        id: string;
        name: string;
    }

    interface Listed {
        enabled: boolean;
        version: number;
        set_up: boolean;
        instructions: Instruction[];
        processor: { device_id: string; device_name: string; since: string } | null;
        processing_here: boolean;
        last_active: string | null;
        this_device: Device;
        offline: boolean;
    }

    /** What an instruction may be allowed to do, in the order they are offered. */
    const ACTIONS: { id: Action; name: string }[] = [
        { id: "move", name: "Move it to a folder the instruction names" },
        { id: "set_categories", name: "Give it categories" },
        { id: "flag", name: "Flag it, with a due day if the instruction gives one" },
        { id: "set_read", name: "Mark it read or unread" },
        { id: "create_task", name: "Make a To Do task from it, in a list the instruction names or in Tasks" },
    ];
    const LONGEST = 2000;

    let { accountId, username }: Props = $props();
    let listed = $state<Listed | null>(null);
    let problem = $state<string | null>(null);
    let busy = $state(false);
    /** The instruction about to be deleted, until it is confirmed. */
    let deleting = $state<string | null>(null);
    /** Taking or giving up the role, until it is confirmed. */
    let confirming = $state<"take" | "release" | null>(null);
    /** This device's new name, while it is being renamed. */
    let naming = $state<string | null>(null);

    const blank = () => ({ id: null as string | null, name: "", text: "", enabled: true, actions: [] as Action[] });
    /** The instruction being written, while its form is open. */
    let form = $state<ReturnType<typeof blank> | null>(null);

    async function read() {
        try {
            listed = await execute<Listed>("mail.instructions.list", { account_id: accountId });
        } catch (failure) {
            problem = say(failure);
        }
    }

    $effect(() => {
        read();
    });

    /** Run a change to the list, made to the version shown. Whatever happens, the list is read again, so a change made elsewhere shows. */
    async function changing(command: string, args: Record<string, unknown>): Promise<boolean> {
        busy = true;
        problem = null;
        try {
            await execute(command, { account_id: accountId, version: listed?.version, ...args });
            return true;
        } catch (failure) {
            problem = say(failure);
            return false;
        } finally {
            busy = false;
            await read();
        }
    }

    async function save(submitted: SubmitEvent) {
        submitted.preventDefault();
        if (!form) return;
        if (form.actions.length === 0) {
            problem = "Choose at least one thing the instruction may do.";
            return;
        }
        const args: Record<string, unknown> = { name: form.name.trim(), text: form.text.trim(), enabled: form.enabled, actions: form.actions };
        if (form.id) args.instruction_id = form.id;
        // Refused because the list changed elsewhere, the form stays open with what was written.
        if (await changing("mail.instructions.set", args)) form = null;
    }

    function allow(action: Action, on: boolean) {
        if (!form) return;
        form.actions = on ? [...form.actions.filter((each) => each !== action), action] : form.actions.filter((each) => each !== action);
    }

    const turn = (each: Instruction, on: boolean) => changing("mail.instructions.set", { instruction_id: each.id, name: each.name, text: each.text, enabled: on, actions: each.actions });

    const move = (each: Instruction, position: number) => changing("mail.instructions.reorder", { instruction_id: each.id, position });

    async function remove(each: Instruction) {
        deleting = null;
        await changing("mail.instructions.delete", { instruction_id: each.id });
    }

    async function role(command: "mail.instructions.processor.take" | "mail.instructions.processor.release") {
        confirming = null;
        busy = true;
        problem = null;
        try {
            listed = await execute<Listed>(command, { account_id: accountId });
        } catch (failure) {
            problem = say(failure);
            await read();
        } finally {
            busy = false;
        }
    }

    async function rename(submitted: SubmitEvent) {
        submitted.preventDefault();
        if (naming === null) return;
        problem = null;
        try {
            await execute("app.device.rename", { name: naming.trim() });
            naming = null;
        } catch (failure) {
            problem = say(failure);
        }
        await read();
    }

    interface Model {
        provider: "anthropic" | "ollama" | null;
        model: string | null;
        cloud_consent: boolean;
        key_saved: boolean;
        ollama_address: string;
        default_anthropic_model: string;
    }

    interface Status {
        ready: boolean;
        not_ready_because: string | null;
        waiting: number;
        failed: number;
        today: { requests: number };
        daily_cap: number;
        last_error: { message_id: string; error: string; at: string } | null;
    }

    interface Entry {
        at: string;
        command: string;
        outcome: "ok" | "refused" | "failed";
        detail: string;
        subject: string | null;
        instruction: string | null;
        undo_id: string | null;
        /** mail.undo, or tasks.undo for a task made. */
        undo_command?: string;
    }

    let model = $state<Model | null>(null);
    let status = $state<Status | null>(null);
    let entries = $state<Entry[]>([]);
    /** The model being chosen, while its form is open. */
    let choosing = $state<{ provider: "anthropic" | "ollama"; model: string; consent: boolean; address: string; key: string } | null>(null);
    let ollamaModels = $state<string[] | null>(null);
    let ollamaProblem = $state<string | null>(null);
    let tried = $state<string | null>(null);
    /** Undo ids already taken back. */
    let undone = $state<string[]>([]);

    async function readModel() {
        try {
            model = await execute<Model>("mail.instructions.model.get", { account_id: accountId });
            status = await execute<Status>("mail.instructions.status", { account_id: accountId });
            entries = (await execute<{ entries: Entry[] }>("mail.instructions.log", { account_id: accountId, limit: 20 })).entries;
        } catch {
            // The list above says what is wrong.
        }
    }

    $effect(() => {
        readModel();
    });

    function choose() {
        if (!model) return;
        choosing = {
            provider: model.provider ?? "anthropic",
            model: model.model ?? (model.provider === "ollama" ? "" : model.default_anthropic_model),
            consent: model.cloud_consent,
            address: model.ollama_address,
            key: "",
        };
        problem = null;
        tried = null;
        if (choosing.provider === "ollama") findModels();
    }

    async function findModels() {
        if (!choosing) return;
        ollamaModels = null;
        ollamaProblem = null;
        try {
            const found = await execute<{ reachable: boolean; models: string[]; error: string | null }>("mail.instructions.model.ollama_models", { address: choosing.address });
            ollamaModels = found.models;
            if (!found.reachable) ollamaProblem = found.error;
            else if (choosing && !found.models.includes(choosing.model)) choosing.model = found.models[0] ?? "";
        } catch (failure) {
            ollamaProblem = say(failure);
        }
    }

    async function saveModel(submitted: SubmitEvent) {
        submitted.preventDefault();
        if (!choosing) return;
        busy = true;
        problem = null;
        try {
            if (choosing.provider === "anthropic" && choosing.key.trim()) {
                await execute("mail.instructions.model.set_key", { key: choosing.key.trim() });
            }
            const args: Record<string, unknown> = { account_id: accountId, provider: choosing.provider, model: choosing.model.trim() || undefined };
            if (choosing.provider === "anthropic") args.cloud_consent = choosing.consent;
            else args.ollama_address = choosing.address.trim();
            await execute("mail.instructions.model.set", args);
            choosing = null;
        } catch (failure) {
            problem = say(failure);
        } finally {
            busy = false;
            await readModel();
        }
    }

    async function forgetKey() {
        await execute("mail.instructions.model.set_key", { key: "" }).catch((failure) => (problem = say(failure)));
        await readModel();
    }

    async function test() {
        tried = "Asking the model…";
        try {
            const result = await execute<{ ok: boolean; model: string; seconds: number; error: string | null }>("mail.instructions.model.test", { account_id: accountId });
            tried = result.ok ? `${result.model} answered properly in ${result.seconds.toFixed(1)} seconds.` : `It did not work: ${result.error}`;
        } catch (failure) {
            tried = say(failure);
        }
    }

    async function processNow() {
        busy = true;
        problem = null;
        try {
            const pass = await execute<{ processed: number; already_done: number; failed: number; stopped: string | null }>("mail.instructions.process", { account_id: accountId });
            tried = pass.stopped ?? `${pass.processed} message${pass.processed === 1 ? "" : "s"} dealt with${pass.failed ? `, ${pass.failed} could not be` : ""}.`;
        } catch (failure) {
            problem = say(failure);
        } finally {
            busy = false;
            await readModel();
        }
    }

    async function undo(entry: Entry) {
        if (!entry.undo_id) return;
        try {
            // A task is in the To Do of the account a shared mailbox belongs to.
            const tasks = entry.undo_command === "tasks.undo";
            await execute(entry.undo_command ?? "mail.undo", { account_id: tasks ? accountId.split("~")[0] : accountId, undo_id: entry.undo_id });
            undone = [...undone, entry.undo_id];
        } catch (failure) {
            problem = say(failure);
        }
    }

    /** What an entry in the log did, in words. */
    const did = (entry: Entry) =>
        ({
            "mail.messages.move": "Moved",
            "mail.messages.set_categories": "Categorised",
            "mail.messages.set_flag": "Flagged",
            "mail.messages.set_read": "Marked",
            "tasks.create": "Made a task",
        })[entry.command] ?? entry.command.replace(/^mail\.instructions\./, "Wanted to ");

    const when = (iso: string | null) => (iso ? new Date(iso).toLocaleString(undefined, { day: "numeric", month: "short", hour: "numeric", minute: "2-digit" }) : "never");
    const allowed = (each: Instruction) =>
        ACTIONS.filter((action) => each.actions.includes(action.id))
            .map((action) => action.name.toLowerCase())
            .join("; ");
</script>

<section class="instructions" aria-label="Mail instructions for {username}">
    <p class="muted">
        Instructions are written in plain words, such as “Invoices from Telstra go in Accounts/Telstra”. An AI model reads each message arriving in the inbox and does what the instructions that fit it allow. They are kept in the mailbox, so every computer signed in to it shows the same list, and they are carried out by one computer you choose.
    </p>
    {#if problem}
        <p class="problem" role="alert">{problem}</p>
    {/if}

    {#if listed}
        {#if listed.offline}
            <p class="muted" role="status">Microsoft 365 cannot be reached, so these are the instructions as last read. They can be changed once it can be reached.</p>
        {/if}

        <div class="device" aria-label="Processing device">
            {#if listed.processing_here}
                <p><strong>This computer</strong> ({listed.this_device.name}) carries out these instructions.</p>
            {:else if listed.processor}
                <p><strong>{listed.processor.device_name}</strong> carries out these instructions. It was last active {when(listed.last_active)}.</p>
            {:else}
                <p>No computer carries out these instructions yet, so nothing is done with them.</p>
            {/if}

            {#if confirming === "take"}
                <p>
                    {listed.processor ? `Move the job from ${listed.processor.device_name} to this computer (${listed.this_device.name})? ${listed.processor.device_name} will stop carrying out the instructions.` : `Carry out this mailbox's instructions on this computer (${listed.this_device.name})?`}
                </p>
                <div class="buttons">
                    <button type="button" class="primary" disabled={busy || listed.offline} onclick={() => role("mail.instructions.processor.take")}>Yes, on this computer</button>
                    <button type="button" onclick={() => (confirming = null)}>Cancel</button>
                </div>
            {:else if confirming === "release"}
                <p>Stop carrying out this mailbox's instructions anywhere? Nothing is done with them until a computer is chosen again.</p>
                <div class="buttons">
                    <button type="button" class="danger" disabled={busy || listed.offline} onclick={() => role("mail.instructions.processor.release")}>Stop</button>
                    <button type="button" onclick={() => (confirming = null)}>Cancel</button>
                </div>
            {:else}
                <div class="buttons">
                    {#if !listed.processing_here}
                        <button type="button" disabled={busy || listed.offline} onclick={() => (confirming = "take")}>{listed.processor ? "Carry them out on this computer instead" : "Carry them out on this computer"}</button>
                    {/if}
                    {#if listed.processor}
                        <button type="button" disabled={busy || listed.offline} onclick={() => (confirming = "release")}>Stop carrying them out</button>
                    {/if}
                </div>
            {/if}

            {#if naming === null}
                <p class="muted">
                    This computer is called “{listed.this_device.name}” on your other computers.
                    <button type="button" class="link" onclick={() => (naming = listed?.this_device.name ?? "")}>Rename</button>
                </p>
            {:else}
                <form class="rename" aria-label="Rename this computer" onsubmit={rename}>
                    <input type="text" aria-label="This computer's name" bind:value={naming} required maxlength="100" />
                    <button type="submit" class="primary" disabled={!naming.trim()}>Save</button>
                    <button type="button" onclick={() => (naming = null)}>Cancel</button>
                </form>
            {/if}
        </div>

        {#if model}
            <div class="device" aria-label="Model">
                {#if model.provider === "anthropic"}
                    <p><strong>{model.model}</strong> at Anthropic reads the mail.</p>
                    {#if !model.cloud_consent}
                        <p class="warning">Sending this mailbox's mail to Anthropic has not been agreed to, so nothing is sent.</p>
                    {/if}
                {:else if model.provider === "ollama"}
                    <p><strong>{model.model}</strong> on Ollama reads the mail.</p>
                {:else}
                    <p>No model has been chosen yet.</p>
                {/if}
                {#if listed.processing_here && model.provider === "anthropic"}
                    <p class="muted">{model.key_saved ? "An Anthropic API key is saved on this computer." : "No Anthropic API key is saved on this computer."}</p>
                {:else if listed.processing_here && model.provider === "ollama"}
                    <p class="muted">This computer reaches Ollama at {model.ollama_address}.</p>
                {/if}

                {#if choosing}
                    <form aria-label="Choose the model" onsubmit={saveModel}>
                        <fieldset>
                            <legend>Read the mail with</legend>
                            <label><input type="radio" name="provider" value="anthropic" bind:group={choosing.provider} /> A model at Anthropic, with an API key</label>
                            <label><input type="radio" name="provider" value="ollama" bind:group={choosing.provider} onchange={findModels} /> A model on an Ollama server</label>
                        </fieldset>
                        {#if choosing.provider === "anthropic"}
                            <label class="stacked">Model <input type="text" bind:value={choosing.model} placeholder={model.default_anthropic_model} /></label>
                            {#if listed.processing_here}
                                <label class="stacked">
                                    API key {model.key_saved ? "(one is saved; type a new one to replace it)" : ""}
                                    <input type="password" autocomplete="off" bind:value={choosing.key} placeholder="sk-ant-…" />
                                </label>
                            {/if}
                            <label class="consent">
                                <input type="checkbox" bind:checked={choosing.consent} />
                                <span>
                                    Send this mailbox's mail to Anthropic to be read
                                    <small class="muted">For each message that arrives: who it is from and to, its subject, its text (up to 8,000 characters) and the names of its attachments. Never what is in the attachments.</small>
                                </span>
                            </label>
                        {:else}
                            <label class="stacked">Ollama's address <input type="text" bind:value={choosing.address} onchange={findModels} /></label>
                            {#if ollamaProblem}
                                <p class="warning">{ollamaProblem}</p>
                            {/if}
                            <label class="stacked">
                                Model
                                <select aria-label="Model" bind:value={choosing.model} disabled={!ollamaModels?.length}>
                                    {#each ollamaModels ?? [] as each (each)}
                                        <option value={each}>{each}</option>
                                    {/each}
                                </select>
                            </label>
                        {/if}
                        <div class="buttons">
                            <button type="submit" class="primary" disabled={busy || listed.offline || (choosing.provider === "ollama" && !choosing.model)}>Save</button>
                            <button type="button" onclick={() => (choosing = null)}>Cancel</button>
                        </div>
                    </form>
                {:else}
                    <div class="buttons">
                        <button type="button" disabled={listed.offline} onclick={choose}>{model.provider ? "Change the model" : "Choose a model"}</button>
                        {#if listed.processing_here && model.provider}
                            <button type="button" onclick={test}>Try it</button>
                        {/if}
                        {#if listed.processing_here && model.key_saved}
                            <button type="button" onclick={forgetKey}>Forget the API key</button>
                        {/if}
                    </div>
                {/if}
                {#if tried}
                    <p class="muted" role="status">{tried}</p>
                {/if}
            </div>
        {/if}

        {#if listed.processing_here && status}
            <div class="device" aria-label="Processing">
                {#if status.ready}
                    <p>New mail in the Inbox is dealt with after each sync. {status.today.requests} sent to the model today{model?.provider === "anthropic" ? `, of at most ${status.daily_cap}` : ""}.</p>
                {:else}
                    <p class="warning">Nothing is being done: {status.not_ready_because}</p>
                {/if}
                {#if status.waiting || status.failed}
                    <p class="muted">{status.waiting} waiting to be tried again{status.failed ? `, ${status.failed} given up on` : ""}.</p>
                {/if}
                {#if status.last_error}
                    <p class="muted">Last trouble, {when(status.last_error.at)}: {status.last_error.error}</p>
                {/if}
                <div class="buttons">
                    <button type="button" disabled={busy || !status.ready} onclick={processNow}>Deal with new mail now</button>
                </div>
            </div>
        {/if}

        {#if listed.processing_here && status?.ready}
            <InstructionRun {accountId} />
        {/if}

        {#if entries.length > 0}
            <details class="log">
                <summary>What the instructions have done</summary>
                <ul aria-label="What the instructions have done">
                    {#each entries as entry, n (n)}
                        <li>
                            <span class="what">
                                <span>{entry.outcome === "ok" ? did(entry) : entry.outcome === "refused" ? `Not done (${did(entry)})` : "Did not work"}: {entry.subject ?? ""}</span>
                                <span class="muted">{when(entry.at)}{entry.instruction ? ` · ${entry.instruction}` : ""}{entry.detail ? ` · ${entry.detail}` : ""}</span>
                            </span>
                            {#if entry.outcome === "ok" && entry.undo_id}
                                <button type="button" disabled={undone.includes(entry.undo_id)} onclick={() => undo(entry)}>{undone.includes(entry.undo_id) ? "Undone" : "Undo"}</button>
                            {/if}
                        </li>
                    {/each}
                </ul>
            </details>
        {/if}

        {#if listed.instructions.length > 0}
            <label class="whole">
                <input type="checkbox" checked={listed.enabled} disabled={busy || listed.offline} onchange={(event) => changing("mail.instructions.set_enabled", { enabled: event.currentTarget.checked })} />
                Instructions are on for this mailbox
            </label>
        {:else}
            <p class="muted">There are no instructions.</p>
        {/if}

        <ol>
            {#each listed.instructions as each, at (each.id)}
                <li>
                    <input type="checkbox" aria-label={`${each.name} is on`} checked={each.enabled} disabled={busy || listed.offline} onchange={(event) => turn(each, event.currentTarget.checked)} />
                    <span class="what">
                        <span class="name">{each.name}</span>
                        <span class="text">{each.text}</span>
                        <span class="muted">May: {allowed(each)}.</span>
                    </span>
                    {#if deleting === each.id}
                        <button type="button" class="danger" disabled={busy} onclick={() => remove(each)}>Delete it?</button>
                        <button type="button" onclick={() => (deleting = null)}>Keep</button>
                    {:else}
                        <button type="button" aria-label={`Move ${each.name} up`} disabled={busy || listed.offline || at === 0} onclick={() => move(each, at)}>↑</button>
                        <button type="button" aria-label={`Move ${each.name} down`} disabled={busy || listed.offline || at === listed.instructions.length - 1} onclick={() => move(each, at + 2)}>↓</button>
                        <button type="button" aria-label={`Change ${each.name}`} disabled={listed.offline} onclick={() => ((form = { ...each, actions: [...each.actions] }), (problem = null))}>Change</button>
                        <button type="button" aria-label={`Delete ${each.name}`} disabled={listed.offline} onclick={() => (deleting = each.id)}>Delete</button>
                    {/if}
                </li>
            {/each}
        </ol>

        {#if form}
            <form aria-label={form.id ? "Change instruction" : "New instruction"} onsubmit={save}>
                <label class="stacked">Name <input type="text" bind:value={form.name} required maxlength="100" /></label>
                <label class="stacked">
                    The instruction, in plain words
                    <textarea bind:value={form.text} required maxlength={LONGEST} rows="4" placeholder="Anything from a customer asking for a quote is flagged due in two days."></textarea>
                    <small class="muted">{form.text.length} of {LONGEST} characters</small>
                </label>
                <fieldset>
                    <legend>It may</legend>
                    {#each ACTIONS as action (action.id)}
                        <label><input type="checkbox" checked={form.actions.includes(action.id)} onchange={(event) => allow(action.id, event.currentTarget.checked)} /> {action.name}</label>
                    {/each}
                    <p class="muted">The model can do nothing else, whatever a message says.</p>
                </fieldset>
                <label><input type="checkbox" bind:checked={form.enabled} /> On</label>
                <div class="buttons">
                    <button type="submit" class="primary" disabled={busy || !form.name.trim() || !form.text.trim()}>{busy ? "Saving…" : "Save instruction"}</button>
                    <button type="button" onclick={() => (form = null)}>Cancel</button>
                </div>
            </form>
        {:else}
            <div class="buttons"><button type="button" disabled={listed.offline || listed.instructions.length >= 50} onclick={() => ((form = blank()), (problem = null))}>New instruction</button></div>
        {/if}
    {:else if !problem}
        <p class="muted" role="status">Reading from Microsoft 365…</p>
    {/if}
</section>

<style>
    .instructions {
        margin: 0.75rem 0 0 3rem;
        display: flex;
        flex-direction: column;
        gap: 0.6rem;
        max-width: 40rem;
    }

    p {
        margin: 0;
        font-size: 0.8rem;
    }

    .device {
        display: flex;
        flex-direction: column;
        gap: 0.4rem;
        padding: 0.6rem 0.75rem;
        border: 1px solid var(--line);
        border-radius: 6px;
    }

    .device p:not(.muted) {
        font-size: 0.9rem;
    }

    .whole {
        display: flex;
        align-items: center;
        gap: 0.5rem;
        font-size: 0.9rem;
    }

    ol {
        margin: 0;
        padding: 0;
        list-style: none;
    }

    li {
        display: flex;
        align-items: flex-start;
        gap: 0.5rem;
        padding: 0.4rem 0;
        border-bottom: 1px solid var(--line);
    }

    .what {
        flex: 1;
        min-width: 0;
        display: flex;
        flex-direction: column;
        font-size: 0.85rem;
    }

    .name {
        font-weight: 600;
    }

    .text {
        white-space: pre-wrap;
        overflow-wrap: anywhere;
    }

    form,
    fieldset {
        display: flex;
        flex-direction: column;
        gap: 0.4rem;
    }

    form.rename {
        flex-direction: row;
        align-items: center;
    }

    fieldset {
        margin: 0;
        padding: 0;
        border: 0;
        font-size: 0.9rem;
    }

    legend {
        padding: 0;
        font-weight: 600;
        margin-bottom: 0.25rem;
    }

    .stacked {
        display: flex;
        flex-direction: column;
        gap: 0.2rem;
        font-size: 0.9rem;
    }

    input[type="text"],
    textarea {
        font: inherit;
        padding: 0.35rem;
    }

    .buttons {
        display: flex;
        flex-wrap: wrap;
        gap: 0.5rem;
    }

    .warning {
        color: var(--danger);
    }

    .consent {
        display: flex;
        align-items: flex-start;
        gap: 0.5rem;
        font-size: 0.9rem;
    }

    .consent span {
        display: flex;
        flex-direction: column;
    }

    .log summary {
        cursor: pointer;
        font-size: 0.9rem;
    }

    .log ul {
        margin: 0.4rem 0 0;
        padding: 0;
        list-style: none;
    }

    select {
        font: inherit;
        padding: 0.35rem;
    }

    .link {
        padding: 0;
        border: 0;
        background: none;
        color: var(--accent);
        font: inherit;
        cursor: pointer;
        text-decoration: underline;
    }

    .problem {
        padding: 0.5rem 0.75rem;
        background: color-mix(in srgb, var(--danger) 14%, var(--surface));
        font-size: 0.85rem;
    }
</style>
