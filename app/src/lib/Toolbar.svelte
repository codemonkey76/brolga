<script lang="ts">
    import {
        Archive,
        CalendarClock,
        CircleCheck,
        Flag,
        FlagOff,
        Forward,
        Mail as Unread,
        MailOpen,
        OctagonAlert,
        Paperclip,
        RefreshCw,
        Reply,
        ReplyAll,
        ShieldCheck,
        SquarePen,
        Tag,
        Shredder,
        Trash,
        X,
    } from "@lucide/svelte";
    import { type Category, type FlagStatus, type Folder, type Message, categoryColour, dueSaid, flagDays } from "./mail";

    interface Props {
        /** The message the actions are for. None, and they are greyed out. */
        message: Message | null;
        /** Every folder, to move the message to one. */
        folders: Folder[];
        /** The standard folder the message is in, if it is in one. */
        within: string | null;
        categories: Category[];
        syncing: boolean;
        onWrite: (kind: "new" | "reply" | "reply_all" | "forward" | "forward_as_attachment") => void;
        onDelete: () => void;
        /** Delete for good. Offered only where a message has already been deleted once. */
        onPurge: () => void;
        onJunk: () => void;
        onNotJunk: () => void;
        onArchive: () => void;
        onMove: (folder: Folder) => void;
        onSetRead: (isRead: boolean) => void;
        /** `due` is a day, `YYYY-MM-DD`; empty for no day; left out to leave the day as it is. */
        onSetFlag: (status: FlagStatus, due?: string) => void;
        onCategorise: (names: string[]) => void;
        onSync: () => void;
    }

    let { message, folders, within, categories, syncing, onWrite, onDelete, onPurge, onJunk, onNotJunk, onArchive, onMove, onSetRead, onSetFlag, onCategorise, onSync }: Props =
        $props();

    /** The message being asked about, once "Delete permanently" has been pressed once. */
    let sureOf = $state<string | null>(null);
    let dayMenu = $state<HTMLDetailsElement>();
    const none = $derived(message === null);
    const SIZE = 18;

    function due(day: string) {
        // Closed outright: the menu is the browser's own, and is not left
        // open behind a choice made from the date box.
        if (dayMenu) dayMenu.open = false;
        onSetFlag("flagged", day);
    }
</script>

<!-- Each button is an icon alone. Its name is there for a screen reader and
     on hover, with the key that does the same. -->
