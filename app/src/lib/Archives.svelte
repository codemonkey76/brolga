<script lang="ts">
    import { type Archive, cancelArchive, importArchive, listArchives, removeArchive } from "./accounts";
    import { chooseFiles, say } from "./commands";

    interface Props {
        /** An archive was added, finished or removed, so what there is to read has changed. */
        onChanged?: () => void;
    }

    let { onChanged }: Props = $props();
    let archives = $state<Archive[]>([]);
    let problem = $state<string | null>(null);
    /** The archive about to be removed, until that is confirmed. */
    let removing = $state<string | null>(null);

    async function read() {
        const before = archives;
        try {
            archives = await listArchives();
        } catch (failure) {
            problem = say(failure);
            return;
        }
        // One that was importing and is no longer has more in it to read.
        if (before.some((was) => was.state === "importing" && archives.find((now) => now.id === was.id)?.state !== "importing")) onChanged?.();
    }

    read();

    // While a file is being read, how far it has got is asked every second.
    $effect(() => {
        if (!archives.some((each) => each.state === "importing")) return;
        const asking = setInterval(read, 1000);
        return () => clearInterval(asking);
    });

    async function add() {
        problem = null;
        for (const path of await chooseFiles()) {
            try {
                await importArchive(path);
            } catch (failure) {
                problem = say(failure);
            }
        }
        await read();
        onChanged?.();
    }

    async function stop(archive: Archive) {
        await cancelArchive(archive.id).catch((failure) => (problem = say(failure)));
        await read();
    }

    async function remove(archive: Archive) {
        removing = null;
        await removeArchive(archive.id).catch((failure) => (problem = say(failure)));
        await read();
        onChanged?.();
    }

    const count = (n: number) => `${n.toLocaleString()} message${n === 1 ? "" : "s"}`;

    function said(archive: Archive): string {
        const skipped = archive.skipped > 0 ? `, ${archive.skipped.toLocaleString()} could not be read` : "";
        if (archive.state === "importing") {
            return archive.messages_total ? `Reading: ${(archive.messages_done ?? 0).toLocaleString()} of ${count(archive.messages_total)}${skipped}` : "Reading the file…";
        }
        if (archive.state === "cancelled") return `Stopped: ${count(archive.messages)} were read${skipped}`;
        if (archive.state === "failed") return `Failed${archive.messages > 0 ? ` after ${count(archive.messages)}` : ""}: ${archive.error ?? "the file could not be read"}`;
        return `${count(archive.messages)}${skipped}`;
    }
</script>

<section class="card archives" aria-labelledby="archives-heading">
    <h2 id="archives-heading">Archives</h2>
    <p class="muted">Mail read in from an Outlook PST or OST file and kept on this computer only. The file is read, never changed, and nothing from it is sent to Microsoft 365. Each archive appears in Mail beside your accounts.</p>
    {#if problem}
        <p class="problem" role="alert">{problem}</p>
    {/if}
    {#if archives.length > 0}
        <ul aria-labelledby="archives-heading">
            {#each archives as each (each.id)}
                <li>
                    <span class="what">
                        <span class="name">{each.name}</span>
                        <span class="muted" role={each.state === "importing" ? "status" : undefined}>{said(each)}</span>
                        {#if each.state === "importing" && each.messages_total}
                            <progress max={each.messages_total} value={each.messages_done ?? 0} aria-label={`Reading ${each.name}`}></progress>
                        {/if}
                    </span>
                    {#if each.state === "importing"}
                        <button type="button" aria-label={`Stop reading ${each.name}`} onclick={() => stop(each)}>Stop</button>
                    {:else if removing === each.id}
                        <button type="button" class="danger" onclick={() => remove(each)}>Remove it?</button>
                        <button type="button" onclick={() => (removing = null)}>Keep</button>
                    {:else}
                        <button type="button" aria-label={`Remove ${each.name}`} onclick={() => (removing = each.id)}>Remove</button>
                    {/if}
                </li>
            {/each}
        </ul>
    {/if}
    <div><button type="button" onclick={add}>Import a PST file…</button></div>
</section>

<style>
    .archives {
        margin-top: 1rem;
        padding: 1rem;
        border: 1px solid var(--line);
        border-radius: 0.5rem;
        background: var(--raised);
        display: flex;
        flex-direction: column;
        gap: 0.6rem;
    }

    h2 {
        margin: 0;
        font-size: 1rem;
    }

    p {
        margin: 0;
        font-size: 0.85rem;
    }

    ul {
        margin: 0;
        padding: 0;
        list-style: none;
    }

    li {
        display: flex;
        align-items: center;
        gap: 0.5rem;
        padding: 0.4rem 0;
        border-top: 1px solid var(--line);
    }

    .what {
        flex: 1;
        min-width: 0;
        display: flex;
        flex-direction: column;
        gap: 0.15rem;
        font-size: 0.85rem;
    }

    .name {
        font-weight: 600;
    }

    progress {
        width: 100%;
        height: 0.4rem;
    }

    .problem {
        padding: 0.5rem 0.75rem;
        background: color-mix(in srgb, var(--danger) 14%, var(--surface));
    }
</style>
