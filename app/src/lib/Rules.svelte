<script lang="ts">
    import { execute, say } from "./commands";
    import { type Category, type Folder, listCategories, listFolders } from "./mail";

    interface Props {
        accountId: string;
        username: string;
    }

    interface Person {
        address: string;
        name?: string;
    }

    type Part = Record<string, unknown>;

    interface Rule {
        id: string;
        name: string;
        enabled: boolean;
        sequence: number;
        read_only: boolean;
        has_error?: boolean;
        conditions: Part;
        exceptions: Part;
        actions: Part;
        summary: string;
    }

    /** What the form below can hold. A rule with anything else is shown and not edited, so that saving it loses nothing. */
    const CONDITIONS = ["fromAddresses", "senderContains", "subjectContains", "bodyOrSubjectContains", "hasAttachments", "sentOnlyToMe", "importance"];
    const ACTIONS = ["moveToFolder", "markAsRead", "assignCategories", "markImportance", "forwardTo", "delete", "stopProcessingRules"];

    let { accountId, username }: Props = $props();
    let rules = $state<Rule[] | null>(null);
    let folders = $state<Folder[]>([]);
    let categories = $state<Category[]>([]);
    let problem = $state<string | null>(null);
    let busy = $state(false);
    /** The rule about to be deleted, until it is confirmed. */
    let deleting = $state<string | null>(null);

    const blank = () => ({
        id: null as string | null,
        name: "",
        from: "",
        sender: "",
        subject: "",
        anywhere: "",
        attachments: false,
        onlyMe: false,
        importance: "",
        folder: "",
        read: false,
        category: "",
        mark: "",
        forward: "",
        remove: false,
        stop: false,
    });
    /** The rule being written, while its form is open. */
    let form = $state<ReturnType<typeof blank> | null>(null);

    const fits = (rule: Rule) =>
        !rule.read_only &&
        Object.keys(rule.exceptions).length === 0 &&
        Object.keys(rule.conditions).every((name) => CONDITIONS.includes(name)) &&
        Object.keys(rule.actions).every((name) => ACTIONS.includes(name));

    const words = (list: unknown) => (Array.isArray(list) ? list.join(", ") : "");
    const people = (list: unknown) => (Array.isArray(list) ? (list as Person[]).map((each) => each.address).join(", ") : "");
    const split = (typed: string) =>
        typed
            .split(/[,;\n]/)
            .map((each) => each.trim())
            .filter(Boolean);

    async function read() {
        try {
            rules = (await execute<{ rules: Rule[] }>("mail.rules.list", { account_id: accountId })).rules;
        } catch (failure) {
            problem = say(failure);
        }
    }

    $effect(() => {
        read();
        listFolders(accountId)
            .then((found) => (folders = found))
            .catch(() => {});
        listCategories(accountId)
            .then((found) => (categories = found))
            .catch(() => {});
    });

    function change(rule: Rule) {
        const c = rule.conditions;
        const a = rule.actions;
        form = {
            id: rule.id,
            name: rule.name,
            from: people(c.fromAddresses),
            sender: words(c.senderContains),
            subject: words(c.subjectContains),
            anywhere: words(c.bodyOrSubjectContains),
            attachments: c.hasAttachments === true,
            onlyMe: c.sentOnlyToMe === true,
            importance: typeof c.importance === "string" ? c.importance : "",
            folder: typeof a.moveToFolder === "string" ? a.moveToFolder : "",
            read: a.markAsRead === true,
            category: Array.isArray(a.assignCategories) ? String(a.assignCategories[0] ?? "") : "",
            mark: typeof a.markImportance === "string" ? a.markImportance : "",
            forward: people(a.forwardTo),
            remove: a.delete === true,
            stop: a.stopProcessingRules === true,
        };
        problem = null;
    }

    async function save(submitted: SubmitEvent) {
        submitted.preventDefault();
        if (!form) return;
        const conditions: Part = {};
        if (split(form.from).length) conditions.fromAddresses = split(form.from);
        if (split(form.sender).length) conditions.senderContains = split(form.sender);
        if (split(form.subject).length) conditions.subjectContains = split(form.subject);
        if (split(form.anywhere).length) conditions.bodyOrSubjectContains = split(form.anywhere);
        if (form.attachments) conditions.hasAttachments = true;
        if (form.onlyMe) conditions.sentOnlyToMe = true;
        if (form.importance) conditions.importance = form.importance;
        const actions: Part = {};
        if (form.folder) actions.moveToFolder = form.folder;
        if (form.read) actions.markAsRead = true;
        if (form.category) actions.assignCategories = [form.category];
        if (form.mark) actions.markImportance = form.mark;
        if (split(form.forward).length) actions.forwardTo = split(form.forward);
        if (form.remove) actions.delete = true;
        if (form.stop) actions.stopProcessingRules = true;
        if (Object.keys(actions).length === 0) {
            problem = "Choose at least one thing for the rule to do.";
            return;
        }

        busy = true;
        problem = null;
        try {
            if (form.id) {
                await execute("mail.rules.update", { account_id: accountId, rule_id: form.id, name: form.name.trim(), conditions, actions });
            } else {
                await execute("mail.rules.create", { account_id: accountId, name: form.name.trim(), conditions, actions });
            }
            form = null;
            await read();
        } catch (failure) {
            problem = say(failure);
        } finally {
            busy = false;
        }
    }

    async function turn(rule: Rule, enabled: boolean) {
        problem = null;
        try {
            await execute("mail.rules.update", { account_id: accountId, rule_id: rule.id, enabled });
        } catch (failure) {
            problem = say(failure);
        }
        await read();
    }

    async function remove(rule: Rule) {
        problem = null;
        deleting = null;
        try {
            await execute("mail.rules.delete", { account_id: accountId, rule_id: rule.id });
        } catch (failure) {
            problem = say(failure);
        }
        await read();
    }
