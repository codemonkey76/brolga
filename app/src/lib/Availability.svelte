<script lang="ts">
    import { type FreeBusy, freeBusy, timeSaid } from "./calendar";

    interface Props {
        accountId: string;
        /** Whose time to look at, by address: the owner first. */
        people: string[];
        /** The day looked at, `YYYY-MM-DD`, by this computer's calendar. */
        day: string;
        /** How long the meeting is, in minutes. */
        minutes: number;
        /** Take a time for the meeting. */
        onPick: (start: Date) => void;
    }

    let { accountId, people, day, minutes, onPick }: Props = $props();

    /** The working day shown: from eight until six, in half hours. */
    const FROM = 8;
    const TO = 18;
    const SLOT = 30;
    let found = $state<FreeBusy | null>(null);
    let problem = $state<string | null>(null);
    let looking = $state(false);

    const midnight = $derived(new Date(`${day}T00:00`));
    const STATUS = ["free", "tentative", "busy", "away"];

    async function look() {
        looking = true;
        problem = null;
        try {
            const start = midnight;
            const end = new Date(start.getFullYear(), start.getMonth(), start.getDate() + 1);
            found = await freeBusy(accountId, people, start, end, SLOT, Math.max(SLOT, minutes));
        } catch (failure) {
            found = null;
            problem = failure && typeof failure === "object" && "message" in failure ? String(failure.message) : String(failure);
        } finally {
            looking = false;
        }
    }

    // Asked again whenever who is asked about or the day changes.
    $effect(() => {
        void people.join(",");
        void day;
        void minutes;
        if (!Number.isNaN(+midnight) && people.length > 0) look();
    });

    /** The slots of the working day, each with where it is in the day's view. */
    const slots = $derived(Array.from({ length: ((TO - FROM) * 60) / SLOT }, (_, at) => (FROM * 60) / SLOT + at));
    /** Times in the working day when everyone is free for long enough. */
    const suggested = $derived(
        (found?.free_slots ?? [])
            .map((each) => ({ start: new Date(each.start), end: new Date(each.end) }))
            .filter((each) => each.start.getHours() >= FROM && (each.end.getHours() < TO || (each.end.getHours() === TO && each.end.getMinutes() === 0)) && each.start.getDate() === midnight.getDate())
            .slice(0, 6),
    );
</script>

<section class="availability" aria-label="Availability">
    {#if problem}
        <p class="problem" role="alert">{problem}</p>
    {:else if !found}
        <p class="muted">{looking ? "Looking…" : ""}</p>
    {:else}
        <table>
            <thead>
                <tr>
                    <th></th>
                    {#each slots as slot (slot)}
                        <th class:hour={slot % 2 === 0}>{slot % 2 === 0 ? ((slot / 2 + 11) % 12) + 1 : ""}</th>
                    {/each}
                </tr>
            </thead>
            <tbody>
                {#each found.people as person (person.address)}
                    <tr>
                        <th scope="row" title={person.error ?? person.address}>{person.address.split("@")[0]}</th>
                        {#if person.error}
                            <td colspan={slots.length} class="unknown">Not known</td>
                        {:else}
                            {#each slots as slot (slot)}
                                {@const status = STATUS[Number(person.availability[slot] ?? "0")] ?? "busy"}
                                <td class={status} title={status}></td>
                            {/each}
                        {/if}
                    </tr>
                {/each}
            </tbody>
        </table>
        {#if suggested.length > 0}
            <p class="suggest">
                <span>Everyone is free at</span>
                {#each suggested as each (+each.start)}
                    <button type="button" onclick={() => onPick(each.start)}>{timeSaid(each.start)}</button>
                {/each}
            </p>
        {:else}
            <p class="muted">No time between {FROM} and {TO - 12} that day suits everyone.</p>
        {/if}
    {/if}
</section>

<style>
    .availability {
        font-size: 0.8rem;
    }

    table {
        width: 100%;
        border-collapse: collapse;
        table-layout: fixed;
    }

    th[scope="row"] {
        width: 4.5rem;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
        text-align: left;
        font-weight: 400;
    }

    thead th {
        font-weight: 400;
        font-size: 0.7rem;
        text-align: left;
        color: color-mix(in srgb, CanvasText 60%, transparent);
    }

    thead th:first-child {
        width: 4.5rem;
    }

    td {
        height: 0.9rem;
        border: 1px solid var(--surface);
        background: color-mix(in srgb, #3a8a3a 25%, var(--surface));
    }

    td.tentative {
        background: repeating-linear-gradient(45deg, var(--accent), var(--accent) 2px, transparent 2px, transparent 4px);
    }

    td.busy {
        background: var(--accent);
    }

    td.away {
        background: #7b4bb7;
    }

    td.unknown {
        background: none;
        color: color-mix(in srgb, CanvasText 60%, transparent);
    }

    .suggest {
        display: flex;
        flex-wrap: wrap;
        align-items: center;
        gap: 0.3rem;
        margin: 0.4rem 0 0;
    }

    .suggest button {
        font-size: 0.8rem;
        padding: 0.15rem 0.4rem;
    }

    .problem {
        color: var(--danger);
    }

    p {
        margin: 0.3rem 0 0;
    }
</style>
