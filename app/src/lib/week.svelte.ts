/**
 * The day the week starts on, chosen in the calendar and kept on this
 * computer. Microsoft 365 keeps no such choice for a mailbox, and what a
 * computer's language says of it is often not what its user goes by, so it
 * is asked for, and is Monday until it is.
 */

import { WEEK_STARTS } from "./calendar";

const KEY = "brolga.week.start";

function kept(): number {
    try {
        const said = localStorage.getItem(KEY);
        const day = said === null ? NaN : Number(said);
        return WEEK_STARTS.some((each) => each.day === day) ? day : 1;
    } catch {
        return 1;
    }
}

/** `first` is the day as `getDay` counts them: 0 for Sunday. */
export const week = $state({ first: kept() });

export function setWeekStart(day: number) {
    if (!WEEK_STARTS.some((each) => each.day === day)) return;
    week.first = day;
    try {
        localStorage.setItem(KEY, String(day));
    } catch {
        // It holds for as long as the window is open.
    }
}
