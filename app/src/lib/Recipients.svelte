<script lang="ts">
    import { tick } from "svelte";
    import { type Person, suggestPeople } from "./mail";
    import { lastStarts, personWritten } from "./people-line";

    interface Props {
        accountId: string;
        label: string;
        /** The line as typed: `Sam Taylor <sam@example.test>, jo@example.test`. */
        value: string;
        placeholder?: string;
        input?: HTMLInputElement;
    }

    let { accountId, label, value = $bindable(), placeholder, input = $bindable() }: Props = $props();

    let offered = $state<Person[]>([]);
    let at = $state(0);
    let soon: ReturnType<typeof setTimeout> | undefined;
    /** Bumped with every lookup, so an answer for what is no longer typed is dropped. */
    let asking = 0;
    const listId = `people-${Math.random().toString(36).slice(2)}`;

    /** What is being typed now: the part after the last comma or semicolon that is not within a name. */
    const typing = (text: string) => text.slice(lastStarts(text)).trim();

    function typed() {
        clearTimeout(soon);
        const now = typing(value);
        // A whole address already, or nearly nothing: there is nothing to offer.
        if (now.length < 2 || now.includes(">")) {
            offered = [];
            return;
        }
        const mine = ++asking;
        soon = setTimeout(async () => {
            const people = await suggestPeople(accountId, now).catch(() => []);
            if (mine !== asking) return;
            offered = people;
            at = 0;
        }, 120);
    }

    async function choose(person: Person) {
        const before = value.slice(0, lastStarts(value)).trimEnd();
        value = `${before ? `${before} ` : ""}${personWritten(person)}, `;
        offered = [];
        asking += 1;
        input?.focus();
        // Once the box shows the new line, tell the form around this that
        // it changed, as typing would. Sooner, and the box's old contents
        // would be read back over it.
        await tick();
        input?.dispatchEvent(new Event("input", { bubbles: true }));
    }

    function keydown(event: KeyboardEvent) {
        if (offered.length === 0) return;
        if (event.key === "ArrowDown" || event.key === "ArrowUp") {
            event.preventDefault();
            at = (at + (event.key === "ArrowDown" ? 1 : offered.length - 1)) % offered.length;
        } else if (event.key === "Enter" || event.key === "Tab") {
            // Enter here takes the suggestion; it does not send the message.
            event.preventDefault();
            event.stopPropagation();
            choose(offered[at]);
        } else if (event.key === "Escape") {
            event.preventDefault();
            event.stopPropagation();
            offered = [];
        }
    }
</script>

<span class="line">
    <input
        bind:this={input}
        bind:value
        type="text"
        role="combobox"
        aria-label={label}
        aria-autocomplete="list"
        aria-expanded={offered.length > 0}
        aria-controls={listId}
        aria-activedescendant={offered.length > 0 ? `${listId}-${at}` : undefined}
        autocomplete="off"
        spellcheck="false"
        {placeholder}
        oninput={typed}
        onkeydown={keydown}
        onblur={() => setTimeout(() => (offered = []), 150)}
    />
    {#if offered.length > 0}
        <ul id={listId} role="listbox" aria-label="Suggested people">
            <!-- Not keyed by address: two people offered may share one. -->
            {#each offered as person, index}
                <!-- svelte-ignore a11y_click_events_have_key_events -->
                <li id="{listId}-{index}" role="option" aria-selected={index === at} class:at={index === at} onmousedown={(event) => event.preventDefault()} onclick={() => choose(person)}>
                    {#if person.name}<span class="name">{person.name}</span>{/if}
                    <span class="muted">{person.address}</span>
                </li>
            {/each}
        </ul>
    {/if}
</span>

<style>
    .line {
        flex: 1;
        min-width: 0;
        position: relative;
        display: flex;
    }

    input {
        flex: 1;
        min-width: 0;
        border: 0;
        background: none;
        padding: 0.35rem 0;
    }

    input:focus {
        outline: none;
    }

    ul {
        position: absolute;
        top: 100%;
        left: 0;
        z-index: 5;
        min-width: 18rem;
        max-width: 100%;
        margin: 0.15rem 0 0;
        padding: 0.25rem;
        list-style: none;
        background: var(--surface);
        border: 1px solid var(--line);
        border-radius: 0.4rem;
        box-shadow: 0 6px 18px color-mix(in srgb, CanvasText 18%, transparent);
    }

    li {
        display: flex;
        gap: 0.5rem;
        padding: 0.3rem 0.5rem;
        border-radius: 0.3rem;
        cursor: default;
        font-size: 0.9rem;
        white-space: nowrap;
        overflow: hidden;
        text-overflow: ellipsis;
    }

    li.at {
        background: color-mix(in srgb, var(--accent) 18%, var(--surface));
    }

    .name {
        font-weight: 600;
    }
</style>
