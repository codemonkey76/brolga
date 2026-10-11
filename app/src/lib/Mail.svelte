<script lang="ts">
    import type { Account } from "./accounts";
    import { say } from "./commands";
    import { type Attachment, type Folder, type Message, type Opened, type Sort, type View, getConversation, listAttachments, listFolders, listMessages, openLink, openMessage, saveAttachment, searchMail, setFavourite, setTrustedSender, syncNow } from "./mail";
    import MessageList from "./MessageList.svelte";
    import ReadingPane from "./ReadingPane.svelte";

    interface Props {
        accounts: Account[];
    }

    let { accounts }: Props = $props();

    const PAGE = 100;
    let accountId = $state<string | null>(null);
    let folders = $state<Folder[]>([]);
    let folderId = $state<string | null>(null);
    let total = $state(0);
    let rows = $state<(Message | undefined)[]>([]);
    /** Bumped whenever the list is replaced, so a slow answer for the old one is dropped. */
    let listing = 0;
    const asked = new Set<number>();

    let query = $state("");
    let searching = $state(false);
    let opened = $state<Opened | null>(null);
    let selectedId = $state<string | null>(null);
    /** The message being read, which in a conversation need not be the one selected in the list. */
    let readingId: string | null = null;
    /** The whole conversation of the selected line, oldest first, when it has more than one message. */
    let thread = $state<Message[]>([]);

    /** What is attached to the message being read, apart from pictures within its body. */
    let attachments = $state<Attachment[]>([]);
    /** What became of the last attachment saved. */
    let saved = $state<string | null>(null);

    async function save(attachment: Attachment) {
        if (!accountId || !opened) return;
        saved = null;
        try {
            const result = await saveAttachment(accountId, opened.message.id, attachment.id);
            saved = `Saved to ${result.path}`;
        } catch (failure) {
            saved = say(failure);
        }
    }

    /** How the list is laid out. This window's business only. */
    let view = $state<View>({ sort: "newest", conversations: false, plain: false, pane: "right" });
    const SORTS: Sort[] = ["newest", "oldest", "sender", "subject"];
    try {
        const kept: unknown = JSON.parse(localStorage.getItem("brolga.view") ?? "null");
        if (kept && typeof kept === "object") {
            const { sort, conversations, plain, pane } = kept as Partial<View>;
            view = {
                sort: SORTS.includes(sort as Sort) ? (sort as Sort) : "newest",
                conversations: conversations === true,
                plain: plain === true,
                pane: pane === "bottom" ? "bottom" : "right",
            };
        }
    } catch {
        // Nothing kept: newest first, a line for each message.
    }

    async function lay(next: View) {
        const relist = next.sort !== view.sort || next.conversations !== view.conversations;
        const reread = next.plain !== view.plain;
        view = next;
        try {
            localStorage.setItem("brolga.view", JSON.stringify(next));
        } catch {
            // It still holds for as long as the window is open.
        }
        if (reread && opened) void read(opened.message);
        if (relist && folderId && !searching) {
            reset();
            await need(0);
        }
    }

    async function trust(address: string, trusted: boolean) {
        if (!accountId || !opened) return;
        try {
            await setTrustedSender(accountId, address, trusted);
            await read(opened.message);
        } catch (failure) {
            readingProblem = say(failure);
        }
    }
    let opening = $state(false);
    let readingProblem = $state<string | null>(null);
    let imagesShown = $state(false);
    let syncing = $state(false);
    let problem = $state<string | null>(null);
    let searchBox = $state<HTMLInputElement>();

    const account = $derived(accounts.find((a) => a.id === accountId) ?? null);
    const folder = $derived(folders.find((f) => f.id === folderId) ?? null);
    const favourites = $derived(folders.filter((f) => f.favourite));

    /** Folders closed up in the tree. Which ones is this window's business only. */
    let collapsed = $state<string[]>([]);
    const collapsedKey = (id: string) => `brolga.collapsed.${id}`;

    function recallCollapsed() {
        collapsed = [];
        if (!accountId) return;
        try {
            const kept: unknown = JSON.parse(localStorage.getItem(collapsedKey(accountId)) ?? "[]");
            if (Array.isArray(kept)) collapsed = kept.filter((each) => typeof each === "string");
        } catch {
            // Nothing kept, or nowhere to keep it: every folder starts open.
        }
    }

    function toggle(id: string) {
        collapsed = collapsed.includes(id) ? collapsed.filter((each) => each !== id) : [...collapsed, id];
        try {
            if (accountId) localStorage.setItem(collapsedKey(accountId), JSON.stringify(collapsed));
        } catch {
            // It still works for as long as the window is open.
        }
    }

    /** The tree as it shows: nothing inside a closed folder, and which folders have something inside. */
    const tree = $derived.by(() => {
        const shown: { folder: Folder; parent: boolean; open: boolean }[] = [];
        let closedAt: number | null = null;
        folders.forEach((each, index) => {
            if (closedAt !== null && each.depth > closedAt) return;
            const open = !collapsed.includes(each.id);
            closedAt = open ? null : each.depth;
            shown.push({ folder: each, parent: (folders[index + 1]?.depth ?? 0) > each.depth, open });
        });
        return shown;
    });

    async function pin(each: Folder) {
        if (!accountId) return;
        try {
            await setFavourite(accountId, each.id, !each.favourite);
            folders = await listFolders(accountId);
        } catch (failure) {
            problem = say(failure);
        }
    }

    async function loadFolders() {
        if (!accountId) return;
        folders = await listFolders(accountId);
        if (!folders.some((f) => f.id === folderId)) {
            const inbox = folders.find((f) => f.well_known === "inbox") ?? folders[0];
            if (inbox) await showFolder(inbox.id);
        }
    }

    function reset() {
        listing += 1;
        asked.clear();
        rows = [];
        total = 0;
    }

    async function showFolder(id: string) {
        folderId = id;
        query = "";
        searching = false;
        reset();
        await need(0);
    }

    /** Load the page of the folder that holds this position, once. */
    async function need(index: number) {
        if (!accountId || !folderId || searching) return;
        const offset = Math.floor(index / PAGE) * PAGE;
        if (asked.has(offset)) return;
        asked.add(offset);

        const mine = listing;
        try {
            const page = await listMessages(accountId, folderId, offset, PAGE, view);
            if (mine !== listing) return;
            total = page.total;
            const next = rows.slice();
            page.messages.forEach((message, i) => (next[offset + i] = message));
            rows = next;
        } catch (failure) {
            asked.delete(offset);
            problem = say(failure);
        }
    }

    /** Fetch again what is on screen, in place, so new mail appears without the list jumping. */
    async function refresh() {
        if (!accountId || !folderId || searching) return;
        const mine = listing;
        const next = rows.slice();
        let now = total;
        for (const offset of [...asked].sort((a, b) => a - b)) {
            const page = await listMessages(accountId, folderId, offset, PAGE, view);
            if (mine !== listing) return;
            now = page.total;
            for (let i = 0; i < PAGE; i++) next[offset + i] = page.messages[i];
        }
        next.length = Math.min(next.length, now);
        rows = next;
        total = now;
    }

    async function search() {
        if (!accountId) return;
        if (!query.trim()) {
            if (folderId) await showFolder(folderId);
            return;
        }
        searching = true;
        reset();
        const mine = listing;
        try {
            const found = await searchMail(accountId, query);
            if (mine !== listing) return;
            rows = found;
            total = found.length;
        } catch (failure) {
            problem = say(failure);
        }
    }

    /** Choose a line of the list. For a conversation, its other messages are fetched to read beside it. */
    async function open(message: Message) {
        selectedId = message.id;
        thread = [];
        const reading = read(message);
        if (accountId && view.conversations && !searching && message.conversation_id) {
            try {
                const all = await getConversation(accountId, message.conversation_id);
                if (selectedId === message.id && all.length > 1) thread = all;
            } catch {
                // The message itself still opens; it is only shown on its own.
            }
        }
        await reading;
    }

    async function read(message: Message, withImages = false) {
        if (!accountId) return;
        readingId = message.id;
        imagesShown = withImages;
        if (opened?.message.id !== message.id) {
            attachments = [];
            saved = null;
        }
        opening = true;
        readingProblem = null;
        try {
            const result = await openMessage(accountId, message.id, withImages, view.plain);
            if (readingId !== message.id) return;
            opened = result;
            imagesShown = result.remote_content_allowed;
            if (result.message.has_attachments && attachments.length === 0) {
                try {
                    const all = await listAttachments(accountId, message.id);
                    if (readingId === message.id) attachments = all.filter((each) => !each.is_inline);
                } catch (failure) {
                    if (readingId === message.id) saved = say(failure);
                }
            }
        } catch (failure) {
            if (readingId === message.id) {
                opened = null;
                readingProblem = say(failure);
            }
        } finally {
            opening = false;
        }
    }

    async function sync() {
        if (syncing) return;
        syncing = true;
        problem = null;
        try {
            const results = await syncNow();
            const failed = results.find((r) => r.account_id === accountId && !r.ok) ?? results.find((r) => !r.ok);
            if (failed) problem = failed.error;
            await loadFolders();
            await refresh();
        } catch (failure) {
            problem = say(failure);
        } finally {
            syncing = false;
            lastSync = Date.now();
        }
    }

    // New mail is fetched every two minutes while the window is open, and
    // when the window is come back to after half a minute or more away.
    const EVERY = 2 * 60_000;
    let lastSync = 0;
    $effect(() => {
        const timer = setInterval(sync, EVERY);
        const back = () => Date.now() - lastSync > 30_000 && sync();
        window.addEventListener("focus", back);

        return () => {
            clearInterval(timer);
            window.removeEventListener("focus", back);
        };
    });

    function keydown(event: KeyboardEvent) {
        if (event.key === "F9") {
            event.preventDefault();
            sync();
        } else if (event.key === "F3" || ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "e")) {
            event.preventDefault();
            searchBox?.focus();
            searchBox?.select();
        }
    }

    async function chooseAccount(id: string) {
        accountId = id;
        recallCollapsed();
        folderId = null;
        opened = null;
        selectedId = null;
        reset();
        await loadFolders().catch((failure) => (problem = say(failure)));
    }

    // Show what is already on this computer at once, then fetch what is new.
    $effect(() => {
        const firstAccount = accounts[0];
        if (!accountId && firstAccount) {
            chooseAccount(firstAccount.id).then(sync);
        }
    });
