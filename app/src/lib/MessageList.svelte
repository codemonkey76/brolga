<script lang="ts">
    import { type Category, type Message, categoryColour, dateGroup, when, who } from "./mail";

    interface Props {
        /** How many messages there are in all, loaded or not. */
        total: number;
        /** The messages by position. A gap is one not loaded yet. */
        rows: (Message | undefined)[];
        selectedId: string | null;
        onSelect: (message: Message) => void;
        /** Asked for when a position on screen has not been loaded. */
        onNeed: (index: number) => void;
        /** The mailbox's categories, for the colour of each. */
        categories: Category[];
        /** The list is in date order, so it can be marked off into today, yesterday and so on. */
        byDate: boolean;
    }

    let { total, rows, selectedId, onSelect, onNeed, byDate, categories }: Props = $props();

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

    function move(by: number) {
        const at = rows.findIndex((row) => row?.id === selectedId);
        const to = Math.min(total - 1, Math.max(0, at < 0 ? 0 : at + by));
        const row = rows[to];
        if (!row) return onNeed(to);
        onSelect(row);
        // Keep the selection on screen.
        if (viewport) {
            if (to * ROW < viewport.scrollTop) viewport.scrollTop = to * ROW;
            else if ((to + 1) * ROW > viewport.scrollTop + height) viewport.scrollTop = (to + 1) * ROW - height;
        }
    }

    function keydown(event: KeyboardEvent) {
        const moves: Record<string, number> = { ArrowDown: 1, ArrowUp: -1, PageDown: 8, PageUp: -8, Home: -total, End: total };
        const by = moves[event.key];
        if (by === undefined) return;
        event.preventDefault();
        move(by);
    }
</script>

{#if top}
    <p class="stretch">{top}</p>
{/if}
<div
    class="viewport"
    role="listbox"
    aria-label="Messages"
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
                class:selected={message?.id === selectedId}
                class:unread={message && (message.conversation ? message.conversation.unread > 0 : !message.is_read)}
                style:transform="translateY({index * ROW}px)"
                role="option"
                aria-selected={message?.id === selectedId}
                aria-setsize={total}
                aria-posinset={index + 1}
                tabindex="-1"
                onclick={() => message && onSelect(message)}
                onkeydown={() => {}}
            >
                {#if starts(index)}
                    <span class="starts">{group(index)}</span>
                {/if}
                {#if message}
                    <span class="from">{who(message.from)}</span>
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
        grid-template-columns: 1fr auto;
        grid-template-rows: auto auto auto;
        column-gap: 0.5rem;
        border-bottom: 1px solid var(--line);
        border-left: 3px solid transparent;
        cursor: default;
        font-size: 0.85rem;
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

    .subject,
    .preview {
        grid-column: 1 / -1;
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
