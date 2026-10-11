import { execute } from "./commands";

export interface Calendar {
    id: string;
    name: string;
    color: string;
    hex_color: string;
    is_default: boolean;
    can_edit: boolean;
    shown: boolean;
}

export interface CalendarEvent {
    id: string;
    calendar_id: string;
    subject: string;
    preview: string;
    location: string;
    /** UTC. For an all-day event only the date part means anything. */
    start: string;
    end: string;
    is_all_day: boolean;
    /** The zone the event was made in. */
    time_zone: string;
    show_as: string;
    sensitivity?: string;
    is_cancelled: boolean;
    is_organizer: boolean;
    organizer: { name: string | null; address: string | null };
    response: string;
    kind: string;
    /** The series it is one of, if it repeats. */
    series_id?: string | null;
    reminder_minutes: number | null;
    join_url: string | null;
    attendees: { name: string | null; address: string | null; kind: string; response: string }[];
}

export type View = "day" | "work_week" | "week" | "month" | "agenda";

export const listCalendars = async (account_id: string) =>
    (await execute<{ calendars: Calendar[] }>("calendar.calendars.list", { account_id })).calendars;

export const listEvents = async (account_id: string, start: Date, end: Date) =>
    (await execute<{ events: CalendarEvent[] }>("calendar.events.list", { account_id, start: start.toISOString(), end: end.toISOString() })).events;

export const getEvent = (account_id: string, event_id: string) =>
    execute<{ event: CalendarEvent; description: string; description_complete: boolean; repeat?: Repeat | null }>("calendar.events.get", { account_id, event_id });

/** How an event repeats: every so many days, weeks, months or years, from its start. */
export interface Repeat {
    unit: "day" | "week" | "month" | "year";
    every?: number;
    days_of_week?: string[] | null;
    /** For a month or a year: on that weekday of the month, not that day of it. */
    week_of_month?: string | null;
    /** The last day it may fall on. */
    until?: string | null;
    count?: number | null;
}

/** Which of a repeating series a change is for. */
export type Scope = "occurrence" | "following" | "series";

export interface Invited {
    address: string;
    name?: string;
    kind?: "required" | "optional" | "resource";
}

/** A line of people as typed, `Sam Taylor <sam@example.test>, jo@example.test`, as those invited. */
export function invitedFrom(line: string, kind: Invited["kind"]): Invited[] {
    return line
        .split(/[,;]/)
        .map((each) => each.trim())
        .filter(Boolean)
        .map((each) => {
            const named = /^(.*?)\s*<([^>]+)>$/.exec(each);
            const name = named?.[1].replace(/^"|"$/g, "").trim();
            return { address: (named?.[2] ?? each).trim(), ...(name ? { name } : {}), kind };
        });
}

/** Those invited, as a line to change. */
export const invitedLine = (people: { name: string | null; address: string | null }[]) =>
    people.map((each) => (each.name && each.name !== each.address ? `${each.name} <${each.address}>` : (each.address ?? ""))).join(", ");

/** What an event is made of, to make one or to change one. What is left out is left as it is. */
export interface EventFields {
    /** Everyone invited: the whole list. */
    attendees?: Invited[];
    /** True makes it a Teams meeting. */
    online_meeting?: boolean;
    repeat?: Repeat;
    subject?: string;
    /** By the clock in `time_zone`: `2026-10-20T09:00`. For an all-day event, its first day. */
    start?: string;
    /** For an all-day event, the day after its last. */
    end?: string;
    time_zone?: string;
    is_all_day?: boolean;
    location?: string;
    description?: string;
    show_as?: string;
    is_private?: boolean;
    reminder_minutes?: number | null;
}

export const createEvent = async (account_id: string, calendar_id: string | null, fields: EventFields) =>
    (await execute<{ event: CalendarEvent }>("calendar.events.create", { account_id, ...(calendar_id ? { calendar_id } : {}), ...fields })).event;

export const updateEvent = async (account_id: string, event_id: string, fields: EventFields, scope: Scope = "occurrence") =>
    (await execute<{ event: CalendarEvent }>("calendar.events.update", { account_id, event_id, ...(scope === "occurrence" ? {} : { scope }), ...fields })).event;

export const deleteEvent = (account_id: string, event_id: string, scope: Scope = "occurrence") =>
    execute("calendar.events.delete", { account_id, event_id, ...(scope === "occurrence" ? {} : { scope }) });

