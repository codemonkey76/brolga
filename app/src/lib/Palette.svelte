<script lang="ts">
    import { tick } from "svelte";

    /** Something the window can do, with the keys that do it. */
    export interface Action {
        title: string;
        /** As a person would say the keys: "Ctrl+Shift+R". Empty if it has none. */
        keys: string;
        run: () => void;
    }

    interface Props {
        actions: Action[];
        onClose: () => void;
    }

    let { actions, onClose }: Props = $props();
    let typed = $state("");
    let at = $state(0);

    /** Every word typed is somewhere in the title, in any order. */
    const found = $derived.by(() => {
        const words = typed.toLowerCase().split(/\s+/).filter(Boolean);
        return actions.filter((each) => words.every((word) => each.title.toLowerCase().includes(word)));
    });
    $effect(() => {
        void typed;
        at = 0;
    });

    function choose(action: Action | undefined) {
        if (!action) return;
        onClose();
        // Once this is gone, so that what it does lands on what was behind it.
        tick().then(action.run);
    }

    function keydown(event: KeyboardEvent) {
        if (event.key === "Escape") onClose();
        else if (event.key === "ArrowDown") at = Math.min(at + 1, found.length - 1);
        else if (event.key === "ArrowUp") at = Math.max(at - 1, 0);
        else if (event.key === "Enter") choose(found[at]);
        else return;
        event.preventDefault();
        event.stopPropagation();
    }
</script>

<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
<div class="veil" onclick={onClose}>
    <div class="palette" role="dialog" aria-modal="true" aria-label="Commands" tabindex="-1" onclick={(event) => event.stopPropagation()} onkeydown={keydown}>
        <!-- svelte-ignore a11y_autofocus -->
        <input
            type="text"
            role="combobox"
            aria-label="Find a command"
            aria-expanded="true"
            aria-controls="palette-list"
            aria-activedescendant={found[at] ? `palette-${at}` : undefined}
            placeholder="Type what you want to do"
            autofocus
            bind:value={typed}
        />
        <ul id="palette-list" role="listbox" aria-label="Commands">
            {#each found as action, index (action.title)}
                <!-- svelte-ignore a11y_click_events_have_key_events -->
                <li id="palette-{index}" role="option" aria-selected={index === at} class:current={index === at} onclick={() => choose(action)} onmousemove={() => (at = index)}>
                    <span>{action.title}</span>
                    {#if action.keys}<kbd>{action.keys}</kbd>{/if}
                </li>
            {:else}
                <li class="none muted" role="option" aria-selected="false" aria-disabled="true">Nothing by that name.</li>
            {/each}
        </ul>
    </div>
</div>

<style>
    .veil {
        position: fixed;
        inset: 0;
        z-index: 50;
        display: flex;
        justify-content: center;
        align-items: flex-start;
        padding-top: 12vh;
        background: color-mix(in srgb, CanvasText 28%, transparent);
    }

    .palette {
        width: min(34rem, 92vw);
        max-height: 70vh;
        display: flex;
        flex-direction: column;
        border: 1px solid var(--line);
        border-radius: 0.6rem;
        background: var(--surface);
        box-shadow: 0 12px 40px color-mix(in srgb, black 30%, transparent);
        overflow: hidden;
    }

    input {
        padding: 0.75rem 0.9rem;
        border: 0;
        border-bottom: 1px solid var(--line);
        background: transparent;
        outline-offset: -2px;
    }

    ul {
        margin: 0;
        padding: 0.3rem;
        list-style: none;
        overflow-y: auto;
    }

    li {
        display: flex;
        justify-content: space-between;
        align-items: center;
        gap: 1rem;
        padding: 0.4rem 0.6rem;
        border-radius: 0.35rem;
        cursor: pointer;
    }

    li.current {
        background: color-mix(in srgb, var(--accent) 18%, transparent);
    }

    li.none {
        cursor: default;
    }

    kbd {
        font: inherit;
        font-size: 0.8rem;
        color: var(--muted);
        white-space: nowrap;
    }
</style>
