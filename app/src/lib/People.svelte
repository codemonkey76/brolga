<script lang="ts">
    import { Pencil, Plus, Trash, UserPlus } from "@lucide/svelte";
    import type { Account } from "./accounts";
    import { chooseFiles, chooseSavePath, execute, say } from "./commands";
    import { type Contact, type ContactFields, type ContactFolder, type Found, createContact, deleteContact, initials, listContactFolders, listContacts, searchPeople, updateContact } from "./people";

    interface Props {
        accounts: Account[];
    }

    let { accounts }: Props = $props();

    let accountId = $state<string | null>(null);
    let folders = $state<ContactFolder[]>([]);
    /** The folder shown. Null for all of them. */
    let folderId = $state<string | null>(null);
    let query = $state("");
    let contacts = $state<Contact[]>([]);
    /** People of the organisation matching what is searched for, who are not contacts. */
    let others = $state<Found[]>([]);
    /** False where the organisation could not be asked. */
    let asked = $state(true);
    let selected = $state<Contact | null>(null);
    /** A contact is being made (`new`) or the selected one changed (`change`). */
    let editing = $state<"new" | "change" | null>(null);
    let sure = $state(false);
    let problem = $state<string | null>(null);
    let loading = 0;

    /** What the last reading or writing of a file came to. */
    let filed = $state<string | null>(null);

    const count = (n: number, one: string) => `${n} ${one}${n === 1 ? "" : "s"}`;

    /** Add the contacts in vCard files a person chooses, to the folder shown. */
    async function importFile() {
        if (!accountId) return;
        const paths = await chooseFiles();
        problem = null;
        filed = null;
        const said: string[] = [];
        for (const path of paths) {
            try {
                const read = await execute<{ imported: number; already_here: number; skipped: number; cut_short: boolean; stopped: string | null }>("contacts.import", {
                    account_id: accountId,
                    path,
                    ...(folderId ? { folder_id: folderId } : {}),
                });
                said.push(
                    [
                        `${count(read.imported, "contact")} added`,
                        read.already_here > 0 ? `${read.already_here} already here` : "",
                        read.skipped > 0 ? `${count(read.skipped, "card")} could not be used` : "",
                        read.cut_short ? "the file held more than 2000 and the rest were left" : "",
                    ]
                        .filter(Boolean)
                        .join(", ") + ".",
                );
                if (read.stopped) problem = read.stopped;
            } catch (failure) {
                problem = say(failure);
            }
        }
        if (said.length > 0) filed = said.join(" ");
        await load();
    }

    /** Write the contacts of the folder shown, or all of them, to a vCard file. */
    async function exportFile() {
        if (!accountId) return;
        const path = await chooseSavePath("contacts.vcf");
        if (!path) return;
        problem = null;
        filed = null;
        try {
            const written = await execute<{ path: string; count: number }>("contacts.export", { account_id: accountId, path, ...(folderId !== null ? { folder_id: folderId } : {}) });
            filed = `${count(written.count, "contact")} written to ${written.path}.`;
        } catch (failure) {
            problem = say(failure);
        }
    }

    async function load() {
        if (!accountId) return;
        const mine = ++loading;
        try {
            const [found, listed] = await Promise.all([listContacts(accountId, query.trim(), folderId), listContactFolders(accountId)]);
            if (mine !== loading) return;
            contacts = found;
            folders = listed;
            if (selected) selected = found.find((each) => each.id === selected?.id) ?? null;
            // The organisation is looked in once there is enough typed to look for.
            if (query.trim().length >= 2) {
                const looked = await searchPeople(accountId, query.trim());
                if (mine !== loading) return;
                others = looked.people.filter((each) => each.source === "directory");
                asked = looked.directory_searched;
            } else {
                others = [];
                asked = true;
            }
        } catch (failure) {
            if (mine === loading) problem = say(failure);
        }
    }

    $effect(() => {
        const first = accounts[0];
        if (first && !accountId) accountId = first.id;
    });

    let waiting: ReturnType<typeof setTimeout> | undefined;
    $effect(() => {
        void accountId;
        void folderId;
        void query;
        clearTimeout(waiting);
        waiting = setTimeout(load, query ? 200 : 0);
        return () => clearTimeout(waiting);
    });

    // ---- the form ---------------------------------------------------------
    let form = $state({ given: "", surname: "", company: "", title: "", department: "", emails: "", business: "", home: "", mobile: "", notes: "" });
    let was = { given: "", surname: "", company: "", title: "", department: "", emails: "", business: "", home: "", mobile: "", notes: "" };
    let saving = $state(false);
    let formProblem = $state<string | null>(null);

    function open(contact: Contact | null, from?: Found) {
        const [given, ...rest] = (from?.name ?? "").split(" ");
        form = {
            given: contact?.given_name ?? given ?? "",
            surname: contact?.surname ?? rest.join(" "),
            company: contact?.company ?? from?.company ?? "",
            title: contact?.job_title ?? from?.job_title ?? "",
            department: contact?.department ?? from?.department ?? "",
            emails: contact ? contact.emails.map((each) => each.address).join("\n") : (from?.address ?? ""),
            business: contact ? contact.business_phones.join("\n") : (from?.phones ?? []).join("\n"),
            home: (contact?.home_phones ?? []).join("\n"),
            mobile: contact?.mobile ?? "",
            notes: contact?.notes ?? "",
        };
        // A new contact sends everything written; a changed one only what was changed.
        was = contact ? { ...form } : { given: "", surname: "", company: "", title: "", department: "", emails: "", business: "", home: "", mobile: "", notes: "" };
        formProblem = null;
        editing = contact ? "change" : "new";
    }

    const lines = (text: string) =>
        text
            .split(/[\n,;]/)
            .map((each) => each.trim())
            .filter(Boolean);

    async function save(submitted: SubmitEvent) {
        submitted.preventDefault();
        if (!accountId) return;
        const fields: ContactFields = {};
        if (form.given !== was.given) fields.given_name = form.given;
        if (form.surname !== was.surname) fields.surname = form.surname;
        if (form.company !== was.company) fields.company = form.company;
        if (form.title !== was.title) fields.job_title = form.title;
        if (form.department !== was.department) fields.department = form.department;
        if (form.emails !== was.emails) fields.emails = lines(form.emails);
        if (form.business !== was.business) fields.business_phones = lines(form.business);
        if (form.home !== was.home) fields.home_phones = lines(form.home);
        if (form.mobile !== was.mobile) fields.mobile = form.mobile;
        if (form.notes !== was.notes) fields.notes = form.notes;
        if (Object.keys(fields).length === 0) {
            editing = null;
            return;
        }
        saving = true;
        formProblem = null;
        try {
            const saved = editing === "change" && selected ? await updateContact(accountId, selected.id, fields) : await createContact(accountId, folderId, fields);
            editing = null;
            selected = saved;
            await load();
        } catch (failure) {
            formProblem = say(failure);
        } finally {
            saving = false;
        }
    }

    async function remove() {
        if (!accountId || !selected) return;
        try {
            await deleteContact(accountId, selected.id);
            selected = null;
            sure = false;
            await load();
        } catch (failure) {
            problem = say(failure);
        }
    }

    function keydown(event: KeyboardEvent) {
        if (event.key === "Escape" && editing) editing = null;
    }
