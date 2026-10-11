<script lang="ts">
    import { type Calendar, type CalendarEvent, type EventFields, type Repeat, type Scope, type Room, addDays, invitedFrom, invitedLine, listRooms, placeInMonth, span } from "./calendar";
    import Availability from "./Availability.svelte";
    import Recipients from "./Recipients.svelte";

    interface Props {
        accountId: string;
        /** The owner's own address, to count their time when a time is looked for. */
        me: string;
        /** The event being changed. None, and a new one is being made. */
        event: CalendarEvent | null;
        /** What the event says, where one is being changed and it is known. */
        description: string | null;
        /** When a new event is to start. */
        startAt: Date;
        calendars: Calendar[];
        /** For one of a series: how much of the series is being changed. */
        scope?: Scope;
        /** How the series repeats now, where the event is one of a series and it is known. */
        repeat?: Repeat | null;
        /** Make or change the event. Only what is to change is given. Throws if it could not be done. */
        onSave: (fields: EventFields, calendarId: string | null) => Promise<void>;
        onCancel: () => void;
    }

    let { accountId, me, event, description, startAt, calendars, scope = "occurrence", repeat = null, onSave, onCancel }: Props = $props();

    const pad = (n: number) => String(n).padStart(2, "0");
    const dayOf = (date: Date) => `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}`;
    const timeOf = (date: Date) => `${pad(date.getHours())}:${pad(date.getMinutes())}`;

    // What the form starts as. An all-day event ends, to a person, on its last day.
    // svelte-ignore state_referenced_locally
    const was = (() => {
        const at = event ? span(event) : { start: startAt, end: new Date(+startAt + 30 * 60_000) };
        const allDay = event?.is_all_day ?? false;
        const last = allDay ? addDays(at.end, -1) : at.end;
        return {
            subject: event?.subject ?? "",
            allDay,
            startDay: dayOf(at.start),
            startTime: allDay ? "09:00" : timeOf(at.start),
            endDay: dayOf(last),
            endTime: allDay ? "09:30" : timeOf(at.end),
            location: event?.location ?? "",
            showAs: event?.show_as ?? "busy",
            isPrivate: event?.sensitivity === "private",
            reminder: event ? (event.reminder_minutes === null ? "" : String(event.reminder_minutes)) : "15",
            description: description ?? "",
            required: invitedLine((event?.attendees ?? []).filter((each) => each.kind !== "optional" && each.kind !== "resource")),
            rooms: (event?.attendees ?? []).filter((each) => each.kind === "resource").map((each) => ({ address: each.address ?? "", name: each.name ?? each.address ?? "" })),
            optional: invitedLine((event?.attendees ?? []).filter((each) => each.kind === "optional")),
            unit: (repeat?.unit ?? "") as Repeat["unit"] | "",
            every: repeat?.every ?? 1,
            weekdays: [...(repeat?.days_of_week ?? [])].sort(),
            relative: Boolean(repeat?.week_of_month),
            ends: repeat?.until ? "until" : repeat?.count ? "count" : "never",
            until: repeat?.until ?? "",
            count: repeat?.count ?? 10,
        };
    })();


    let subject = $state(was.subject);
    let allDay = $state(was.allDay);
    let startDay = $state(was.startDay);
    let startTime = $state(was.startTime);
    let endDay = $state(was.endDay);
    let endTime = $state(was.endTime);
    let location = $state(was.location);
    let showAs = $state(was.showAs);
    let isPrivate = $state(was.isPrivate);
    let reminder = $state(was.reminder);
    let words = $state(was.description);
    let required = $state(was.required);
    let optional = $state(was.optional);
    let showOptional = $state(was.optional !== "");
    let online = $state(false);
    /** The rooms booked: each is invited as a resource, and answers for itself. */
    let booked = $state(was.rooms);
    /** The rooms there are, while one is being chosen. Null when none is. */
    let rooms = $state<Room[] | null>(null);
    let roomProblem = $state<string | null>(null);

    async function findRooms() {
        if (rooms) {
            rooms = null;
            return;
        }
        roomProblem = null;
        try {
            const [start, end] = allDay ? [new Date(`${startDay}T00:00`), new Date(`${endDay}T23:59`)] : [new Date(`${startDay}T${startTime}`), new Date(`${endDay}T${endTime}`)];
            rooms = await listRooms(accountId, start, end);
        } catch (failure) {
            roomProblem = failure && typeof failure === "object" && "message" in failure ? String(failure.message) : String(failure);
        }
    }

    function book(room: Room) {
        if (!booked.some((each) => each.address === room.address)) booked = [...booked, { address: room.address, name: room.name }];
        // The room is where it is, unless somewhere was already written.
        if (!location.trim()) location = room.name;
        rooms = null;
    }
    /** Whether everyone's free and busy times are shown. */
    let finding = $state(false);
    /** Those whose time is looked at: the owner, and everyone invited whose address reads as one. */
    const asked = $derived([
        ...new Set(
            [me, ...[...invitedFrom(required, "required"), ...invitedFrom(optional, "optional")].map((each) => each.address)]
                .map((each) => each.toLowerCase())
                .filter((each) => /^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(each)),
        ),
    ]);
    const length = $derived(Math.max(30, Math.round((+new Date(`${endDay}T${endTime}`) - +new Date(`${startDay}T${startTime}`)) / 60_000) || 30));
    // svelte-ignore state_referenced_locally
    let calendarId = $state(event?.calendar_id ?? calendars.find((c) => c.is_default && c.can_edit)?.id ?? calendars.find((c) => c.can_edit)?.id ?? null);
    // svelte-ignore state_referenced_locally
    const title = !event ? "New event" : scope === "series" ? "Change the whole series" : scope === "following" ? "Change this and following" : "Change event";
    let saving = $state(false);
    let problem = $state<string | null>(null);

    // One occurrence of a series does not repeat on its own; a series whose
    // way of repeating could not be read keeps the one it has.
    // svelte-ignore state_referenced_locally
    const canRepeat = !event?.series_id || (scope !== "occurrence" && repeat !== null);
    let unit = $state(was.unit);
    let every = $state(was.every);
    let weekdays = $state(was.weekdays);
    let relative = $state(was.relative);
    let ends = $state(was.ends);
    let until = $state(was.until);
    let count = $state(was.count);
    const WEEK = ["monday", "tuesday", "wednesday", "thursday", "friday", "saturday", "sunday"];
    const startDate = $derived(new Date(`${startDay}T00:00`));
    const place = $derived(placeInMonth(Number.isNaN(+startDate) ? new Date() : startDate));

    /** How it is to repeat, as asked for. Nothing if it is not to. */
    function repeatAsked(): Repeat | null {
        if (unit === "") return null;
        const asked: Repeat = { unit, every: Math.max(1, Math.round(every) || 1) };
        if (unit === "week" && weekdays.length > 0) asked.days_of_week = [...weekdays].sort();
        if ((unit === "month" || unit === "year") && relative) {
            asked.week_of_month = place.isLast && place.nth === "fourth" ? "last" : place.nth;
            asked.days_of_week = [place.weekday];
        }
        if (ends === "until" && until) asked.until = until;
        if (ends === "count") asked.count = Math.max(1, Math.round(count) || 1);
        return asked;
    }
    const repeatWas = JSON.stringify(repeatAsked());

    const REMINDERS = [
        ["", "None"],
        ["0", "At the start"],
        ["5", "5 minutes before"],
        ["15", "15 minutes before"],
        ["30", "30 minutes before"],
        ["60", "1 hour before"],
        ["120", "2 hours before"],
        ["1440", "1 day before"],
        ["10080", "1 week before"],
    ];

    // The end follows the start, keeping the event as long as it was.
    function startMoved(day: string, time: string) {
        const length = +new Date(`${endDay}T${endTime}`) - +new Date(`${startDay}T${startTime}`);
        startDay = day;
        startTime = time;
        const end = new Date(+new Date(`${day}T${time}`) + (Number.isFinite(length) ? Math.max(0, length) : 0));
        if (!Number.isNaN(+end)) {
            endDay = dayOf(end);
            endTime = timeOf(end);
        }
    }

    async function save(submitted: SubmitEvent) {
        submitted.preventDefault();
        if (!startDay || !endDay || (!allDay && (!startTime || !endTime))) {
            problem = "Say when it starts and ends.";
            return;
        }
        const fields: EventFields = {};
        if (!event || subject !== was.subject) fields.subject = subject;
        const moved = allDay !== was.allDay || startDay !== was.startDay || endDay !== was.endDay || (!allDay && (startTime !== was.startTime || endTime !== was.endTime));
        const asked = canRepeat ? repeatAsked() : null;
        const repeats = asked !== null && (!event || JSON.stringify(asked) !== repeatWas);
        if (event?.series_id && canRepeat && asked === null) {
            problem = "A series cannot be made into one event here. Delete the ones you do not want instead.";
            return;
        }
        if (repeats && asked) fields.repeat = asked;
        // How it repeats is said from its start, so the start goes with it.
        if (!event || moved || repeats) {
            fields.is_all_day = allDay;
            if (allDay) {
                const [year, month, date] = endDay.split("-").map(Number);
                fields.start = startDay;
                fields.end = dayOf(new Date(year, month - 1, date + 1));
            } else {
                fields.start = `${startDay}T${startTime}`;
                fields.end = `${endDay}T${endTime}`;
            }
            fields.time_zone = Intl.DateTimeFormat().resolvedOptions().timeZone;
        }
        if (location !== was.location) fields.location = location;
        if (showAs !== was.showAs || !event) fields.show_as = showAs;
        if (isPrivate !== was.isPrivate) fields.is_private = isPrivate;
        if (reminder !== was.reminder || !event) fields.reminder_minutes = reminder === "" ? null : Number(reminder);
        // The words are sent only if they were changed: what Microsoft 365
        // keeps in them, such as how to join online, is not to be lost.
        if (words !== was.description) fields.description = words;
        // Who is invited is given whole, and only if it was changed: then only those it changes are told.
        if (required !== was.required || optional !== was.optional || JSON.stringify(booked) !== JSON.stringify(was.rooms)) {
            fields.attendees = [
                ...invitedFrom(required, "required"),
                ...invitedFrom(optional, "optional"),
                ...booked.map((each) => ({ address: each.address, name: each.name, kind: "resource" as const })),
            ];
            const wrong = fields.attendees.find((each) => !/^[^\s@]+@[^\s@]+$/.test(each.address));
            if (wrong) {
                problem = `“${wrong.address}” is not an email address.`;
                return;
            }
        }
        if (online) fields.online_meeting = true;
        if (event && Object.keys(fields).length === 0) {
            onCancel();
            return;
        }

        saving = true;
        problem = null;
        try {
            await onSave(fields, event ? null : calendarId);
        } catch (failure) {
            problem = failure && typeof failure === "object" && "message" in failure ? String(failure.message) : String(failure);
        } finally {
            saving = false;
        }
    }
