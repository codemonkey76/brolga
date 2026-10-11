<script lang="ts">
    import type { Account } from "./accounts";
    import Agenda from "./Agenda.svelte";
    import {
        type Calendar,
        type CalendarEvent,
        type View,
        addDays,
        calendarColour,
        daysOf,
        getEvent,
        listCalendars,
        listEvents,
        sameDay,
        showCalendar,
        startOfDay,
        startOfWeek,
        step,
        whenSaid,
    } from "./calendar";
    import { execute, say } from "./commands";
    import { openLink } from "./mail";
    import MonthGrid from "./MonthGrid.svelte";
    import TimeGrid from "./TimeGrid.svelte";

    interface Props {
        accounts: Account[];
    }

    let { accounts }: Props = $props();

    const VIEWS: { id: View; name: string }[] = [
        { id: "day", name: "Day" },
        { id: "work_week", name: "Work week" },
        { id: "week", name: "Week" },
        { id: "month", name: "Month" },
        { id: "agenda", name: "Agenda" },
    ];

    let accountId = $state<string | null>(null);
    let view = $state<View>("work_week");
    try {
        const kept = localStorage.getItem("brolga.calendar.view");
        if (VIEWS.some((each) => each.id === kept)) view = kept as View;
    } catch {
        // The working week, then.
    }
    /** A day in what is shown. */
    let anchor = $state(startOfDay(new Date()));
    let now = $state(new Date());
    let calendars = $state<Calendar[]>([]);
    let events = $state<CalendarEvent[]>([]);
    let problem = $state<string | null>(null);
    let syncing = $state(false);
    /** Bumped with each load, so an answer for what is no longer shown is dropped. */
    let loading = 0;

    let selected = $state<CalendarEvent | null>(null);
    let description = $state<{ text: string; complete: boolean } | null>(null);

    const days = $derived(daysOf(view, anchor));
    const title = $derived.by(() => {
        const month = (date: Date) => date.toLocaleDateString(undefined, { month: "long", year: "numeric" });
        if (view === "month") return month(anchor);
        if (view === "day") return anchor.toLocaleDateString(undefined, { weekday: "long", day: "numeric", month: "long", year: "numeric" });
        const [first, last] = [days[0], days[days.length - 1]];
        const short = (date: Date) => date.toLocaleDateString(undefined, { day: "numeric", month: "short" });
        return `${short(first)} – ${short(last)} ${last.getFullYear()}`;
    });

    async function load() {
        if (!accountId) return;
        const mine = ++loading;
        try {
            const [found, listed] = await Promise.all([listEvents(accountId, days[0], addDays(days[days.length - 1], 1)), listCalendars(accountId)]);
            if (mine !== loading) return;
            events = found;
            calendars = listed;
        } catch (failure) {
            if (mine === loading) problem = say(failure);
        }
    }

    async function sync() {
        if (syncing) return;
        syncing = true;
        problem = null;
        try {
            const results = (await execute<{ accounts: { account_id: string; ok: boolean; error: string | null; calendar_error?: string | null }[] }>("mail.sync")).accounts;
            const mine = results.find((r) => r.account_id === accountId);
            problem = mine?.calendar_error ?? (mine && !mine.ok ? mine.error : null);
            await load();
        } catch (failure) {
            problem = say(failure);
        } finally {
            syncing = false;
        }
    }

    // What is on this computer is shown at once, and then brought up to date.
    $effect(() => {
        const first = accounts[0];
        if (first && !accountId) {
            accountId = first.id;
            load().then(sync);
        }
    });

    // Whatever changes what is shown, the events for it are fetched.
    $effect(() => {
        void days;
        void accountId;
        load();
    });

    // The line for now moves, and every two minutes the calendar is synced.
    $effect(() => {
        const clock = setInterval(() => (now = new Date()), 30_000);
        const timer = setInterval(sync, 2 * 60_000);
        return () => {
            clearInterval(clock);
            clearInterval(timer);
        };
    });

    function show(next: View) {
        view = next;
        try {
            localStorage.setItem("brolga.calendar.view", next);
        } catch {
            // It holds for as long as the window is open.
        }
    }

    async function select(event: CalendarEvent) {
        selected = event;
        description = null;
        if (!accountId) return;
        try {
            const got = await getEvent(accountId, event.id);
            if (selected?.id === event.id) description = { text: got.description, complete: got.description_complete };
        } catch {
            // What the list knows of it is still shown.
        }
    }

    async function toggle(calendar: Calendar) {
        if (!accountId) return;
        try {
            await showCalendar(accountId, calendar.id, !calendar.shown);
            await load();
        } catch (failure) {
            problem = say(failure);
        }
    }

    function keydown(event: KeyboardEvent) {
        if (event.target instanceof HTMLInputElement || event.target instanceof HTMLSelectElement) return;
        if (event.key === "F9") {
            event.preventDefault();
            sync();
        } else if (event.key === "Escape" && selected) {
            selected = null;
        } else if (event.key.toLowerCase() === "t" && !event.ctrlKey && !event.metaKey) {
            anchor = startOfDay(new Date());
        }
    }

    // ---- the small month to steer by -----------------------------------
    let mini = $state(new Date(new Date().getFullYear(), new Date().getMonth(), 1));
    const miniDays = $derived(daysOf("month", mini));
    $effect(() => {
        // It follows what is being shown.
        mini = new Date(anchor.getFullYear(), anchor.getMonth(), 1);
    });

    const SAID: Record<string, string> = {
        accepted: "You accepted",
        tentativelyAccepted: "You accepted tentatively",
        declined: "You declined",
        notResponded: "You have not answered",
        organizer: "You are the organiser",
    };
    const ANSWERED: Record<string, string> = { accepted: "Accepted", tentativelyAccepted: "Tentative", declined: "Declined", none: "No answer", notResponded: "No answer", organizer: "Organiser" };
