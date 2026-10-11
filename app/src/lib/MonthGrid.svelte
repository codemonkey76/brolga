<script lang="ts">
    import { type Calendar, type CalendarEvent, calendarColour, onDay, sameDay, span, timeSaid } from "./calendar";

    interface Props {
        days: Date[];
        /** The month being shown: days outside it are shown faintly. */
        month: number;
        events: CalendarEvent[];
        calendars: Calendar[];
        now: Date;
        selectedId: string | null;
        onSelect: (event: CalendarEvent) => void;
        /** Show one day on its own. */
        onDay: (day: Date) => void;
    }

    let { days, month, events, calendars, now, selectedId, onSelect, onDay: showDay }: Props = $props();

    /** How many events a day shows before the rest are a number. */
    const SHOWN = 3;
    const names = $derived(days.slice(0, 7).map((day) => day.toLocaleDateString(undefined, { weekday: "short" })));
    // All-day events first, then by time.
    const on = (day: Date) =>
        events.filter((event) => onDay(event, day)).sort((a, b) => Number(b.is_all_day) - Number(a.is_all_day) || +span(a).start - +span(b).start);
</script>

<div class="month">
    <div class="names" aria-hidden="true">
        {#each names as name (name)}<span>{name}</span>{/each}
    </div>
    <div class="cells">
        {#each days as day (+day)}
            {@const here = on(day)}
            <div class="cell" class:outside={day.getMonth() !== month} class:today={sameDay(day, now)}>
                <button type="button" class="date" aria-label={day.toLocaleDateString(undefined, { weekday: "long", day: "numeric", month: "long" })} onclick={() => showDay(day)}>
                    {day.getDate()}
                </button>
                <ul>
                    {#each here.slice(0, SHOWN) as event (event.id)}
                        <li>
                            <button
                                type="button"
                                class="event"
                                class:selected={event.id === selectedId}
                                class:cancelled={event.is_cancelled}
                                class:block={event.is_all_day}
                                style:--colour={calendarColour(calendars, event.calendar_id)}
                                onclick={() => onSelect(event)}
                            >
                                {#if !event.is_all_day}<span class="time">{timeSaid(span(event).start)}</span>{/if}
                                {event.subject || "(No subject)"}
                            </button>
                        </li>
                    {/each}
                </ul>
                {#if here.length > SHOWN}
                    <button type="button" class="more" onclick={() => showDay(day)}>{here.length - SHOWN} more</button>
                {/if}
            </div>
        {/each}
    </div>
</div>

<style>
    .month {
        height: 100%;
        min-height: 0;
        display: flex;
        flex-direction: column;
    }

    .names,
    .cells {
        display: grid;
        grid-template-columns: repeat(7, minmax(0, 1fr));
    }

    .names {
        flex: none;
        border-bottom: 1px solid var(--line);
        font-size: 0.75rem;
        font-weight: 600;
        color: var(--muted);
    }

    .names span {
        padding: 0.3rem 0.5rem;
    }

    .cells {
        flex: 1;
        min-height: 0;
        grid-template-rows: repeat(6, minmax(0, 1fr));
    }

    .cell {
        min-width: 0;
        min-height: 0;
        overflow: hidden;
        padding: 0.15rem 0.25rem;
        border-right: 1px solid var(--line);
        border-bottom: 1px solid var(--line);
    }

    .cell.outside {
        background: var(--raised);
    }

    .cell.outside .date {
        color: var(--muted);
    }

    .date,
    .more {
        border: 0;
        background: none;
        padding: 0 0.25rem;
        font-size: 0.8rem;
    }

    .cell.today .date {
        background: var(--accent-fill);
        color: var(--on-accent);
        border-radius: 999px;
        font-weight: 700;
    }

    .more {
        color: var(--muted);
        font-size: 0.72rem;
    }

    ul {
        margin: 0.1rem 0 0;
        padding: 0;
        list-style: none;
        display: flex;
        flex-direction: column;
        gap: 1px;
    }

    .event {
        width: 100%;
        display: block;
        padding: 0 0.25rem;
        border: 0;
        border-left: 3px solid var(--colour);
        border-radius: 0.2rem;
        background: none;
        color: inherit;
        font-size: 0.75rem;
        line-height: 1.35;
        text-align: left;
        white-space: nowrap;
        overflow: hidden;
        text-overflow: ellipsis;
    }

    .event.block {
        background: color-mix(in srgb, var(--colour) 24%, var(--surface));
    }

    .event.selected {
        outline: 2px solid var(--colour);
    }

    .event.cancelled {
        text-decoration: line-through;
        color: color-mix(in srgb, CanvasText 78%, Canvas);
    }

    /* Read against the event's own colour, so a little darker than muted. */
    .time {
        color: color-mix(in srgb, CanvasText 78%, Canvas);
    }
</style>
