<script lang="ts">
    import { type Folder, TOP, canHold } from "./mail";

    interface Props {
        /** The folder to be moved. */
        folder: Folder;
        /** Every folder of its mailbox, in tree order. */
        folders: Folder[];
        /** Where it was dropped, to be agreed to. With none, where it goes is chosen here. */
        into?: Folder | null;
        /** The move is being made. */
        busy?: boolean;
        /** Why it could not be made. */
        problem?: string | null;
        /** Move it into the folder with this id, or to the top of the mailbox. */
        onMove: (destination: string) => void;
        onCancel: () => void;
    }

    let { folder, folders, into = null, busy = false, problem = null, onMove, onCancel }: Props = $props();

    /** The folder chosen to put it in, where one is chosen here. */
    let chosen = $state<string | null>(null);
    const atTop = $derived(!folders.some((each) => each.id === folder.parent_id));
    const destination = $derived(into?.id ?? chosen);

    function keydown(event: KeyboardEvent) {
        if (event.key === "Escape") {
            event.preventDefault();
            onCancel();
        }
    }

    /** The keys start in the dialog: on the first choice, or on Cancel where there is only a yes or no. */
    function first(node: HTMLElement) {
        node.querySelector<HTMLElement>("input:not(:disabled), button")?.focus();
    }
</script>

<div class="veil">
    <div class="ask" role="dialog" aria-modal="true" aria-labelledby="move-folder-title" tabindex="-1" onkeydown={keydown} use:first>
        <form onsubmit={(event) => (event.preventDefault(), destination && onMove(destination))}>
            {#if into}
                <h2 id="move-folder-title">Move {folder.name} into {into.name}?</h2>
                <p class="muted">Everything in {folder.name} goes with it.</p>
            {:else}
                <h2 id="move-folder-title">Move {folder.name} to</h2>
                <ul>
                    <li>
                        <label><input type="radio" name="destination" value={TOP} disabled={atTop} bind:group={chosen} />Top of the mailbox</label>
                    </li>
                    {#each folders as each (each.id)}
                        <li style:padding-left="{(each.depth + 1) * 0.9}rem">
                            <label><input type="radio" name="destination" value={each.id} disabled={!canHold(folders, folder, each)} bind:group={chosen} />{each.name}</label>
                        </li>
                    {/each}
                </ul>
            {/if}
            {#if problem}<p class="caution" role="alert">{problem}</p>{/if}

            <div class="buttons">
                <button type="button" onclick={onCancel}>Cancel</button>
                <button type="submit" class="primary" disabled={busy || !destination}>{busy ? "Moving…" : "Move"}</button>
            </div>
        </form>
    </div>
</div>

<style>
    .veil {
        position: fixed;
        inset: 0;
        z-index: 50;
        display: grid;
        place-items: center;
        background: color-mix(in srgb, CanvasText 35%, transparent);
    }

    .ask {
        width: min(26rem, calc(100vw - 2rem));
        max-height: calc(100vh - 2rem);
        display: flex;
        padding: 1.25rem 1.5rem;
        background: var(--surface);
        border: 1px solid var(--line);
        border-radius: 0.6rem;
        box-shadow: 0 12px 40px color-mix(in srgb, CanvasText 30%, transparent);
    }

    form {
        flex: 1;
        min-width: 0;
        display: flex;
        flex-direction: column;
    }

    h2 {
        margin: 0 0 0.75rem;
        font-size: 1.1rem;
        overflow-wrap: anywhere;
    }

    p {
        margin: 0 0 1rem;
    }

    ul {
        min-height: 4rem;
        overflow-y: auto;
        margin: 0 0 1rem;
        padding: 0.3rem;
        list-style: none;
        border: 1px solid var(--line);
        border-radius: 0.4rem;
    }

    label {
        display: flex;
        align-items: center;
        gap: 0.4rem;
        padding: 0.2rem 0.3rem;
    }

    label:has(input:disabled) {
        color: var(--muted);
    }

    .caution {
        color: var(--danger);
    }

    .buttons {
        display: flex;
        justify-content: flex-end;
        gap: 0.5rem;
    }
</style>