</script>

<svelte:window onkeydown={keydown} />

<div class="calendar">
    <aside aria-label="Calendars">
        <div class="mini" aria-label="Month to steer by">
            <div class="mini-head">
                <button type="button" aria-label="Previous month" onclick={() => (mini = new Date(mini.getFullYear(), mini.getMonth() - 1, 1))}>‹</button>
                <span>{mini.toLocaleDateString(undefined, { month: "long", year: "numeric" })}</span>
                <button type="button" aria-label="Next month" onclick={() => (mini = new Date(mini.getFullYear(), mini.getMonth() + 1, 1))}>›</button>
            </div>
            <div class="mini-grid">
                {#each miniDays.slice(0, 7) as day (+day)}<span class="muted">{day.toLocaleDateString(undefined, { weekday: "narrow" })}</span>{/each}
                {#each miniDays as day (+day)}
                    <button
                        type="button"
                        class:outside={day.getMonth() !== mini.getMonth()}
                        class:today={sameDay(day, now)}
                        class:shown={days.some((each) => sameDay(each, day))}
                        aria-label={day.toLocaleDateString(undefined, { day: "numeric", month: "long", year: "numeric" })}
                        onclick={() => (anchor = day)}
                    >
                        {day.getDate()}
                    </button>
                {/each}
            </div>
        </div>

        <h2 id="calendars-heading">Calendars</h2>
        <ul aria-labelledby="calendars-heading">
            {#each calendars as each (each.id)}
                <li>
                    <label>
                        <input type="checkbox" checked={each.shown} onchange={() => toggle(each)} />
                        <span class="dot" style:background={calendarColour(calendars, each.id)}></span>
                        <span class="name">{each.name}</span>
                    </label>
                </li>
            {/each}
        </ul>
    </aside>

    <section class="main" aria-label="Calendar">
        <div class="bar">
            <button type="button" title="Today (T)" onclick={() => (anchor = startOfDay(new Date()))}>Today</button>
            <button type="button" aria-label="Earlier" onclick={() => (anchor = step(view, anchor, -1))}>‹</button>
            <button type="button" aria-label="Later" onclick={() => (anchor = step(view, anchor, 1))}>›</button>
            <h1>{title}</h1>
            <span class="views" role="group" aria-label="View">
                {#each VIEWS as each (each.id)}
                    <button type="button" class:current={view === each.id} aria-pressed={view === each.id} onclick={() => show(each.id)}>{each.name}</button>
                {/each}
            </span>
            <button type="button" disabled={syncing} onclick={sync} title="Sync now (F9)">{syncing ? "Syncing…" : "Sync"}</button>
        </div>

        {#if problem}
            <p class="problem" role="alert">{problem}</p>
        {/if}

        <div class="body">
            {#if view === "month"}
                <MonthGrid
                    {days}
                    month={anchor.getMonth()}
                    {events}
                    {calendars}
                    {now}
                    selectedId={selected?.id ?? null}
                    onSelect={select}
                    onDay={(day) => {
                        anchor = day;
                        show("day");
                    }}
                />
            {:else if view === "agenda"}
                <Agenda {days} {events} {calendars} {now} selectedId={selected?.id ?? null} onSelect={select} />
            {:else}
                <TimeGrid {days} {events} {calendars} {now} selectedId={selected?.id ?? null} onSelect={select} />
            {/if}
        </div>
    </section>

    {#if selected}
        <aside class="details" aria-label="Event">
            <button type="button" class="close" aria-label="Close" title="Escape" onclick={() => (selected = null)}>✕</button>
            <h2 class:cancelled={selected.is_cancelled}>{selected.subject || "(No subject)"}</h2>
            {#if selected.is_cancelled}<p class="warn">Cancelled</p>{/if}
            <p>{whenSaid(selected)}</p>
            {#if !selected.is_all_day && selected.time_zone && selected.time_zone !== Intl.DateTimeFormat().resolvedOptions().timeZone}
                <p class="muted small">Made in {selected.time_zone}; shown in your time.</p>
            {/if}
            {#if selected.location}<p>📍 {selected.location}</p>{/if}
            {#if selected.join_url}
                {@const url = selected.join_url}
                <p><button type="button" class="primary" onclick={() => openLink(url).catch((failure) => (problem = say(failure)))}>Join online</button></p>
            {/if}
            <p class="muted small">
                <span class="dot" style:background={calendarColour(calendars, selected.calendar_id)}></span>
                {calendars.find((c) => c.id === selected?.calendar_id)?.name ?? "Calendar"}
                · Shows as {selected.show_as === "oof" ? "out of office" : selected.show_as === "workingElsewhere" ? "working elsewhere" : selected.show_as}
                {#if selected.reminder_minutes !== null}· Reminder {selected.reminder_minutes} min before{/if}
            </p>
            {#if selected.organizer.name || selected.organizer.address}
                <p class="small">Organised by {selected.organizer.name || selected.organizer.address}{SAID[selected.response] && !selected.is_organizer ? ` · ${SAID[selected.response]}` : ""}</p>
            {/if}
            {#if selected.attendees.length > 0}
                <h3>People</h3>
                <ul class="people">
                    {#each selected.attendees as each (each.address)}
                        <li>{each.name || each.address} <span class="muted">{ANSWERED[each.response] ?? each.response}{each.kind === "optional" ? " · optional" : ""}</span></li>
                    {/each}
                </ul>
            {/if}
            <h3>Description</h3>
            <!-- Plain words from the core: put on the page as text, and nothing else. -->
            <p class="description">{description ? description.text || "None." : selected.preview || "…"}</p>
            {#if description && !description.complete}
                <p class="muted small">Only the first words are on this computer. The rest shows when Microsoft 365 can be reached.</p>
            {/if}
        </aside>
    {/if}
</div>

<style>
    .calendar {
        height: 100%;
        min-height: 0;
        display: flex;
    }

    aside {
        flex: none;
        width: 14rem;
        padding: 0.75rem;
        overflow-y: auto;
        background: var(--raised);
        border-right: 1px solid var(--line);
    }

    aside.details {
        position: relative;
        width: 20rem;
        border-right: 0;
        border-left: 1px solid var(--line);
        background: var(--surface);
    }

    .main {
        flex: 1;
        min-width: 0;
        display: flex;
        flex-direction: column;
    }

    .bar {
        flex: none;
        display: flex;
        align-items: center;
        gap: 0.4rem;
        padding: 0.5rem 0.75rem;
        border-bottom: 1px solid var(--line);
    }

    .bar h1 {
        flex: 1;
        margin: 0 0.5rem;
        font-size: 1.05rem;
        white-space: nowrap;
        overflow: hidden;
        text-overflow: ellipsis;
    }

    .views {
        display: flex;
    }

    .views button {
        border-radius: 0;
        margin-left: -1px;
        font-size: 0.85rem;
    }

    .views button.current {
        background: color-mix(in srgb, var(--accent) 18%, var(--surface));
        font-weight: 600;
    }

    .body {
        flex: 1;
        min-height: 0;
    }

    .problem {
        flex: none;
        margin: 0;
        padding: 0.5rem 0.75rem;
        background: color-mix(in srgb, var(--danger) 14%, var(--surface));
        font-size: 0.85rem;
    }

    h2 {
        margin: 1rem 0 0.4rem;
        font-size: 0.7rem;
        font-weight: 600;
        letter-spacing: 0.04em;
        text-transform: uppercase;
        color: var(--muted);
    }

    .details h2 {
        margin: 0 2rem 0.5rem 0;
        font-size: 1.1rem;
        letter-spacing: 0;
        text-transform: none;
        color: inherit;
    }

    .details h2.cancelled {
        text-decoration: line-through;
    }

    .details h3 {
        margin: 1rem 0 0.25rem;
        font-size: 0.8rem;
        color: var(--muted);
    }

    .details p {
        margin: 0 0 0.5rem;
        overflow-wrap: anywhere;
    }

    .small {
        font-size: 0.8rem;
    }

    .warn {
        color: var(--danger);
        font-weight: 600;
    }

    .description {
        white-space: pre-wrap;
        font-size: 0.9rem;
    }

    .close {
        position: absolute;
        top: 0.5rem;
        right: 0.5rem;
        border: 0;
        background: none;
    }

    ul {
        margin: 0;
        padding: 0;
        list-style: none;
    }

    aside label {
        display: flex;
        align-items: center;
        gap: 0.4rem;
        padding: 0.2rem 0;
        font-size: 0.9rem;
    }

    .name {
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    }

    .dot {
        flex: none;
        display: inline-block;
        width: 0.7rem;
        height: 0.7rem;
        border-radius: 50%;
    }

    .people li {
        padding: 0.1rem 0;
        font-size: 0.85rem;
    }

    .mini-head {
        display: flex;
        align-items: center;
        justify-content: space-between;
        font-size: 0.85rem;
        font-weight: 600;
    }

    .mini-head button,
    .mini-grid button {
        border: 0;
        background: none;
        padding: 0.1rem 0.3rem;
    }

    .mini-grid {
        display: grid;
        grid-template-columns: repeat(7, 1fr);
        text-align: center;
        font-size: 0.72rem;
    }

    .mini-grid button {
        font-size: 0.75rem;
        border-radius: 999px;
    }

    .mini-grid button.outside {
        color: var(--muted);
    }

    .mini-grid button.shown {
        background: color-mix(in srgb, var(--accent) 16%, transparent);
    }

    .mini-grid button.today {
        background: var(--accent);
        color: Canvas;
        font-weight: 700;
    }
</style>