</script>

<svelte:window onkeydown={keydown} />

<div class="people">
    <aside aria-label="Folders of contacts">
        <button type="button" class="primary new" onclick={() => ((selected = null), open(null))}><Plus size={16} aria-hidden="true" />New contact</button>
        <ul>
            <li><button type="button" class:current={folderId === null} aria-current={folderId === null} onclick={() => (folderId = null)}>All contacts</button></li>
            {#each folders as each (each.id)}
                <li><button type="button" class:current={folderId === each.id} aria-current={folderId === each.id} onclick={() => (folderId = each.id)}>{each.name}</button></li>
            {/each}
        </ul>
        <div class="files">
            <button type="button" title="Add the contacts in a vCard file (.vcf)" onclick={importFile}>Import…</button>
            <button type="button" title="Write these contacts to a vCard file (.vcf)" onclick={exportFile}>Export…</button>
        </div>
    </aside>

    <section class="list" aria-label="People">
        <input type="search" aria-label="Search people" placeholder="Search contacts and your organisation" bind:value={query} />
        {#if problem}<p class="problem" role="alert">{problem}</p>{/if}
        {#if filed}<p class="muted none" role="status">{filed}</p>{/if}
        <ul aria-label="Contacts">
            {#each contacts as each (each.id)}
                <li>
                    <button type="button" class="row" class:current={selected?.id === each.id} aria-current={selected?.id === each.id} onclick={() => ((selected = each), (editing = null), (sure = false))}>
                        <span class="face" aria-hidden="true">{initials(each.name)}</span>
                        <span class="who">
                            <span class="name">{each.name || "(No name)"}</span>
                            <span class="muted">{[each.job_title, each.company].filter(Boolean).join(" · ") || (each.emails[0]?.address ?? "")}</span>
                        </span>
                    </button>
                </li>
            {:else}
                <li class="muted none">{query.trim() ? "No contact matches." : "No contacts yet."}</li>
            {/each}
        </ul>
        {#if others.length > 0}
            <h2 id="organisation-heading">In your organisation</h2>
            <ul aria-labelledby="organisation-heading">
                {#each others as each (each.address)}
                    <li class="row other">
                        <span class="face" aria-hidden="true">{initials(each.name)}</span>
                        <span class="who">
                            <span class="name">{each.name || each.address}</span>
                            <span class="muted">{[each.job_title, each.department].filter(Boolean).join(" · ") || each.address}</span>
                            {#if each.job_title || each.department}<span class="muted">{each.address}</span>{/if}
                        </span>
                        <button type="button" aria-label={`Add ${each.name || each.address} to contacts`} title="Add to contacts" onclick={() => ((selected = null), open(null, each))}
                            ><UserPlus size={16} aria-hidden="true" /></button
                        >
                    </li>
                {/each}
            </ul>
        {:else if !asked && query.trim().length >= 2}
            <p class="muted none">Your organisation could not be searched just now. Contacts on this computer are shown.</p>
        {/if}
    </section>

    <section class="card" aria-label="Contact">
        {#if editing}
            <form aria-label={editing === "new" ? "New contact" : "Change contact"} onsubmit={save}>
                <h2>{editing === "new" ? "New contact" : "Change contact"}</h2>
                <div class="pair">
                    <!-- svelte-ignore a11y_autofocus -->
                    <label>First name <input type="text" autofocus bind:value={form.given} /></label>
                    <label>Last name <input type="text" bind:value={form.surname} /></label>
                </div>
                <div class="pair">
                    <label>Company <input type="text" bind:value={form.company} /></label>
                    <label>Job title <input type="text" bind:value={form.title} /></label>
                </div>
                <label>Department <input type="text" bind:value={form.department} /></label>
                <label>Email addresses <textarea rows="2" placeholder="One to a line, three at most" bind:value={form.emails}></textarea></label>
                <div class="pair">
                    <label>Mobile <input type="text" bind:value={form.mobile} /></label>
                    <label>Work phones <textarea rows="1" bind:value={form.business}></textarea></label>
                </div>
                <label>Home phones <textarea rows="1" bind:value={form.home}></textarea></label>
                <label>Notes <textarea rows="4" bind:value={form.notes}></textarea></label>
                {#if formProblem}<p class="problem" role="alert">{formProblem}</p>{/if}
                <div class="buttons">
                    <button type="submit" class="primary" disabled={saving}>{saving ? "Saving…" : "Save"}</button>
                    <button type="button" disabled={saving} onclick={() => (editing = null)}>Cancel</button>
                </div>
            </form>
        {:else if selected}
            {@const contact = selected}
            <div class="head">
                <span class="face big" aria-hidden="true">{initials(contact.name)}</span>
                <div>
                    <h2>{contact.name || "(No name)"}</h2>
                    <p class="muted">{[contact.job_title, contact.department, contact.company].filter(Boolean).join(" · ")}</p>
                </div>
                <span class="tools">
                    {#if sure}
                        <button type="button" class="danger" onclick={remove}>Delete for good?</button>
                        <button type="button" onclick={() => (sure = false)}>Keep</button>
                    {:else}
                        <button type="button" aria-label="Change" title="Change" onclick={() => open(contact)}><Pencil size={16} aria-hidden="true" /></button>
                        <button type="button" aria-label="Delete" title="Delete" onclick={() => (sure = true)}><Trash size={16} aria-hidden="true" /></button>
                    {/if}
                </span>
            </div>
            <dl>
                {#each contact.emails as each (each.address)}<dt>Email</dt><dd>{each.address}</dd>{/each}
                {#if contact.mobile}<dt>Mobile</dt><dd>{contact.mobile}</dd>{/if}
                {#each contact.business_phones as each (each)}<dt>Work</dt><dd>{each}</dd>{/each}
                {#each contact.home_phones as each (each)}<dt>Home</dt><dd>{each}</dd>{/each}
                {#if contact.office}<dt>Office</dt><dd>{contact.office}</dd>{/if}
                {#if contact.birthday}
                    <dt>Birthday</dt>
                    <dd>{new Date(`${contact.birthday}T00:00`).toLocaleDateString(undefined, { day: "numeric", month: "long", year: "numeric" })}</dd>
                {/if}
                {#if contact.notes}<dt>Notes</dt><dd class="notes">{contact.notes}</dd>{/if}
            </dl>
        {:else}
            <p class="muted none">Choose a contact to see their details.</p>
        {/if}
    </section>
</div>

<style>
    .files {
        display: flex;
        gap: 0.4rem;
        margin-top: 0.75rem;
    }

    .files button {
        flex: 1;
        font-size: 0.85rem;
    }

    .people {
        height: 100%;
        min-height: 0;
        display: grid;
        grid-template-columns: 12rem minmax(16rem, 24rem) minmax(0, 1fr);
    }

    aside,
    .list,
    .card {
        min-height: 0;
        overflow-y: auto;
        border-right: 1px solid var(--line);
    }

    aside {
        padding: 0.6rem;
        background: var(--raised);
    }

    .card {
        border-right: 0;
        padding: 1rem 1.25rem;
    }

    ul {
        margin: 0;
        padding: 0;
        list-style: none;
    }

    aside ul {
        margin-top: 0.6rem;
    }

    aside li button {
        width: 100%;
        border: 0;
        background: none;
        text-align: left;
        padding: 0.35rem 0.5rem;
        border-radius: 0.3rem;
    }

    aside li button.current,
    .row.current {
        background: color-mix(in srgb, var(--accent) 18%, transparent);
    }

    .new {
        display: inline-flex;
        align-items: center;
        gap: 0.3rem;
        font-size: 0.9rem;
    }

    .list input {
        width: calc(100% - 1rem);
        margin: 0.5rem;
        box-sizing: border-box;
        font: inherit;
    }

    .list h2 {
        margin: 0.75rem 0.75rem 0.25rem;
        font-size: 0.75rem;
        text-transform: uppercase;
        letter-spacing: 0.05em;
        color: color-mix(in srgb, CanvasText 60%, transparent);
    }

    .row {
        width: 100%;
        display: flex;
        align-items: center;
        gap: 0.6rem;
        padding: 0.45rem 0.75rem;
        border: 0;
        border-bottom: 1px solid var(--line);
        border-radius: 0;
        background: none;
        text-align: left;
        box-sizing: border-box;
    }

    .row.other button {
        margin-left: auto;
        flex: none;
        display: inline-grid;
        place-items: center;
    }

    .who {
        min-width: 0;
        display: flex;
        flex-direction: column;
    }

    .who span {
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    }

    .name {
        font-weight: 600;
    }

    .who .muted {
        font-size: 0.85rem;
    }

    .face {
        flex: none;
        display: inline-grid;
        place-items: center;
        width: 2rem;
        height: 2rem;
        border-radius: 50%;
        background: color-mix(in srgb, var(--accent) 22%, var(--surface));
        font-size: 0.8rem;
        font-weight: 600;
    }

    .face.big {
        width: 3.5rem;
        height: 3.5rem;
        font-size: 1.2rem;
    }

    .none {
        padding: 0.75rem;
    }

    .head {
        display: flex;
        align-items: center;
        gap: 1rem;
    }

    .head h2 {
        margin: 0;
        font-size: 1.3rem;
    }

    .head p {
        margin: 0.15rem 0 0;
    }

    .tools {
        margin-left: auto;
        display: flex;
        gap: 0.35rem;
    }

    .tools button {
        display: inline-flex;
        align-items: center;
        font-size: 0.85rem;
    }

    dl {
        display: grid;
        grid-template-columns: 6rem 1fr;
        gap: 0.4rem 1rem;
        margin: 1.25rem 0 0;
    }

    dt {
        color: color-mix(in srgb, CanvasText 60%, transparent);
    }

    dd {
        margin: 0;
    }

    .notes {
        white-space: pre-wrap;
    }

    form {
        display: flex;
        flex-direction: column;
        gap: 0.6rem;
        max-width: 34rem;
    }

    form h2 {
        margin: 0;
        font-size: 1.1rem;
    }

    form label {
        display: flex;
        flex-direction: column;
        gap: 0.15rem;
        font-size: 0.85rem;
    }

    form input,
    form textarea {
        font: inherit;
        font-size: 1rem;
    }

    .pair {
        display: grid;
        grid-template-columns: 1fr 1fr;
        gap: 0.6rem;
    }

    .buttons {
        display: flex;
        gap: 0.5rem;
    }

    .problem {
        margin: 0.25rem 0.75rem;
        color: var(--danger);
    }

    form .problem {
        margin: 0;
    }
</style>
