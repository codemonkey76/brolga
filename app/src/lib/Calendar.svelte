<script lang="ts">
    import type { Account } from "./accounts";
    import Agenda from "./Agenda.svelte";
    import {
        type Answer,
        type Calendar,
        type EventFields,
        type Repeat,
        type Scope,
        repeatSaid,
        cancelEvent,
        createEvent,
        deleteEvent,
        updateEvent,
        type CalendarEvent,
        type View,
        addDays,
        calendarColour,
        daysOf,
        getEvent,
        respond,
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
    import EventEditor from "./EventEditor.svelte";
    import Rsvp from "./Rsvp.svelte";
    import { Pencil, Plus, Trash } from "@lucide/svelte";
    import TimeGrid from "./TimeGrid.svelte";

    interface Props {
        accounts: Account[];
        /** A day to open on, when the calendar is come to for one. */
        startOn?: Date | null;
    }

    let { accounts, startOn = null }: Props = $props();

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
    // svelte-ignore state_referenced_locally
    let anchor = $state(startOfDay(startOn ?? new Date()));
    let now = $state(new Date());
    let calendars = $state<Calendar[]>([]);
    let events = $state<CalendarEvent[]>([]);
    let problem = $state<string | null>(null);
    let syncing = $state(false);
    /** Bumped with each load, so an answer for what is no longer shown is dropped. */
    let loading = 0;

    let selected = $state<CalendarEvent | null>(null);
    let description = $state<{ text: string; complete: boolean } | null>(null);
    /** How the selected event's series repeats, where it is of one and it is known. */
    let repeat = $state<Repeat | null>(null);
    /** How much of a series is being changed. */
    let scope = $state<Scope>("occurrence");
    /** What is being asked of a series: how much of it to change, or to delete. */
    let asking = $state<"change" | "delete" | null>(null);
    /** An event is being made (`new`) or the selected one changed (`change`). */
    let editing = $state<"new" | "change" | null>(null);
    /** True once Delete has been pressed once, and it is being asked whether to. */
    let sure = $state(false);

    /** When a new event starts, unless said otherwise: the next half hour, on the day shown if it is not today. */
    function newStart(): Date {
        const at = new Date();
        const day = days.some((each) => sameDay(each, at)) ? at : (view === "month" ? anchor : days[0]);
        const start = new Date(day.getFullYear(), day.getMonth(), day.getDate(), sameDay(day, at) ? at.getHours() : 9, sameDay(day, at) && at.getMinutes() >= 30 ? 60 : sameDay(day, at) ? 30 : 0);
        return start;
    }

    async function save(fields: EventFields, calendarId: string | null) {
        if (!accountId) return;
        const saved = editing === "change" && selected ? await updateEvent(accountId, selected.id, fields, scope) : await createEvent(accountId, calendarId, fields);
        editing = null;
        await load();
        // A series is in the calendar as its occurrences, not as itself.
        if (saved.kind === "seriesMaster") selected = null;
        else await select(saved);
    }

    /** The words to go with a cancellation, while a meeting is being cancelled. Null when none is. */
    let cancelling = $state<string | null>(null);
    /** A meeting the owner called, with people invited: deleting it is cancelling it for them. */
    const mine = $derived(Boolean(selected?.is_organizer && selected.attendees.length > 0));

    async function cancel(of: Scope) {
        const event = selected;
        if (!accountId || !event || cancelling === null) return;
        try {
            await cancelEvent(accountId, event.id, cancelling.trim(), of);
            selected = null;
            cancelling = null;
            await load();
        } catch (failure) {
            problem = say(failure);
        }
    }

    async function remove(of: Scope = "occurrence") {
        const event = selected;
        if (!accountId || !event) return;
        try {
            await deleteEvent(accountId, event.id, of);
            asking = null;
            selected = null;
            sure = false;
            await load();
        } catch (failure) {
            problem = say(failure);
        }
    }

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
        editing = null;
        sure = false;
        asking = null;
        cancelling = null;
        scope = "occurrence";
        description = null;
        repeat = null;
        const id = accountId;
        if (!id) return;
        fetching = (async () => {
            try {
                const got = await getEvent(id, event.id);
                if (selected?.id === event.id) {
                    description = { text: got.description, complete: got.description_complete };
                    repeat = got.repeat ?? null;
                }
            } catch {
                // What the list knows of it is still shown.
            }
        })();
        await fetching;
    }

    /** The rest of the selected event being fetched: what it says, and how it repeats. */
    let fetching: Promise<void> = Promise.resolve();

    /** Open the selected event to be changed, once all of it is here: the form starts from what it is. */
    async function change(of: Scope) {
        const event = selected;
        await fetching;
        if (selected?.id !== event?.id) return;
        scope = of;
        asking = null;
        editing = "change";
    }

    /** Answer the invitation that is open. A failure is for the buttons to say. */
    async function answer(given: Answer, comment: string, tell: boolean) {
        const event = selected;
        if (!accountId || !event) return;
        const result = await respond(accountId, event.id, given, comment, tell);
        // A declined meeting leaves the calendar.
        if (selected?.id === event.id) selected = given === "decline" ? null : { ...event, response: result.response };
        await load();
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
        if (event.key === "Escape" && editing) {
            editing = null;
            return;
        }
        if (event.target instanceof HTMLInputElement || event.target instanceof HTMLSelectElement || event.target instanceof HTMLTextAreaElement) return;
        if (event.key === "F9") {
            event.preventDefault();
            sync();
        } else if (event.key.toLowerCase() === "n" && !event.ctrlKey && !event.metaKey && !editing) {
            event.preventDefault();
            selected = null;
            editing = "new";
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
            <button type="button" class="primary new" title="New event (N)" onclick={() => ((selected = null), (editing = "new"))}><Plus size={16} aria-hidden="true" />New event</button>
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

    {#if editing}
        <aside class="details" aria-label="Event">
            {#key editing + scope + (selected?.id ?? "")}
                <EventEditor
                    accountId={accountId ?? ""}
                    me={accounts.find((each) => each.id === accountId)?.username ?? ""}
                    event={editing === "change" ? selected : null}
                    description={editing === "change" ? (description?.complete ? description.text : null) : ""}
                    startAt={newStart()}
                    {calendars}
                    scope={editing === "change" ? scope : "occurrence"}
                    repeat={editing === "change" ? repeat : null}
                    onSave={save}
                    onCancel={() => (editing = null)}
                />
            {/key}
        </aside>
    {:else if selected}
        <aside class="details" aria-label="Event">
            <button type="button" class="close" aria-label="Close" title="Escape" onclick={() => (selected = null)}>✕</button>
            {#if calendars.find((c) => c.id === selected?.calendar_id)?.can_edit ?? true}
                <div class="tools">
                    {#if cancelling !== null}
                        <!-- A meeting one called: those invited are told, with a word if there is one. -->
                        <div class="cancel" role="group" aria-label="Cancel meeting">
                            <textarea aria-label="Message to everyone invited" placeholder="A message to everyone invited (optional)" rows="2" maxlength="2000" bind:value={cancelling}></textarea>
                            <button type="button" class="danger" onclick={() => cancel("occurrence")}>{selected.series_id ? "Cancel this one" : "Cancel the meeting"}</button>
                            {#if selected.series_id}<button type="button" class="danger" onclick={() => cancel("series")}>Cancel the whole series</button>{/if}
                            <button type="button" onclick={() => (cancelling = null)}>Keep</button>
                        </div>
                    {:else if asking}
                        <!-- One of a series: how much of it? -->
                        <span class="ask">{asking === "delete" ? "Delete" : "Change"}</span>
                        {#each [["occurrence", "This event"], ["following", "This and following"], ["series", "The whole series"]] as [each, label] (each)}
                            <button
                                type="button"
                                class:danger={asking === "delete"}
                                onclick={() => {
                                    if (asking === "delete") remove(each as Scope);
                                    else change(each as Scope);
                                }}>{label}</button
                            >
                        {/each}
                        <button type="button" onclick={() => (asking = null)}>Cancel</button>
                    {:else if sure}
                        <button type="button" class="danger" onclick={() => remove()}>
                            {selected.is_organizer && selected.attendees.length > 0 ? "Delete, and tell everyone it is cancelled?" : "Delete for good?"}
                        </button>
                        <button type="button" onclick={() => (sure = false)}>Keep</button>
                    {:else}
                        {#if selected.is_organizer || selected.response === "none"}
                            <button type="button" aria-label="Change" title="Change" onclick={() => (selected?.series_id ? (asking = "change") : change("occurrence"))}
                                ><Pencil size={16} aria-hidden="true" /></button
                            >
                        {/if}
                        <button
                            type="button"
                            aria-label={mine ? "Cancel meeting" : "Delete"}
                            title={mine ? "Cancel the meeting" : "Delete"}
                            onclick={() => (mine ? (cancelling = "") : selected?.series_id ? (asking = "delete") : (sure = true))}
                            ><Trash size={16} aria-hidden="true" /></button
                        >
                    {/if}
                </div>
            {/if}
            <h2 class:cancelled={selected.is_cancelled}>{selected.subject || "(No subject)"}</h2>
            {#if selected.is_cancelled}<p class="warn">Cancelled</p>{/if}
            <p>{whenSaid(selected)}</p>
            {#if selected.series_id}<p class="muted small">↻ {repeat ? repeatSaid(repeat) : "Repeats"}</p>{/if}
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
            {#if !selected.is_organizer && !selected.is_cancelled && ["notResponded", "accepted", "tentativelyAccepted", "declined"].includes(selected.response)}
                <Rsvp response={selected.response} onAnswer={answer} />
            {/if}
            {#if selected.attendees.length > 0}
                {@const answers = selected.attendees}
                <h3>People</h3>
                {#if selected.is_organizer}
                    <p class="muted small">
                        {answers.filter((a) => a.response === "accepted").length} accepted · {answers.filter((a) => a.response === "tentativelyAccepted").length} tentative · {answers.filter(
                            (a) => a.response === "declined",
                        ).length} declined · {answers.filter((a) => !["accepted", "tentativelyAccepted", "declined"].includes(a.response)).length} no answer
                    </p>
                {/if}
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
    .bar .new {
        display: inline-flex;
        align-items: center;
        gap: 0.3rem;
        white-space: nowrap;
        font-size: 0.9rem;
    }

    .tools {
        display: flex;
        flex-wrap: wrap;
        gap: 0.35rem;
        margin-bottom: 0.4rem;
    }

    .tools button {
        display: inline-flex;
        align-items: center;
        font-size: 0.85rem;
    }

    .cancel {
        display: flex;
        flex-wrap: wrap;
        gap: 0.35rem;
        width: 100%;
    }

    .cancel textarea {
        width: 100%;
        box-sizing: border-box;
        font: inherit;
    }

    .tools .ask {
        align-self: center;
        font-size: 0.85rem;
        font-weight: 600;
    }

    .tools .danger {
        color: var(--danger);
    }

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
        /* Room to scroll the foot of a form clear of a reminder shown over it. */
        padding-bottom: 5rem;
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
