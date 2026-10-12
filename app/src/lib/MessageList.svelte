<script lang="ts">
    import { Check, Flag, FlagOff, Forward, Reply, ReplyAll, Trash } from "@lucide/svelte";
    import { type Category, type Message, categoryColour, dateGroup, when, who } from "./mail";

    interface Props {
        /** How many messages there are in all, loaded or not. */
        total: number;
        /** The messages by position. A gap is one not loaded yet. */
        rows: (Message | undefined)[];
        selectedId: string | null;
        /** The lines chosen together, when there is more than one. */
        picked: string[];
        /** Choose a line alone, add it to or take it from those chosen, or choose all from where the choosing began to it. */
        onSelect: (message: Message, how: "one" | "toggle" | "range") => void;
        /** Choose every line. */
        onAll: () => void;
        /** Flag a line, or clear its flag, from the line itself. */
        onFlag: (message: Message) => void;
        /** Delete a line from the line itself. Left out where there is nowhere further to delete to. */
        onDelete?: (message: Message) => void;
        /** Asked for when a position on screen has not been loaded. */
        onNeed: (index: number) => void;
        /** The mailbox's categories, for the colour of each. */
        categories: Category[];
        /** The list is in date order, so it can be marked off into today, yesterday and so on. */
        byDate: boolean;
    }

    let { total, rows, selectedId, picked, onSelect, onAll, onFlag, onDelete, onNeed, byDate, categories }: Props = $props();

    const chosen = $derived(new Set(picked.length > 1 ? picked : selectedId ? [selectedId] : []));

    // Only the rows on screen are in the page, however long the folder is,
    // so a folder of a hundred thousand scrolls like one of a hundred.
    const ROW = 68;
    const SPARE = 8;
    let viewport = $state<HTMLDivElement>();
    let scrollTop = $state(0);
    let height = $state(600);

    const first = $derived(Math.max(0, Math.floor(scrollTop / ROW) - SPARE));
    const last = $derived(Math.min(total, Math.ceil((scrollTop + height) / ROW) + SPARE));
    const visible = $derived(Array.from({ length: Math.max(0, last - first) }, (_, i) => first + i));

    // Rows are all one height, which is what lets a long folder scroll
    // freely, so a stretch of time is named over the list and marked where
    // it starts, with no heading rows between the messages.
    const group = (index: number) => {
        const row = rows[index];
        return byDate && row ? dateGroup(row.received_at) : "";
    };
    const top = $derived(group(Math.min(Math.max(0, total - 1), Math.floor(scrollTop / ROW))));
    const starts = (index: number) => index > 0 && group(index) !== "" && group(index - 1) !== "" && group(index) !== group(index - 1);

    $effect(() => {
        for (const index of visible) {
            if (!rows[index]) {
                onNeed(index);
                break;
            }
        }
    });

    /** What has been done with a message, to mark its line with. */
    const DONE = {
        replied: { shape: Reply, said: "Replied" },
        replied_all: { shape: ReplyAll, said: "Replied to all" },
        forwarded: { shape: Forward, said: "Forwarded" },
    };

    /** A sender's initials, for the circle beside their mail. */
    function initials(name: string) {
        const words = name.split(/[^\p{L}\p{N}]+/u).filter(Boolean);
        const last = words.length > 1 ? words[words.length - 1] : "";
        return ((words[0]?.[0] ?? "?") + (last[0] ?? "")).toUpperCase();
    }

    // Dark enough for white letters. One sender is always the same colour.
    const CIRCLES = ["#1d4a8c", "#52307a", "#7a1f40", "#005a52", "#8f1d1f", "#5c3a20", "#3c4f5a", "#1f5f1f"];
    function circle(of: string) {
        let sum = 0;
        for (const letter of of) sum = (sum * 31 + letter.charCodeAt(0)) >>> 0;
        return CIRCLES[sum % CIRCLES.length];
    }

    /** Something done from a line with the mouse, which is not also a click on the line. */
    function act(event: MouseEvent, what: () => void) {
        event.stopPropagation();
        what();
    }

    function move(by: number, how: "one" | "range") {
        const at = rows.findIndex((row) => row?.id === selectedId);
        const to = Math.min(total - 1, Math.max(0, at < 0 ? 0 : at + by));
        const row = rows[to];
        if (!row) return onNeed(to);
        onSelect(row, how);
        // Keep the selection on screen.
        if (viewport) {
            if (to * ROW < viewport.scrollTop) viewport.scrollTop = to * ROW;
            else if ((to + 1) * ROW > viewport.scrollTop + height) viewport.scrollTop = (to + 1) * ROW - height;
        }
    }

    function keydown(event: KeyboardEvent) {
        if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "a") {
            event.preventDefault();
            onAll();
            return;
        }
        const moves: Record<string, number> = { ArrowDown: 1, ArrowUp: -1, PageDown: 8, PageUp: -8, Home: -total, End: total };
        const by = moves[event.key];
        if (by === undefined) return;
        event.preventDefault();
        // With Shift held, everything from where the choosing began to here is chosen.
        move(by, event.shiftKey ? "range" : "one");
    }