</script>

<form class="editor" aria-label={title} onsubmit={save}>
    <h2>{title}</h2>
    <!-- svelte-ignore a11y_autofocus -->
    <input type="text" aria-label="Subject" placeholder="Subject" maxlength="255" autofocus bind:value={subject} />

    {#if !event && calendars.filter((c) => c.can_edit).length > 1}
        <label>Calendar
            <select bind:value={calendarId}>
                {#each calendars.filter((c) => c.can_edit) as each (each.id)}
                    <option value={each.id}>{each.name}</option>
                {/each}
            </select>
        </label>
    {/if}

    <label class="check"><input type="checkbox" bind:checked={allDay} /> All day</label>
    <div class="when">
        <span>Starts</span>
        <input type="date" aria-label="Start day" required value={startDay} onchange={(e) => startMoved(e.currentTarget.value, startTime)} />
        {#if !allDay}<input type="time" aria-label="Start time" required value={startTime} onchange={(e) => startMoved(startDay, e.currentTarget.value)} />{/if}
        <span>Ends</span>
        <input type="date" aria-label="End day" required min={startDay} bind:value={endDay} />
        {#if !allDay}<input type="time" aria-label="End time" required bind:value={endTime} />{/if}
    </div>

    {#if canRepeat}
        <div class="repeat">
            <label>Repeats
                <select bind:value={unit}>
                    <option value="">Does not repeat</option>
                    <option value="day">Daily</option>
                    <option value="week">Weekly</option>
                    <option value="month">Monthly</option>
                    <option value="year">Yearly</option>
                </select>
            </label>
            {#if unit !== ""}
                <label>Every
                    <span class="inline"><input type="number" aria-label="Every so many" min="1" max="99" bind:value={every} /> {unit}{every === 1 ? "" : "s"}</span>
                </label>
            {/if}
        </div>
        {#if unit === "week"}
            <div class="weekdays" role="group" aria-label="On">
                {#each WEEK as day (day)}
                    <label class="check"><input type="checkbox" value={day} bind:group={weekdays} /> {day.slice(0, 1).toUpperCase() + day.slice(1, 3)}</label>
                {/each}
            </div>
        {/if}
        {#if unit === "month" || unit === "year"}
            <label>On
                <select aria-label="On which day" value={relative ? "weekday" : "date"} onchange={(e) => (relative = e.currentTarget.value === "weekday")}>
                    <option value="date">{unit === "month" ? `Day ${startDate.getDate()} of the month` : startDate.toLocaleDateString(undefined, { day: "numeric", month: "long" })}</option>
                    <option value="weekday">
                        The {place.isLast && place.nth === "fourth" ? "last" : place.nth} {place.weekday.slice(0, 1).toUpperCase() + place.weekday.slice(1)}{unit === "year"
                            ? ` of ${startDate.toLocaleDateString(undefined, { month: "long" })}`
                            : ""}
                    </option>
                </select>
            </label>
        {/if}
        {#if unit !== ""}
            <div class="repeat">
                <label>Ends
                    <select bind:value={ends}>
                        <option value="never">Never</option>
                        <option value="until">On a day</option>
                        <option value="count">After so many times</option>
                    </select>
                </label>
                {#if ends === "until"}<input type="date" aria-label="Last day" min={startDay} bind:value={until} />{/if}
                {#if ends === "count"}<span class="inline"><input type="number" aria-label="How many times" min="1" max="999" bind:value={count} /> times</span>{/if}
            </div>
        {/if}
    {/if}

    <!-- Not labels: a label would name the first control inside it, which is the button. -->
    <div class="people">
        <span>Invite {#if !showOptional}<button type="button" class="link" onclick={() => (showOptional = true)}>Optional</button>{/if}</span>
        <Recipients {accountId} label="Invite" bind:value={required} placeholder="name@example.com, another@example.com" />
    </div>
    {#if showOptional}
        <div class="people"><span>Optional</span><Recipients {accountId} label="Optional" bind:value={optional} /></div>
    {/if}
    {#if asked.length > 1 && !allDay}
        <button type="button" class="link find" aria-expanded={finding} onclick={() => (finding = !finding)}>{finding ? "Hide when people are free" : "See when people are free"}</button>
        {#if finding}
            <Availability {accountId} people={asked} day={startDay} minutes={length} onPick={(start) => startMoved(dayOf(start), timeOf(start))} />
        {/if}
    {/if}
    {#if event?.join_url}
        <p class="muted small">A Teams meeting. It stays one.</p>
    {:else}
        <label class="check"><input type="checkbox" bind:checked={online} /> Teams meeting</label>
    {/if}

    <div class="place">
        <input type="text" aria-label="Location" placeholder="Location" maxlength="500" bind:value={location} />
        <button type="button" aria-expanded={rooms !== null} onclick={findRooms}>{rooms ? "Close" : "Find a room"}</button>
    </div>
    {#if booked.length > 0}
        <ul class="booked" aria-label="Rooms booked">
            {#each booked as each (each.address)}
                <li>
                    {each.name}
                    <button type="button" class="link" aria-label={`Remove ${each.name}`} onclick={() => (booked = booked.filter((room) => room.address !== each.address))}>Remove</button>
                </li>
            {/each}
        </ul>
    {/if}
    {#if roomProblem}<p class="problem" role="alert">{roomProblem}</p>{/if}
    {#if rooms}
        <ul class="rooms" aria-label="Rooms">
            {#each rooms as room (room.address)}
                <li>
                    <button type="button" class="plain" onclick={() => book(room)}>
                        <span>{room.name}{room.building && room.building !== room.name ? ` · ${room.building}` : ""}</span>
                        <span class="muted">{room.capacity ? `${room.capacity} seats · ` : ""}{room.free === true ? "free" : room.free === false ? "busy" : "not known"}</span>
                    </button>
                </li>
            {:else}
                <li class="muted">Your organisation has no meeting rooms set up.</li>
            {/each}
        </ul>
    {/if}

    <div class="row">
        <label>Show as
            <select bind:value={showAs}>
                <option value="free">Free</option>
                <option value="tentative">Tentative</option>
                <option value="busy">Busy</option>
                <option value="oof">Out of office</option>
                <option value="workingElsewhere">Working elsewhere</option>
            </select>
        </label>
        <label>Reminder
            <select bind:value={reminder}>
                {#each REMINDERS as [value, label] (value)}<option {value}>{label}</option>{/each}
                {#if !REMINDERS.some(([value]) => value === reminder)}<option value={reminder}>{reminder} minutes before</option>{/if}
            </select>
        </label>
    </div>
    <label class="check"><input type="checkbox" bind:checked={isPrivate} /> Private</label>

    {#if event && description === null}
        <p class="muted small">What it says could not be fetched, so it cannot be changed just now.</p>
    {:else}
        <textarea aria-label="Description" placeholder="Description" rows="6" bind:value={words}></textarea>
    {/if}

    {#if problem}<p class="problem" role="alert">{problem}</p>{/if}
    <div class="buttons">
        <button type="submit" class="primary" disabled={saving}>{saving ? "Saving…" : "Save"}</button>
        <button type="button" disabled={saving} onclick={onCancel}>Cancel</button>
    </div>
</form>

<style>
    .editor {
        display: flex;
        flex-direction: column;
        gap: 0.6rem;
    }

    h2 {
        margin: 0;
        font-size: 1.05rem;
    }

    input[type="text"],
    textarea,
    select {
        font: inherit;
        max-width: 100%;
    }

    input[type="text"],
    textarea {
        width: 100%;
        box-sizing: border-box;
    }

    label {
        display: flex;
        flex-direction: column;
        gap: 0.15rem;
        font-size: 0.85rem;
    }

    label.check {
        flex-direction: row;
        align-items: center;
        gap: 0.4rem;
    }

    .when {
        display: grid;
        grid-template-columns: auto 1fr auto;
        align-items: center;
        gap: 0.35rem 0.5rem;
        font-size: 0.85rem;
    }

    .when span {
        grid-column: 1;
    }

    .people {
        display: flex;
        flex-direction: column;
        gap: 0.15rem;
        font-size: 0.85rem;
    }

    /* The field is the mail composer's, which draws no box of its own. */
    .people :global(input) {
        box-sizing: border-box;
        width: 100%;
        padding: 0.35rem 0.5rem;
        border: 1px solid var(--line);
        border-radius: 0.3rem;
        font: inherit;
    }

    .people span {
        display: flex;
        justify-content: space-between;
    }

    .place {
        display: flex;
        gap: 0.35rem;
    }

    .place button {
        flex: none;
        font-size: 0.85rem;
    }

    .booked,
    .rooms {
        margin: 0;
        padding: 0;
        list-style: none;
        font-size: 0.85rem;
    }

    .booked li {
        display: flex;
        justify-content: space-between;
    }

    .rooms {
        max-height: 11rem;
        overflow-y: auto;
        border: 1px solid var(--line);
        border-radius: 0.3rem;
    }

    .rooms .plain {
        width: 100%;
        display: flex;
        justify-content: space-between;
        gap: 0.75rem;
        padding: 0.3rem 0.5rem;
        border: 0;
        background: none;
        text-align: left;
        font-size: 0.85rem;
    }

    .rooms .plain:hover {
        background: var(--raised);
    }

    .find {
        align-self: flex-start;
    }

    .link {
        border: 0;
        background: none;
        padding: 0;
        color: var(--accent);
        font-size: 0.85rem;
    }

    .repeat {
        display: flex;
        flex-wrap: wrap;
        align-items: flex-end;
        gap: 0.75rem;
    }

    .inline {
        display: inline-flex;
        align-items: center;
        gap: 0.3rem;
    }

    input[type="number"] {
        width: 3.5rem;
        font: inherit;
    }

    .weekdays {
        display: flex;
        flex-wrap: wrap;
        gap: 0.2rem 0.6rem;
    }

    .row {
        display: flex;
        flex-wrap: wrap;
        gap: 0.75rem;
    }

    .small {
        font-size: 0.85rem;
    }

    .problem {
        margin: 0;
        color: var(--danger);
        font-size: 0.9rem;
    }

    .buttons {
        display: flex;
        gap: 0.5rem;
    }
</style>
