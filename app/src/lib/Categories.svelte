<script lang="ts">
    import { execute, say } from "./commands";
    import { type Category, categoryColour, listCategories } from "./mail";

    interface Props {
        accountId: string;
        username: string;
        /** The list changed, so whatever shows categories should read it again. */
        onChanged?: () => void;
    }

    /** The colours a category may have, named as Microsoft documents them, in the order of `preset0` to `preset24`. */
    const COLOURS = [
        "Red", "Orange", "Brown", "Yellow", "Green", "Teal", "Olive", "Blue", "Purple", "Cranberry", "Steel", "Dark steel", "Grey", "Dark grey", "Black",
        "Dark red", "Dark orange", "Dark brown", "Dark yellow", "Dark green", "Dark teal", "Dark olive", "Dark blue", "Dark purple", "Dark cranberry",
    ];

    let { accountId, username, onChanged }: Props = $props();
    let categories = $state<Category[] | null>(null);
    let problem = $state<string | null>(null);
    let name = $state("");
    let colour = $state("preset7");
    let deleting = $state<string | null>(null);

    $effect(() => {
        listCategories(accountId)
            .then((found) => (categories = found))
            .catch((failure) => (problem = say(failure)));
    });

    async function run(command: string, args: Record<string, unknown>) {
        problem = null;
        deleting = null;
        try {
            categories = (await execute<{ categories: Category[] }>(command, { account_id: accountId, ...args })).categories;
            onChanged?.();
            return true;
        } catch (failure) {
            problem = say(failure);
            return false;
        }
    }

    /** Give a category another colour. If it was refused, the list goes back to saying the one it has. */
    async function recolour(category: Category, list: HTMLSelectElement) {
        if (!(await run("mail.categories.set_color", { name: category.name, color: list.value }))) list.value = category.color;
    }

    async function add(submitted: SubmitEvent) {
        submitted.preventDefault();
        if (await run("mail.categories.create", { name: name.trim(), color: colour })) name = "";
    }
</script>

{#snippet colours()}
    <option value="none">No colour</option>
    {#each COLOURS as each, index (each)}
        <option value={`preset${index}`}>{each}</option>
    {/each}
{/snippet}

<section class="categories" aria-label="Categories for {username}">
    <p class="muted">The categories this mailbox has, kept by Microsoft 365 and shared with every program that reads it. A name cannot be changed once made.</p>
    {#if problem}
        <p class="problem" role="alert">{problem}</p>
    {/if}
    {#if categories}
        <ul>
            {#each categories as each (each.name)}
                <li>
                    <span class="dot" style:background={categoryColour(each.color)}></span>
                    <span class="name">{each.name}</span>
                    <select aria-label={`Colour of ${each.name}`} value={each.color} onchange={(event) => recolour(each, event.currentTarget)}>
                        {@render colours()}
                    </select>
                    {#if deleting === each.name}
                        <button type="button" class="danger" onclick={() => run("mail.categories.delete", { name: each.name })}>Delete it?</button>
                        <button type="button" onclick={() => (deleting = null)}>Keep</button>
                    {:else}
                        <button type="button" aria-label={`Delete ${each.name}`} onclick={() => (deleting = each.name)}>Delete</button>
                    {/if}
                </li>
            {/each}
        </ul>
        <form onsubmit={add}>
            <input type="text" aria-label="Name of a new category" placeholder="New category" maxlength="255" bind:value={name} />
            <select aria-label="Colour of the new category" bind:value={colour}>
                {@render colours()}
            </select>
            <button type="submit" class="primary" disabled={!name.trim()}>Add</button>
        </form>
    {:else if !problem}
        <p class="muted" role="status">Reading…</p>
    {/if}
</section>

<style>
    .categories {
        margin: 0.75rem 0 0 3rem;
        display: flex;
        flex-direction: column;
        gap: 0.5rem;
        max-width: 34rem;
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

    li,
    form {
        display: flex;
        align-items: center;
        gap: 0.5rem;
        padding: 0.15rem 0;
    }

    .name,
    input[type="text"] {
        flex: 1;
        min-width: 0;
    }

    input,
    select {
        font: inherit;
        padding: 0.3rem;
    }

    .dot {
        flex: none;
        width: 0.75rem;
        height: 0.75rem;
        border-radius: 50%;
    }

    .problem {
        padding: 0.5rem 0.75rem;
        background: color-mix(in srgb, var(--danger) 14%, var(--surface));
        font-size: 0.85rem;
    }
</style>