</script>

<section class="rules" aria-label="Rules for {username}">
    <p class="muted">Rules are kept by Microsoft 365 and run there on mail as it arrives in the inbox, in this order, whether or not Brolga is running.</p>
    {#if problem}
        <p class="problem" role="alert">{problem}</p>
    {/if}

    {#if rules}
        {#if rules.length === 0}
            <p class="muted">There are no rules.</p>
        {/if}
        <ul>
            {#each rules as rule (rule.id)}
                <li>
                    <input type="checkbox" aria-label={`${rule.name} is on`} checked={rule.enabled} disabled={rule.read_only} onchange={(event) => turn(rule, event.currentTarget.checked)} />
                    <span class="what">
                        <span class="name">{rule.name}{#if rule.has_error}<span class="warning"> · Microsoft 365 says this rule is broken</span>{/if}</span>
                        <span class="muted">{rule.summary}</span>
                        {#if rule.read_only}
                            <span class="muted">Read-only: it can be changed only where it was made.</span>
                        {:else if !fits(rule)}
                            <span class="muted">It uses parts this screen cannot edit, so it can be turned on or off or deleted here, and changed elsewhere.</span>
                        {/if}
                    </span>
                    {#if deleting === rule.id}
                        <button type="button" class="danger" onclick={() => remove(rule)}>Delete it?</button>
                        <button type="button" onclick={() => (deleting = null)}>Keep</button>
                    {:else if !rule.read_only}
                        {#if fits(rule)}
                            <button type="button" aria-label={`Change ${rule.name}`} onclick={() => change(rule)}>Change</button>
                        {/if}
                        <button type="button" aria-label={`Delete ${rule.name}`} onclick={() => (deleting = rule.id)}>Delete</button>
                    {/if}
                </li>
            {/each}
        </ul>

        {#if form}
            <form aria-label={form.id ? "Change rule" : "New rule"} onsubmit={save}>
                <label class="stacked">Name <input type="text" bind:value={form.name} required maxlength="255" /></label>

                <fieldset>
                    <legend>When a message arrives and</legend>
                    <label class="stacked">It is from <input type="text" bind:value={form.from} placeholder="someone@example.com, another@example.com" /></label>
                    <label class="stacked">The sender contains <input type="text" bind:value={form.sender} /></label>
                    <label class="stacked">The subject contains <input type="text" bind:value={form.subject} /></label>
                    <label class="stacked">The subject or body contains <input type="text" bind:value={form.anywhere} /></label>
                    <label><input type="checkbox" bind:checked={form.attachments} /> It has attachments</label>
                    <label><input type="checkbox" bind:checked={form.onlyMe} /> It is sent only to me</label>
                    <label class="stacked">
                        Its importance is
                        <select bind:value={form.importance}>
                            <option value="">Any</option>
                            <option value="high">High</option>
                            <option value="normal">Normal</option>
                            <option value="low">Low</option>
                        </select>
                    </label>
                    <p class="muted">Several words or addresses in one box, separated by commas, are alternatives. Everything filled in must be true; with nothing filled in, the rule applies to every message.</p>
                </fieldset>

                <fieldset>
                    <legend>Do this</legend>
                    <label class="stacked">
                        Move it to
                        <select bind:value={form.folder}>
                            <option value="">Leave it where it is</option>
                            {#each folders as folder (folder.id)}
                                <option value={folder.id}>{"  ".repeat(folder.depth ?? 0)}{folder.name}</option>
                            {/each}
                        </select>
                    </label>
                    <label class="stacked">
                        Categorise it
                        <select bind:value={form.category}>
                            <option value="">No category</option>
                            {#each categories as each (each.name)}
                                <option value={each.name}>{each.name}</option>
                            {/each}
                            {#if form.category && !categories.some((each) => each.name === form?.category)}
                                <option value={form.category}>{form.category}</option>
                            {/if}
                        </select>
                    </label>
                    <label class="stacked">
                        Mark its importance
                        <select bind:value={form.mark}>
                            <option value="">Leave it</option>
                            <option value="high">High</option>
                            <option value="normal">Normal</option>
                            <option value="low">Low</option>
                        </select>
                    </label>
                    <label class="stacked">Forward it to <input type="text" bind:value={form.forward} placeholder="someone@example.com" /></label>
                    <label><input type="checkbox" bind:checked={form.read} /> Mark it read</label>
                    <label><input type="checkbox" bind:checked={form.remove} /> Delete it</label>
                    <label><input type="checkbox" bind:checked={form.stop} /> Stop there, and try no later rule</label>
                </fieldset>

                <div class="buttons">
                    <button type="submit" class="primary" disabled={busy || !form.name.trim()}>{busy ? "Saving…" : "Save rule"}</button>
                    <button type="button" onclick={() => (form = null)}>Cancel</button>
                </div>
            </form>
        {:else}
            <div class="buttons"><button type="button" onclick={() => ((form = blank()), (problem = null))}>New rule</button></div>
        {/if}
    {:else if !problem}
        <p class="muted" role="status">Reading from Microsoft 365…</p>
    {/if}
</section>

<style>
    .rules {
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

    ul {
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

    .warning {
        font-weight: 400;
        color: var(--danger);
    }

    form,
    fieldset {
        display: flex;
        flex-direction: column;
        gap: 0.4rem;
    }

    fieldset {
        margin: 0;
        padding: 0;
        border: 0;
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
    select {
        font: inherit;
        padding: 0.35rem;
    }

    .buttons {
        display: flex;
        gap: 0.5rem;
    }

    .problem {
        padding: 0.5rem 0.75rem;
        background: color-mix(in srgb, var(--danger) 14%, var(--surface));
        font-size: 0.85rem;
    }
</style>