</script>

{#if top}
    <p class="stretch">{top}</p>
{/if}
<div
    class="viewport"
    role="listbox"
    aria-label="Messages"
    aria-multiselectable="true"
    tabindex="0"
    bind:this={viewport}
    bind:clientHeight={height}
    onscroll={() => (scrollTop = viewport?.scrollTop ?? 0)}
    onkeydown={keydown}
>
    <div class="spacer" style:height="{total * ROW}px">
        {#each visible as index (index)}
            {@const message = rows[index]}
            <div
                class="row"
                class:selected={message && chosen.has(message.id)}
                class:cursor={picked.length > 1 && message?.id === selectedId}
                class:unread={message && (message.conversation ? message.conversation.unread > 0 : !message.is_read)}
                style:transform="translateY({index * ROW}px)"
                role="option"
                aria-selected={!!message && chosen.has(message.id)}
                aria-setsize={total}
                aria-posinset={index + 1}
                tabindex="-1"
                onclick={(event) => message && onSelect(message, event.shiftKey ? "range" : event.ctrlKey || event.metaKey ? "toggle" : "one")}
                onkeydown={() => {}}
            >
                {#if starts(index)}
                    <span class="starts">{group(index)}</span>
                {/if}
                {#if message}
                    {@const sender = who(message.from)}
                    {@const ticked = picked.length > 1 && chosen.has(message.id)}
                    <!-- The circle, the box and the buttons are for the mouse. The keys and the toolbar do the same, and say so to a screen reader. -->
                    <span class="lead" class:ticked aria-hidden="true">
                        <span class="circle" style:background={circle(message.from.address ?? sender)}>{initials(sender)}</span>
                        <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
                        <span class="box" title="Select" onclick={(event) => act(event, () => onSelect(message, "toggle"))}>{#if ticked}<Check size={13} />{/if}</span>
                    </span>
                    <!-- Not buttons: a line of a list cannot hold one, and these are never reached by key. -->
                    <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
                    <span class="acts" aria-hidden="true">
                        <span class="act" title={message.flag_status === "flagged" ? "Clear flag" : "Flag"} onclick={(event) => act(event, () => onFlag(message))}>
                            {#if message.flag_status === "flagged"}<FlagOff size={15} />{:else}<Flag size={15} />{/if}
                        </span>
                        {#if onDelete}
                            <span class="act" title="Delete" onclick={(event) => act(event, () => onDelete(message))}><Trash size={15} /></span>
                        {/if}
                    </span>
                    <span class="from">{who(message.from)}</span>
                    {#if message.last_action && DONE[message.last_action]}
                        {@const done = DONE[message.last_action]}
                        <span class="done" role="img" title={done.said} aria-label={done.said}><done.shape size={14} aria-hidden="true" /></span>
                    {/if}
                    <span class="date">{when(message.received_at)}</span>
                    <span class="subject">
                        {#if message.has_attachments}<span class="clip" title="Has attachments" aria-label="Has attachments">📎</span>{/if}
                        {#if message.flag_status === "flagged"}<span class="clip" title="Flagged" aria-label="Flagged">🚩</span>{/if}
                        {#if message.flag_status === "complete"}<span class="clip" title="Complete" aria-label="Complete">✓</span>{/if}
                        {message.subject || "(No subject)"}
                        {#each message.categories ?? [] as name (name)}
                            <span class="dot" style:background={categoryColour(categories.find((c) => c.name === name)?.color)} title={name} aria-label="Category {name}"></span>
                        {/each}
                        {#if message.conversation && message.conversation.count > 1}
                            <span class="many" aria-label="{message.conversation.count} messages">{message.conversation.count}</span>
                        {/if}
                    </span>
                    <span class="preview">{message.preview}</span>
                {:else}
                    <span class="from muted">Loading…</span>
                {/if}
            </div>
        {/each}
    </div>
</div>

<style>
    .stretch,
    .starts {
        font-size: 0.7rem;
        font-weight: 600;
        letter-spacing: 0.04em;
        text-transform: uppercase;
        color: var(--muted);
    }

    .stretch {
        flex: none;
        margin: 0;
        padding: 0.2rem 0.75rem;
        border-bottom: 1px solid var(--line);
        background: var(--raised);
    }

    /* Sits on the line above the first message of a stretch. */
    .row .starts {
        position: absolute;
        top: -0.6rem;
        right: 0.75rem;
        padding: 0 0.4rem;
        background: var(--surface);
        border: 1px solid var(--line);
        border-radius: 0.6rem;
        z-index: 1;
        overflow: visible;
    }

    .viewport {
        flex: 1;
        min-height: 0;
        overflow-y: auto;
        contain: strict;
    }

    .spacer {
        position: relative;
    }

    .row {
        position: absolute;
        inset: 0 0 auto 0;
        height: 68px;
        box-sizing: border-box;
        padding: 0.5rem 0.75rem;
        display: grid;
        grid-template-columns: 2rem 1fr auto;
        grid-template-rows: auto auto auto;
        column-gap: 0.6rem;
        border-bottom: 1px solid var(--line);
        border-left: 3px solid transparent;
        cursor: default;
        font-size: 0.85rem;
        /* A click with Shift held chooses lines, not their words. */
        user-select: none;
        -webkit-user-select: none;
    }

    .row span {
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    }

    .row:hover {
        background: var(--raised);
    }

    .row.selected {
        background: color-mix(in srgb, var(--accent) 16%, var(--surface));
    }

    /* Among several chosen, the one the keys move from. */
    .row.cursor {
        box-shadow: inset 0 0 0 1px var(--accent);
    }

    .row.unread {
        border-left-color: var(--accent);
    }

    .row.unread .from,
    .row.unread .subject {
        font-weight: 700;
    }

    .date {
        color: var(--muted);
        font-size: 0.75rem;
    }

    .from {
        grid-area: 1 / 2;
    }

    /* Answered or sent on: at the end of the sender's line. */
    .done {
        grid-area: 1 / 3;
        justify-self: end;
        display: grid;
        place-items: center;
        color: var(--muted);
    }

    .subject {
        grid-area: 2 / 2;
    }

    .date {
        grid-area: 2 / 3;
    }

    .preview {
        grid-area: 3 / 2 / auto / -1;
    }

    /* The sender's circle. Under the mouse, or once several are chosen, a box to choose the line with. */
    .row .lead {
        grid-row: 1 / -1;
        grid-column: 1;
        align-self: center;
        display: grid;
        width: 2rem;
        height: 2rem;
        overflow: visible;
    }

    .circle,
    .box {
        grid-area: 1 / 1;
        display: grid;
        place-items: center;
    }

    .circle {
        border-radius: 50%;
        color: white;
        font-size: 0.75rem;
        font-weight: 600;
    }

    .box {
        place-self: center;
        width: 1.05rem;
        height: 1.05rem;
        border: 1px solid var(--muted);
        border-radius: 0.2rem;
        background: var(--surface);
        visibility: hidden;
    }

    .row:hover .box,
    .lead.ticked .box {
        visibility: visible;
    }

    .row:hover .circle,
    .lead.ticked .circle {
        visibility: hidden;
    }

    .lead.ticked .box {
        border-color: var(--accent-fill);
        background: var(--accent-fill);
        color: var(--on-accent);
    }

    /* What can be done to a line from the line, shown over the end of the sender's name while the mouse is on it. */
    .row .acts {
        position: absolute;
        top: 0.35rem;
        right: 0.5rem;
        display: none;
        gap: 0.1rem;
        overflow: visible;
    }

    .row:hover .acts {
        display: flex;
    }

    .row .act {
        display: grid;
        place-items: center;
        width: 1.5rem;
        height: 1.3rem;
        border-radius: 0.3rem;
        background: var(--raised);
        cursor: pointer;
    }

    .row.selected .act {
        background: color-mix(in srgb, var(--accent) 16%, var(--surface));
    }

    .row .act:hover {
        background: color-mix(in srgb, CanvasText 12%, var(--raised));
    }

    .preview {
        color: var(--muted);
    }

    .dot {
        display: inline-block;
        width: 0.6rem;
        height: 0.6rem;
        margin-left: 0.25rem;
        border-radius: 50%;
        vertical-align: baseline;
    }

    .many {
        margin-left: 0.25rem;
        padding: 0 0.35rem;
        border-radius: 0.6rem;
        background: var(--line);
        font-size: 0.7rem;
        font-weight: 600;
    }
</style>