export interface FreeBusy {
    interval_minutes: number;
    people: { address: string; availability: string; busy: { status: string; start: string; end: string }[]; error: string | null }[];
    /** UTC: when everyone whose time is known is free. */
    free_slots: { start: string; end: string }[];
}

/** When people are free and busy between two moments, and when all of them are free at once. */
export const freeBusy = (account_id: string, people: string[], start: Date, end: Date, interval_minutes: number, duration_minutes: number) =>
    execute<FreeBusy>("calendar.free_busy", { account_id, people, start: start.toISOString(), end: end.toISOString(), interval_minutes, duration_minutes });

/** Cancel a meeting one organised, with words to everyone invited. */
export const cancelEvent = (account_id: string, event_id: string, comment: string, scope: Scope = "occurrence") =>
    execute("calendar.events.cancel", { account_id, event_id, ...(comment ? { comment } : {}), ...(scope === "series" ? { scope } : {}) });

const DAYS = ["sunday", "monday", "tuesday", "wednesday", "thursday", "friday", "saturday"];
const NTH = ["first", "second", "third", "fourth", "last"];
const name = (day: string) => day[0].toUpperCase() + day.slice(1);

/** How an event repeats, as a person would say it. */
export function repeatSaid(repeat: Repeat): string {
    const every = repeat.every ?? 1;
    const days = [...(repeat.days_of_week ?? [])].sort((a, b) => DAYS.indexOf(a) - DAYS.indexOf(b)).map(name);
    let said = every === 1 ? `Every ${repeat.unit}` : `Every ${every} ${repeat.unit}s`;
    if (repeat.unit === "week" && days.length > 0) said += ` on ${days.join(", ")}`;
    if (repeat.week_of_month && days.length > 0) said += ` on the ${repeat.week_of_month} ${days[0]}`;
    if (repeat.until) said += `, until ${new Date(`${repeat.until}T00:00`).toLocaleDateString(undefined, { day: "numeric", month: "short", year: "numeric" })}`;
    if (repeat.count) said += `, ${repeat.count} times`;
    return said;
}

/** Which weekday a day is, and which of its kind in its month: the second Tuesday, or the last. */
export function placeInMonth(day: Date): { weekday: string; nth: string; isLast: boolean } {
    const nth = Math.floor((day.getDate() - 1) / 7);
    const isLast = new Date(day.getFullYear(), day.getMonth(), day.getDate() + 7).getMonth() !== day.getMonth();
    return { weekday: DAYS[day.getDay()], nth: NTH[Math.min(nth, 3)], isLast };
}

export type Answer = "accept" | "tentative" | "decline";

/** Answer an invitation. The organiser is sent the answer unless `send_response` is false. */
export const respond = (account_id: string, event_id: string, response: Answer, comment: string, send_response: boolean) =>
    execute<{ response: string; organizer_told: boolean }>("calendar.events.respond", {
        account_id,
        event_id,
        response,
        send_response,
        ...(comment ? { comment } : {}),
    });

export const showCalendar = (account_id: string, calendar_id: string, shown: boolean) =>
    execute("calendar.calendars.set_shown", { account_id, calendar_id, shown });

/** Colours for calendars that have none of their own, by their place in the list. */
const PALETTE = ["#2f6fce", "#3a8a3a", "#b3305f", "#e8730c", "#7b4bb7", "#00857a", "#8e562e", "#5b7a8c"];

export const calendarColour = (calendars: Calendar[], id: string) => {
    const at = calendars.findIndex((c) => c.id === id);
    return calendars[at]?.hex_color || PALETTE[Math.max(0, at) % PALETTE.length];
};

// ---- days, by this computer's own calendar and clock ---------------------

export const startOfDay = (date: Date) => new Date(date.getFullYear(), date.getMonth(), date.getDate());
export const addDays = (date: Date, days: number) => new Date(date.getFullYear(), date.getMonth(), date.getDate() + days);
export const sameDay = (a: Date, b: Date) => a.getFullYear() === b.getFullYear() && a.getMonth() === b.getMonth() && a.getDate() === b.getDate();
/** The Monday on or before a day. */
export const startOfWeek = (date: Date) => addDays(date, -((date.getDay() + 6) % 7));

