<script lang="ts">
    import type { Account } from "./accounts";
    import { say } from "./commands";
    import { type Folder, type Message, type Opened, listFolders, listMessages, openLink, openMessage, searchMail, syncNow } from "./mail";
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
    let opening = $state(false);
    let readingProblem = $state<string | null>(null);
    let imagesShown = $state(false);
    let syncing = $state(false);
    let problem = $state<string | null>(null);
    let searchBox = $state<HTMLInputElement>();

    const account = $derived(accounts.find((a) => a.id === accountId) ?? null);
    const folder = $derived(folders.find((f) => f.id === folderId) ?? null);

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
            const page = await listMessages(accountId, folderId, offset, PAGE);
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

    async function open(message: Message, withImages = false) {
        if (!accountId) return;
        selectedId = message.id;
        imagesShown = withImages;
        opening = true;
        readingProblem = null;
        try {
            const result = await openMessage(accountId, message.id, withImages);
            if (selectedId === message.id) opened = result;
        } catch (failure) {
            if (selectedId === message.id) {
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
            // What is on screen may have changed underneath.
            if (!searching && folderId) {
                reset();
                await need(0);
            }
        } catch (failure) {
            problem = say(failure);
        } finally {
            syncing = false;
        }
    }

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

<div class="mail">
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

        <ul>
            {#each folders as each (each.id)}
                <li>
                    <button type="button" class:current={each.id === folderId && !searching} aria-current={each.id === folderId && !searching} onclick={() => showFolder(each.id)}>
                        <span class="name">{each.name}</span>
                        {#if each.unread > 0}<span class="count" aria-label="{each.unread} unread">{each.unread}</span>{/if}
                    </button>
                </li>
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
                {folder.name} <span class="muted">· {total === 1 ? "1 message" : `${total.toLocaleString()} messages`}</span>
            {/if}
        </p>

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
        onShowImages={() => opened && open(opened.message, true)}
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

    .folders button {
        width: 100%;
        display: flex;
        justify-content: space-between;
        gap: 0.5rem;
        padding: 0.35rem 0.5rem;
        border: 0;
        background: none;
        text-align: left;
        font-size: 0.9rem;
    }

    .folders button.current {
        background: color-mix(in srgb, var(--accent) 18%, var(--surface));
        font-weight: 600;
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