<div class="toolbar" role="toolbar" aria-label="Message actions">
    <button type="button" class="primary" aria-label="New message" title="New message (Ctrl+N)" onclick={() => onWrite("new")}><SquarePen size={SIZE} aria-hidden="true" /></button>

    <span class="gap"></span>
    <button type="button" disabled={none} aria-label="Reply" title="Reply (Ctrl+R)" onclick={() => onWrite("reply")}><Reply size={SIZE} aria-hidden="true" /></button>
    <button type="button" disabled={none} aria-label="Reply all" title="Reply all (Ctrl+Shift+R)" onclick={() => onWrite("reply_all")}><ReplyAll size={SIZE} aria-hidden="true" /></button>
    <button type="button" disabled={none} aria-label="Forward" title="Forward (Ctrl+F)" onclick={() => onWrite("forward")}><Forward size={SIZE} aria-hidden="true" /></button>
    <button type="button" disabled={none} aria-label="Forward as attachment" title="Forward as attachment" onclick={() => onWrite("forward_as_attachment")}><Paperclip size={SIZE} aria-hidden="true" /></button>

    <span class="gap"></span>
    {#if within !== "deleteditems"}
        <button type="button" disabled={none} aria-label="Delete" title="Delete (Del)" onclick={onDelete}><Trash size={SIZE} aria-hidden="true" /></button>
    {:else if message && sureOf === message.id}
        <button type="button" class="danger words" onclick={() => ((sureOf = null), onPurge())}>Delete for good? It cannot be undone</button>
        <button type="button" aria-label="Keep" title="Keep it" onclick={() => (sureOf = null)}><X size={SIZE} aria-hidden="true" /></button>
    {:else}
        <button type="button" disabled={none} aria-label="Delete permanently" title="Delete permanently" onclick={() => (sureOf = message?.id ?? null)}><Shredder size={SIZE} aria-hidden="true" /></button>
    {/if}
    {#if within !== "archive" && folders.some((f) => f.well_known === "archive")}
        <button type="button" disabled={none} aria-label="Archive" title="Archive (Backspace)" onclick={onArchive}><Archive size={SIZE} aria-hidden="true" /></button>
    {/if}
    {#if within === "junkemail"}
        <button type="button" disabled={none} aria-label="Not junk" title="Not junk: move to the inbox" onclick={onNotJunk}><ShieldCheck size={SIZE} aria-hidden="true" /></button>
    {:else if within !== "deleteditems" && within !== "sentitems" && within !== "drafts" && folders.some((f) => f.well_known === "junkemail")}
        <button type="button" disabled={none} aria-label="Junk" title="Junk" onclick={onJunk}><OctagonAlert size={SIZE} aria-hidden="true" /></button>
    {/if}
    <select
        aria-label="Move to folder"
        title="Move to a folder (Ctrl+Shift+V)"
        disabled={none}
        value=""
        onchange={(event) => {
            const to = folders.find((f) => f.id === event.currentTarget.value);
            event.currentTarget.value = "";
            if (to) onMove(to);
        }}
    >
        <option value="" disabled>Move to…</option>
        {#each folders.filter((f) => f.id !== message?.folder_id) as each (each.id)}
            <option value={each.id}>{"  ".repeat(each.depth)}{each.name}</option>
        {/each}
    </select>

    <span class="gap"></span>
    {#if message?.is_read ?? true}
        <button type="button" disabled={none} aria-label="Mark as unread" title="Mark as unread (Ctrl+U)" onclick={() => onSetRead(false)}><Unread size={SIZE} aria-hidden="true" /></button>
    {:else}
        <button type="button" aria-label="Mark as read" title="Mark as read (Ctrl+Q)" onclick={() => onSetRead(true)}><MailOpen size={SIZE} aria-hidden="true" /></button>
    {/if}
    {#if message?.flag_status === "flagged"}
        <button type="button" aria-label="Mark complete" title="Mark complete" onclick={() => onSetFlag("complete")}><CircleCheck size={SIZE} aria-hidden="true" /></button>
        <button type="button" aria-label="Clear flag" title="Clear flag (Insert)" onclick={() => onSetFlag("notFlagged")}><FlagOff size={SIZE} aria-hidden="true" /></button>
    {:else if message?.flag_status === "complete"}
        <button type="button" aria-label="Clear flag" title="Clear flag" onclick={() => onSetFlag("notFlagged")}><FlagOff size={SIZE} aria-hidden="true" /></button>
    {:else}
        <button type="button" disabled={none} aria-label="Flag" title="Flag (Insert)" onclick={() => onSetFlag("flagged")}><Flag size={SIZE} aria-hidden="true" /></button>
    {/if}
    {#if message && message.flag_status !== "complete"}
        <details class="menu" bind:this={dayMenu}>
            <summary aria-label={message.flag_status === "flagged" ? "Due…" : "Flag for…"} title={message.flag_status === "flagged" ? "Change the day it is due" : "Flag for a day"}>
                <CalendarClock size={SIZE} aria-hidden="true" />
            </summary>
            <ul aria-label="Follow up by">
                {#each flagDays() as each (each.label)}
                    <li><button type="button" class="plain" onclick={() => due(each.day)}>{each.label} <span class="muted">{dueSaid(each.day)}</span></button></li>
                {/each}
                <li>
                    <label>On <input type="date" aria-label="Another day" onchange={(event) => event.currentTarget.value && due(event.currentTarget.value)} /></label>
                </li>
                {#if message.flag_due}
                    <li><button type="button" class="plain" onclick={() => due("")}>No day</button></li>
                {/if}
            </ul>
        </details>
    {/if}
    {#if message && categories.length > 0}
        {@const mine = message.categories ?? []}
        <details class="menu">
            <summary aria-label="Categories" title="Categories"><Tag size={SIZE} aria-hidden="true" /></summary>
            <ul aria-label="Categories">
                {#each categories as each (each.name)}
                    {@const has = mine.includes(each.name)}
                    <li>
                        <label>
                            <input type="checkbox" checked={has} onchange={() => onCategorise(has ? mine.filter((n) => n !== each.name) : [...mine, each.name])} />
                            <span class="dot" style:background={categoryColour(each.color)}></span>
                            {each.name}
                        </label>
                    </li>
                {/each}
            </ul>
        </details>
    {/if}
    {#if message?.flag_status === "flagged"}
        <span class="said flagged">{message.flag_due ? `Due ${dueSaid(message.flag_due)}` : "Flagged"}</span>
    {/if}
    {#if message?.flag_status === "complete"}<span class="said muted">Complete</span>{/if}

    <span class="spring"></span>
    <button type="button" disabled={syncing} aria-label={syncing ? "Syncing…" : "Sync"} title="Sync now (F9)" onclick={onSync}>
        <span class:turning={syncing}><RefreshCw size={SIZE} aria-hidden="true" /></span>
    </button>
</div>

<style>
    .toolbar {
        flex: none;
        display: flex;
        align-items: center;
        gap: 0.15rem;
        padding: 0.3rem 0.5rem;
        border-bottom: 1px solid var(--line);
        background: var(--raised);
    }

    .toolbar > button,
    .menu summary {
        display: inline-grid;
        place-items: center;
        width: 2.1rem;
        height: 2.1rem;
        padding: 0;
        border: 0;
        border-radius: 0.4rem;
        background: none;
        color: inherit;
    }

    .toolbar > button:hover:not(:disabled),
    .menu summary:hover {
        background: color-mix(in srgb, CanvasText 10%, transparent);
    }

    .toolbar > button:disabled {
        opacity: 0.35;
    }

    .toolbar > button.primary {
        background: var(--accent-fill);
        color: var(--on-accent);
    }

    .toolbar > button.words {
        width: auto;
        padding: 0 0.6rem;
        font-size: 0.85rem;
    }

    .danger {
        color: var(--danger);
    }

    .gap {
        width: 1px;
        height: 1.4rem;
        margin: 0 0.35rem;
        background: var(--line);
    }

    .spring {
        flex: 1;
    }

    select {
        max-width: 8.5rem;
        margin-left: 0.25rem;
        padding: 0.25rem 0.4rem;
        font-size: 0.85rem;
    }

    .said {
        margin-left: 0.4rem;
        font-size: 0.8rem;
        white-space: nowrap;
    }

    .flagged {
        color: var(--danger);
    }

    .turning {
        display: grid;
        animation: turn 1s linear infinite;
    }

    @keyframes turn {
        to {
            transform: rotate(360deg);
        }
    }

    .menu {
        position: relative;
    }

    .menu summary {
        list-style: none;
        cursor: default;
    }

    .menu summary::-webkit-details-marker {
        display: none;
    }

    .menu ul {
        position: absolute;
        z-index: 5;
        top: 100%;
        left: 0;
        min-width: 13rem;
        max-height: 16rem;
        overflow-y: auto;
        margin: 0.2rem 0 0;
        padding: 0.3rem;
        list-style: none;
        background: var(--surface);
        border: 1px solid var(--line);
        border-radius: 0.4rem;
        box-shadow: 0 6px 18px color-mix(in srgb, CanvasText 18%, transparent);
    }

    .menu label {
        display: flex;
        align-items: center;
        gap: 0.4rem;
        padding: 0.25rem 0.4rem;
        white-space: nowrap;
    }

    .menu .plain {
        width: 100%;
        display: flex;
        justify-content: space-between;
        gap: 1rem;
        border: 0;
        background: none;
        padding: 0.25rem 0.4rem;
        text-align: left;
    }

    .menu .plain:hover {
        background: var(--raised);
    }

    .dot {
        display: inline-block;
        width: 0.65rem;
        height: 0.65rem;
        border-radius: 50%;
    }
</style>