/** The days a view shows, given a day in it. */
export function daysOf(view: View, anchor: Date): Date[] {
    const day = startOfDay(anchor);
    if (view === "day") return [day];
    if (view === "work_week") return Array.from({ length: 5 }, (_, i) => addDays(startOfWeek(day), i));
    if (view === "week") return Array.from({ length: 7 }, (_, i) => addDays(startOfWeek(day), i));
    if (view === "agenda") return Array.from({ length: 30 }, (_, i) => addDays(day, i));
    // A month: whole weeks, from the Monday on or before its first day, six of them.
    const first = startOfWeek(new Date(day.getFullYear(), day.getMonth(), 1));
    return Array.from({ length: 42 }, (_, i) => addDays(first, i));
}

/** The day to show after stepping a view on or back. */
export function step(view: View, anchor: Date, by: 1 | -1): Date {
    if (view === "day") return addDays(anchor, by);
    if (view === "month") return new Date(anchor.getFullYear(), anchor.getMonth() + by, 1);
    if (view === "agenda") return addDays(anchor, 30 * by);
    return addDays(anchor, 7 * by);
}

/**
 * When an event starts and ends, for showing. A timed event is a moment and
 * is shown in this computer's zone. An all-day event is days, the same days
 * wherever it is seen from, so its dates are read as written and not moved
 * by the zone.
 */
export function span(event: CalendarEvent): { start: Date; end: Date } {
    if (!event.is_all_day) return { start: new Date(event.start), end: new Date(event.end) };
    const day = (text: string) => {
        const [year, month, date] = text.slice(0, 10).split("-").map(Number);
        return new Date(year, month - 1, date);
    };
    return { start: day(event.start), end: day(event.end) };
}

/** Whether an event is on a day, at all. */
export function onDay(event: CalendarEvent, day: Date): boolean {
    const { start, end } = span(event);
    const next = addDays(day, 1);
    // An event of no length belongs to the moment it is at.
    return start < next && (end > day || (start >= day && +start === +end));
}

export const timeSaid = (date: Date) => date.toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" });

/** A span as a person would say it. */
export function whenSaid(event: CalendarEvent): string {
    const { start, end } = span(event);
    const day = (date: Date) => date.toLocaleDateString(undefined, { weekday: "short", day: "numeric", month: "short", year: "numeric" });
    if (event.is_all_day) {
        const last = addDays(end, -1);
        return +last <= +start ? `${day(start)}, all day` : `${day(start)} to ${day(last)}, all day`;
    }
    return sameDay(start, end) ? `${day(start)}, ${timeSaid(start)} to ${timeSaid(end)}` : `${day(start)}, ${timeSaid(start)} to ${day(end)}, ${timeSaid(end)}`;
}

export interface Placed {
    event: CalendarEvent;
    /** Minutes from the day's midnight to where it begins and ends on this day. */
    from: number;
    to: number;
    /** Which of `of` columns it is in, where events overlap. */
    column: number;
    of: number;
}

/**
 * Lay a day's timed events out so that ones that overlap sit side by side.
 * Events are taken in order of start; each goes in the first column that is
 * free; a run of events that overlap one another share the width between them.
 */
export function layOut(events: CalendarEvent[], day: Date): Placed[] {
    const next = addDays(day, 1);
    const minutes = (date: Date) => Math.round((+date - +day) / 60_000);
    const timed = events
        .filter((event) => !event.is_all_day && onDay(event, day))
        .map((event) => {
            const { start, end } = span(event);
            const from = Math.max(0, minutes(start));
            // Shown at least a quarter of an hour tall, to be seen and clicked.
            const to = Math.max(from + 15, Math.min(minutes(next), minutes(end)));
            return { event, from, to, column: 0, of: 1 };
        })
        .sort((a, b) => a.from - b.from || b.to - a.to);

    let run: Placed[] = [];
    let runEnds = -1;
    const close = () => {
        const of = Math.max(...run.map((p) => p.column)) + 1;
        for (const placed of run) placed.of = of;
    };
    for (const placed of timed) {
        if (run.length > 0 && placed.from >= runEnds) {
            close();
            run = [];
        }
        const taken = new Set(run.filter((other) => other.to > placed.from).map((other) => other.column));
        let column = 0;
        while (taken.has(column)) column += 1;
        placed.column = column;
        run.push(placed);
        runEnds = Math.max(runEnds, placed.to);
    }
    if (run.length > 0) close();

    return timed;
}
