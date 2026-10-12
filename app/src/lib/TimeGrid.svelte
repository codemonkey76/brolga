<script lang="ts">
    import { type Calendar, type CalendarEvent, calendarColour, layOut, onDay, sameDay, timeSaid, span } from "./calendar";

    interface Props {
        days: Date[];
        events: CalendarEvent[];
        calendars: Calendar[];
        now: Date;
        selectedId: string | null;
        onSelect: (event: CalendarEvent) => void;
        /** Whether an event may be dragged to another time. */
        movable?: (event: CalendarEvent) => boolean;
        /** An event was dragged to a new start and end. */
        onMove?: (event: CalendarEvent, start: Date, end: Date) => void;
        /** An empty time was double-clicked: a new event is wanted there. */
        onNew?: (start: Date) => void;
    }

    let { days, events, calendars, now, selectedId, onSelect, movable = () => false, onMove, onNew }: Props = $props();

    /** Dragging goes by the quarter hour. */
    const SNAP = 15;
    /** An event being dragged: where it would land if let go now. */
    let drag = $state<{ event: CalendarEvent; resize: boolean; day: number; from: number; to: number; moved: boolean } | null>(null);
    let held: { x: number; y: number; from: number; to: number; day: number } | null = null;
    /** True for the click that ends a drag, which is not a click on the event. */
    let dragged = false;
    let columns: HTMLDivElement[] = $state([]);

    function hold(pointer: PointerEvent, event: CalendarEvent, day: number, from: number, to: number, resize: boolean) {
        // A press is the start of something new, whatever the last drag left behind.
        dragged = false;
        // Only an event that lies within one day, and that may be changed.
        const { start, end } = span(event);
        if (pointer.button !== 0 || !onMove || !movable(event) || !sameDay(start, new Date(+end - 1))) return;
        held = { x: pointer.clientX, y: pointer.clientY, from, to, day };
        drag = { event, resize, day, from, to, moved: false };
        (pointer.currentTarget as HTMLElement).setPointerCapture(pointer.pointerId);
    }

    function pull(pointer: PointerEvent) {
        if (!drag || !held) return;
        const by = Math.round(((pointer.clientY - held.y) * (60 / HOUR)) / SNAP) * SNAP;
        if (!drag.moved && Math.abs(pointer.clientY - held.y) < 4 && Math.abs(pointer.clientX - held.x) < 4) return;
        if (drag.resize) {
            drag = { ...drag, moved: true, to: Math.min(24 * 60, Math.max(held.from + SNAP, held.to + by)) };
            return;
        }
        // Across to whichever day the pointer is over.
        const over = columns.findIndex((column) => {
            const box = column?.getBoundingClientRect();
            return box && pointer.clientX >= box.left && pointer.clientX < box.right;
        });
        const length = held.to - held.from;
        const from = Math.min(24 * 60 - length, Math.max(0, held.from + by));
        drag = { ...drag, moved: true, day: over >= 0 ? over : drag.day, from, to: from + length };
    }

    function drop() {
        const done = drag;
        drag = null;
        held = null;
        if (!done?.moved) return;
        // The click that ends this drag comes at once, if it comes: a drag by the foot ends on the foot, which is
        // not clicked. So it is forgotten straight after, and no later click is taken for it.
        dragged = true;
        setTimeout(() => (dragged = false));
        const day = days[done.day];
        const at = (minutes: number) => new Date(day.getFullYear(), day.getMonth(), day.getDate(), 0, minutes);
        const was = span(done.event);
        if (+at(done.from) !== +was.start || +at(done.to) !== +was.end) onMove?.(done.event, at(done.from), at(done.to));
    }

    function clicked(event: CalendarEvent) {
        if (dragged) {
            dragged = false;
            return;
        }
        onSelect(event);
    }

    function wanted(pointer: MouseEvent, day: Date) {
        if (!onNew || pointer.target !== pointer.currentTarget) return;
        const minutes = Math.floor((pointer.offsetY * (60 / HOUR)) / 30) * 30;
        onNew(new Date(day.getFullYear(), day.getMonth(), day.getDate(), 0, minutes));
    }

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
            {#each days as day, index (+day)}
                <div
                    class="day"
                    class:today={sameDay(day, now)}
                    role="list"
                    aria-label={day.toLocaleDateString(undefined, { weekday: "long", day: "numeric", month: "long" })}
                    bind:this={columns[index]}
                    ondblclick={(pointer) => wanted(pointer, day)}
                >
                    {#each hours as hour (hour)}
                        <span class="line" style:top="{hour * HOUR}px"></span>
                    {/each}
                    {#if sameDay(day, now)}
                        <span class="now" style:top="{nowAt}px" aria-hidden="true"></span>
                    {/if}
                    {#if drag?.moved && drag.day === index}
                        <div class="slot landing" aria-hidden="true" style:top="{drag.from * (HOUR / 60)}px" style:height="{(drag.to - drag.from) * (HOUR / 60)}px" style:left="1px" style:width="calc(100% - 3px)">
                            <span class="event" style:--colour={calendarColour(calendars, drag.event.calendar_id)}>
                                <span class="subject">{drag.event.subject || "(No subject)"}</span>
                                <span class="time">{timeSaid(new Date(2000, 0, 1, 0, drag.from))} to {timeSaid(new Date(2000, 0, 1, 0, drag.to))}</span>
                            </span>
                        </div>
                    {/if}
                    {#each layOut(events, day) as placed (placed.event.id)}
                        <div
                            class="slot"
                            class:lifted={drag?.moved && drag.event.id === placed.event.id}
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
                                onclick={() => clicked(placed.event)}
                                onpointerdown={(pointer) => hold(pointer, placed.event, index, placed.from, placed.to, false)}
                                onpointermove={pull}
                                onpointerup={drop}
                                onpointercancel={() => ((drag = null), (held = null))}
                            >
                                <span class="subject">{placed.event.subject || "(No subject)"}</span>
                                <span class="time">{timeSaid(span(placed.event).start)}{placed.event.location ? ` · ${placed.event.location}` : ""}</span>
                            </button>
                            {#if onMove && movable(placed.event) && placed.to - placed.from >= 30}
                                <!-- The foot of the event, to drag its end by; not on one too short to have a foot apart
                                     from its middle. The form does the same for the keyboard. -->
                                <span
                                    class="grip"
                                    aria-hidden="true"
                                    onpointerdown={(pointer) => hold(pointer, placed.event, index, placed.from, placed.to, true)}
                                    onpointermove={pull}
                                    onpointerup={drop}
                                    onpointercancel={() => ((drag = null), (held = null))}
                                ></span>
                            {/if}
                        </div>
                    {/each}
                </div>
            {/each}
        </div>
    </div>
</div>

<style>
    .slot.lifted {
        opacity: 0.35;
    }

    .slot.landing {
        z-index: 3;
        pointer-events: none;
    }

    .slot.landing .event {
        display: block;
        height: 100%;
        box-sizing: border-box;
        outline: 2px dashed var(--colour);
    }

    .grip {
        position: absolute;
        left: 0;
        right: 0;
        bottom: 0;
        height: 5px;
        cursor: ns-resize;
        touch-action: none;
    }

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
        color: var(--accent-text);
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
        color: color-mix(in srgb, CanvasText 78%, Canvas);
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

    /* Read against the event's own colour, so a little darker than muted. */
    .time {
        color: color-mix(in srgb, CanvasText 78%, Canvas);
        white-space: nowrap;
        overflow: hidden;
        text-overflow: ellipsis;
        max-width: 100%;
    }
</style>
