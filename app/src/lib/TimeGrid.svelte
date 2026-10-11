<script lang="ts">
    import { type Calendar, type CalendarEvent, calendarColour, layOut, onDay, sameDay, timeSaid, span } from "./calendar";

    interface Props {
        days: Date[];
        events: CalendarEvent[];
        calendars: Calendar[];
        now: Date;
        selectedId: string | null;
        onSelect: (event: CalendarEvent) => void;
    }

    let { days, events, calendars, now, selectedId, onSelect }: Props = $props();

    /** How tall an hour is, in pixels. */
    const HOUR = 48;
    const hours = Array.from({ length: 24 }, (_, hour) => hour);
    let grid = $state<HTMLDivElement>();

    // Open on the working day, not on midnight.
    $effect(() => {
        if (grid) grid.scrollTop = 7.5 * HOUR;
    });

    const hourSaid = (hour: number) => new Date(2000, 0, 1, hour).toLocaleTimeString(undefined, { hour: "numeric" });
    const dayHead = (day: Date) => day.toLocaleDateString(undefined, { weekday: "short", day: "numeric" });
    const allDay = (day: Date) => events.filter((event) => event.is_all_day && onDay(event, day));
    const nowAt = $derived((now.getHours() * 60 + now.getMinutes()) * (HOUR / 60));
</script>

<div class="timegrid" style:--days={days.length}>
    <div class="heads">
        <span class="gutter"></span>
        {#each days as day (+day)}
            <div class="head" class:today={sameDay(day, now)}>
                <span class="name">{dayHead(day)}</span>
                <ul class="allday" aria-label="All day, {dayHead(day)}">
                    {#each allDay(day) as event (event.id)}
                        <li>
                            <button
                                type="button"
                                class="event"
                                class:selected={event.id === selectedId}
                                class:cancelled={event.is_cancelled}
                                style:--colour={calendarColour(calendars, event.calendar_id)}
                                onclick={() => onSelect(event)}
                            >
                                {event.subject || "(No subject)"}
                            </button>
                        </li>
                    {/each}
                </ul>
            </div>
        {/each}
    </div>

    <div class="scroll" bind:this={grid}>
        <div class="hours" style:height="{24 * HOUR}px">
            <div class="gutter">
                {#each hours as hour (hour)}
                    <span style:top="{hour * HOUR}px">{hour === 0 ? "" : hourSaid(hour)}</span>
                {/each}
            </div>
            {#each days as day (+day)}
                <div class="day" class:today={sameDay(day, now)} role="list" aria-label={day.toLocaleDateString(undefined, { weekday: "long", day: "numeric", month: "long" })}>
                    {#each hours as hour (hour)}
                        <span class="line" style:top="{hour * HOUR}px"></span>
                    {/each}
                    {#if sameDay(day, now)}
                        <span class="now" style:top="{nowAt}px" aria-hidden="true"></span>
                    {/if}
                    {#each layOut(events, day) as placed (placed.event.id)}
                        <div
                            class="slot"
                            role="listitem"
                            style:top="{placed.from * (HOUR / 60)}px"
                            style:height="{(placed.to - placed.from) * (HOUR / 60)}px"
                            style:left="calc({(placed.column / placed.of) * 100}% + 1px)"
                            style:width="calc({100 / placed.of}% - 3px)"
                        >
                            <button
                                type="button"
                                class="event"
                                class:selected={placed.event.id === selectedId}
                                class:cancelled={placed.event.is_cancelled}
                                class:tentative={placed.event.show_as === "tentative"}
                                class:free={placed.event.show_as === "free"}
                                style:--colour={calendarColour(calendars, placed.event.calendar_id)}
                                onclick={() => onSelect(placed.event)}
                            >
                                <span class="subject">{placed.event.subject || "(No subject)"}</span>
                                <span class="time">{timeSaid(span(placed.event).start)}{placed.event.location ? ` · ${placed.event.location}` : ""}</span>
                            </button>
                        </div>
                    {/each}
                </div>
            {/each}
        </div>
    </div>
</div>

<style>
    .timegrid {
        height: 100%;
        min-height: 0;
        display: flex;
        flex-direction: column;
    }

    .heads,
    .hours {
        display: grid;
        grid-template-columns: 3.5rem repeat(var(--days), minmax(0, 1fr));
    }

    .heads {
        flex: none;
        border-bottom: 1px solid var(--line);
        /* Room for the scroll bar beneath, so the columns line up. */
        scrollbar-gutter: stable;
        overflow: hidden;
    }

    .head {
        min-width: 0;
        padding: 0.3rem 0.25rem;
        border-left: 1px solid var(--line);
    }

    .head .name {
        font-size: 0.8rem;
        font-weight: 600;
    }

    .head.today .name {
        color: var(--accent);
    }

    .allday {
        margin: 0.2rem 0 0;
        padding: 0;
        list-style: none;
        display: flex;
        flex-direction: column;
        gap: 2px;
    }

    .scroll {
        flex: 1;
        min-height: 0;
        overflow-y: auto;
        scrollbar-gutter: stable;
    }

    .hours {
        position: relative;
    }

    .gutter {
        position: relative;
    }

    .gutter span {
        position: absolute;
        right: 0.35rem;
        transform: translateY(-50%);
        font-size: 0.7rem;
        color: var(--muted);
    }

    .day {
        position: relative;
        min-width: 0;
        border-left: 1px solid var(--line);
    }

    .day.today {
        background: color-mix(in srgb, var(--accent) 5%, transparent);
    }

    .line {
        position: absolute;
        left: 0;
        right: 0;
        border-top: 1px solid color-mix(in srgb, var(--line) 60%, transparent);
    }

    .now {
        position: absolute;
        left: 0;
        right: 0;
        z-index: 2;
        border-top: 2px solid var(--danger);
    }

    .slot {
        position: absolute;
        z-index: 1;
    }

    .event {
        width: 100%;
        height: 100%;
        display: flex;
        flex-direction: column;
        align-items: flex-start;
        padding: 0.1rem 0.35rem;
        border: 0;
        border-left: 3px solid var(--colour);
        border-radius: 0.25rem;
        background: color-mix(in srgb, var(--colour) 22%, var(--surface));
        color: inherit;
        font-size: 0.78rem;
        line-height: 1.25;
        text-align: left;
        overflow: hidden;
    }

    .allday .event {
        height: auto;
        white-space: nowrap;
        text-overflow: ellipsis;
        display: block;
    }

    .event.selected {
        outline: 2px solid var(--colour);
    }

    .event.cancelled {
        text-decoration: line-through;
        opacity: 0.6;
    }

    .event.tentative {
        background: repeating-linear-gradient(135deg, color-mix(in srgb, var(--colour) 22%, var(--surface)) 0 6px, var(--surface) 6px 12px);
    }

    .event.free {
        background: var(--surface);
        outline: 1px solid color-mix(in srgb, var(--colour) 50%, transparent);
    }

    .subject {
        font-weight: 600;
        overflow: hidden;
        text-overflow: ellipsis;
        max-width: 100%;
    }

    .time {
        color: var(--muted);
        white-space: nowrap;
        overflow: hidden;
        text-overflow: ellipsis;
        max-width: 100%;
    }
</style>
