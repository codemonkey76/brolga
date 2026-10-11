<script lang="ts">
    import { type Calendar, type CalendarEvent, calendarColour, onDay, sameDay, span, timeSaid } from "./calendar";

    interface Props {
        days: Date[];
        events: CalendarEvent[];
        calendars: Calendar[];
        now: Date;
        selectedId: string | null;
        onSelect: (event: CalendarEvent) => void;
    }

    let { days, events, calendars, now, selectedId, onSelect }: Props = $props();

    const on = (day: Date) =>
        events.filter((event) => onDay(event, day)).sort((a, b) => Number(b.is_all_day) - Number(a.is_all_day) || +span(a).start - +span(b).start);
    /** Only the days that have something on them. */
    const busy = $derived(days.map((day) => ({ day, events: on(day) })).filter((each) => each.events.length > 0));
    const dayName = (day: Date) => day.toLocaleDateString(undefined, { weekday: "long", day: "numeric", month: "long" });
</script>

<div class="agenda">
    {#if busy.length === 0}
        <p class="muted empty">Nothing in these thirty days.</p>
    {/if}
    {#each busy as each (+each.day)}
        <section aria-label={dayName(each.day)}>
            <h3 class:today={sameDay(each.day, now)}>{sameDay(each.day, now) ? "Today · " : ""}{dayName(each.day)}</h3>
            <ul>
                {#each each.events as event (event.id)}
                    <li>
                        <button
                            type="button"
                            class="event"
                            class:selected={event.id === selectedId}
                            class:cancelled={event.is_cancelled}
                            style:--colour={calendarColour(calendars, event.calendar_id)}
                            onclick={() => onSelect(event)}
                        >
                            <span class="time">{event.is_all_day ? "All day" : `${timeSaid(span(event).start)} – ${timeSaid(span(event).end)}`}</span>
                            <span class="subject">{event.subject || "(No subject)"}</span>
                            <span class="muted">{event.location}</span>
                        </button>
                    </li>
                {/each}
            </ul>
        </section>
    {/each}
</div>

<style>
    .agenda {
        height: 100%;
        overflow-y: auto;
        padding: 0.5rem 1rem 2rem;
    }

    h3 {
        margin: 1rem 0 0.25rem;
        font-size: 0.85rem;
    }

    h3.today {
        color: var(--accent-text);
    }

    ul {
        margin: 0;
        padding: 0;
        list-style: none;
    }

    .event {
        width: 100%;
        display: grid;
        grid-template-columns: 10rem minmax(0, 1fr) minmax(0, 1fr);
        gap: 0.75rem;
        padding: 0.4rem 0.6rem;
        border: 0;
        border-left: 3px solid var(--colour);
        border-bottom: 1px solid var(--line);
        border-radius: 0;
        background: none;
        color: inherit;
        font-size: 0.9rem;
        text-align: left;
    }

    .event.selected {
        background: color-mix(in srgb, var(--colour) 16%, var(--surface));
    }

    .event.cancelled .subject {
        text-decoration: line-through;
    }

    .time {
        color: var(--muted);
    }

    .subject {
        font-weight: 600;
    }

    .event span {
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    }

    .empty {
        margin: 3rem auto;
        text-align: center;
    }
</style>