</script>

<svelte:window onkeydown={keydown} />

<div class="mail" class:bottom={view.pane === "bottom"}>
    <nav class="folders" aria-label="Folders">
        {#if accounts.length > 1}
            <select aria-label="Account" value={accountId} onchange={(event) => chooseAccount(event.currentTarget.value)}>
                {#each accounts as each (each.id)}
                    <option value={each.id}>{each.username}</option>
                {/each}
            </select>
        {:else if account}
            <p class="account" title={account.username}>{account.username}</p>
        {/if}

        {#snippet row(each: Folder, parent: boolean, open: boolean, indent: number)}
            <li style:padding-left="{indent * 0.9}rem">
                {#if parent}
                    <button type="button" class="twist" aria-expanded={open} aria-label="{open ? 'Collapse' : 'Expand'} {each.name}" onclick={() => toggle(each.id)}>
                        {open ? "▾" : "▸"}
                    </button>
                {:else}
                    <span class="twist"></span>
                {/if}
                <button type="button" class="folder" class:current={each.id === folderId && !searching} aria-current={each.id === folderId && !searching} onclick={() => showFolder(each.id)}>
                    <span class="name">{each.name}</span>
                    {#if each.unread > 0}<span class="count" aria-label="{each.unread} unread">{each.unread}</span>{/if}
                </button>
                <button type="button" class="pin" class:pinned={each.favourite} aria-pressed={each.favourite} aria-label="{each.favourite ? 'Unpin' : 'Pin'} {each.name}" title={each.favourite ? "Remove from favourites" : "Add to favourites"} onclick={() => pin(each)}>
                    {each.favourite ? "★" : "☆"}
                </button>
            </li>
        {/snippet}

        {#if favourites.length > 0}
            <h2 id="favourites-heading">Favourites</h2>
            <ul aria-labelledby="favourites-heading">
                {#each favourites as each (each.id)}
                    {@render row(each, false, true, 0)}
                {/each}
            </ul>
            <h2 id="folders-heading">Folders</h2>
        {:else}
            <h2 id="folders-heading" class="unseen">Folders</h2>
        {/if}
        <ul aria-labelledby="folders-heading">
            {#each tree as each (each.folder.id)}
                {@render row(each.folder, each.parent, each.open, each.folder.depth)}
            {/each}
        </ul>
    </nav>

    <div class="list">
        <div class="bar">
            <form role="search" onsubmit={(event) => (event.preventDefault(), search())}>
                <input
                    bind:this={searchBox}
                    bind:value={query}
                    type="search"
                    aria-label="Search mail"
                    placeholder="Search mail"
                    oninput={() => query === "" && search()}
                />
            </form>
            <button type="button" disabled={syncing} onclick={sync} title="Sync now (F9)">{syncing ? "Syncing…" : "Sync"}</button>
        </div>

        {#if problem}
            <p class="problem" role="alert">{problem}</p>
        {/if}

        <p class="heading">
            {#if searching}
                {total === 1 ? "1 result" : `${total} results`} for “{query}”
            {:else if folder}
                {@const things = view.conversations ? "conversation" : "message"}
                {folder.name} <span class="muted">· {total === 1 ? `1 ${things}` : `${total.toLocaleString()} ${things}s`}</span>
            {/if}
        </p>

        {#if !searching}
            <div class="view">
                <label>
                    <input type="checkbox" checked={view.conversations} onchange={(event) => lay({ ...view, conversations: event.currentTarget.checked })} />
                    Conversations
                </label>
                <label>
                    Sort
                    <select
                        value={view.conversations ? "newest" : view.sort}
                        disabled={view.conversations}
                        title={view.conversations ? "Conversations are always newest first" : undefined}
                        onchange={(event) => lay({ ...view, sort: event.currentTarget.value as Sort })}
                    >
                        <option value="newest">Newest first</option>
                        <option value="oldest">Oldest first</option>
                        <option value="sender">By sender</option>
                        <option value="subject">By subject</option>
                    </select>
                </label>
                <label>
                    <input type="checkbox" checked={view.plain} onchange={(event) => lay({ ...view, plain: event.currentTarget.checked })} />
                    Plain text
                </label>
                <label>
                    Reading pane
                    <select value={view.pane} onchange={(event) => lay({ ...view, pane: event.currentTarget.value === "bottom" ? "bottom" : "right" })}>
                        <option value="right">Right</option>
                        <option value="bottom">Bottom</option>
                    </select>
                </label>
            </div>
        {/if}

        {#if total === 0}
            <p class="empty muted">{searching ? "Nothing matches." : syncing ? "Fetching your mail…" : "Nothing here."}</p>
        {:else}
            <div class="rows">
                <MessageList {total} {rows} {selectedId} onSelect={open} onNeed={need} />
            </div>
        {/if}
    </div>

    <ReadingPane
        {opened}
        loading={opening}
        problem={readingProblem}
        {imagesShown}
        {thread}
        {attachments}
        {saved}
        onSave={save}
        onTrust={trust}
        onRead={(message) => read(message)}
        onShowImages={() => opened && read(opened.message, true)}
        onOpenLink={(url) => openLink(url).catch((failure) => (readingProblem = say(failure)))}
    />
</div>

<style>
    .mail {
        height: 100%;
        display: grid;
        grid-template-columns: 13rem minmax(16rem, 24rem) minmax(0, 1fr);
        min-height: 0;
    }

    .folders {
        border-right: 1px solid var(--line);
        background: var(--raised);
        overflow-y: auto;
        padding: 0.5rem;
        display: flex;
        flex-direction: column;
        gap: 0.5rem;
    }

    .account {
        margin: 0;
        padding: 0.25rem 0.5rem;
        font-size: 0.8rem;
        font-weight: 600;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    }

    ul {
        margin: 0;
        padding: 0;
        list-style: none;
    }

    h2 {
        margin: 0;
        padding: 0.25rem 0.5rem 0;
        font-size: 0.7rem;
        font-weight: 600;
        letter-spacing: 0.04em;
        text-transform: uppercase;
        color: var(--muted);
    }

    h2.unseen {
        position: absolute;
        width: 1px;
        height: 1px;
        overflow: hidden;
        clip-path: inset(50%);
    }

    li {
        display: flex;
        align-items: center;
    }

    .folders button {
        border: 0;
        background: none;
        font-size: 0.9rem;
    }

    .twist {
        flex: none;
        width: 1.1rem;
        padding: 0;
        color: var(--muted);
    }

    .folder {
        flex: 1;
        min-width: 0;
        display: flex;
        justify-content: space-between;
        gap: 0.5rem;
        padding: 0.35rem 0.4rem;
        text-align: left;
    }

    .folder.current {
        background: color-mix(in srgb, var(--accent) 18%, var(--surface));
        font-weight: 600;
    }

    /* The pin keeps out of the way until the row is pointed at or reached by keyboard. */
    .pin {
        flex: none;
        padding: 0 0.3rem;
        color: var(--muted);
        opacity: 0;
    }

    .pin.pinned,
    li:hover .pin,
    .pin:focus-visible {
        opacity: 1;
    }

    .name {
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    }

    .count {
        font-weight: 700;
        font-size: 0.8rem;
    }

    .mail.bottom {
        grid-template-columns: 13rem minmax(0, 1fr);
        grid-template-rows: minmax(8rem, 40%) minmax(0, 1fr);
    }

    .mail.bottom .folders {
        grid-row: 1 / 3;
    }

    .mail.bottom .list {
        border-right: 0;
        border-bottom: 1px solid var(--line);
    }

    .view {
        display: flex;
        flex-wrap: wrap;
        justify-content: space-between;
        align-items: center;
        gap: 0.5rem;
        padding: 0.4rem 0.75rem;
        font-size: 0.8rem;
        color: var(--muted);
    }

    .view label {
        display: flex;
        align-items: center;
        gap: 0.3rem;
    }

    .view select {
        font-size: 0.8rem;
        padding: 0.1rem 0.25rem;
    }

    .list {
        border-right: 1px solid var(--line);
        display: flex;
        flex-direction: column;
        min-height: 0;
        min-width: 0;
    }

    .bar {
        display: flex;
        gap: 0.5rem;
        padding: 0.5rem;
        border-bottom: 1px solid var(--line);
    }

    .bar form {
        flex: 1;
        min-width: 0;
    }

    .bar input {
        width: 100%;
        box-sizing: border-box;
    }

    .heading {
        margin: 0;
        padding: 0.4rem 0.75rem;
        font-size: 0.85rem;
        font-weight: 600;
        border-bottom: 1px solid var(--line);
    }

    .heading .muted {
        font-weight: 400;
    }

    .rows {
        flex: 1;
        min-height: 0;
    }

    .empty {
        margin: 2rem auto;
    }

    .problem {
        margin: 0.5rem;
        padding: 0.5rem 0.75rem;
        border-radius: 0.4rem;
        background: color-mix(in srgb, var(--danger) 12%, var(--surface));
        font-size: 0.85rem;
    }
</style>
