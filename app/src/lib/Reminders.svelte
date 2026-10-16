<script lang="ts">
    import { BellRing, ListTodo } from "@lucide/svelte";
    import type { Account } from "./accounts";
    import { type CalendarEvent, timeSaid, span } from "./calendar";
    import { execute, notify } from "./commands";

    interface Props {
        accounts: Account[];
        /** Go to the calendar, to the event's day. */
        onOpen: () => void;
        /** Go to a task, in Tasks. */
        onOpenTask?: (account_id: string, task_id: string) => void;
    }

    interface TaskReminder {
        account_id: string;
        task: { id: string; title: string; due: string | null; reminder_at: string };
    }

    interface Reminder {
        account_id: string;
        event: CalendarEvent;
        starts_in_minutes: number;
    }

    let { accounts, onOpen, onOpenTask }: Props = $props();
    let reminders = $state<Reminder[]>([]);
    let taskReminders = $state<TaskReminder[]>([]);
    /** The reminders the desktop has already been told of, so each is told once. */
    const told = new Set<string>();
    /** Bumped with each look, and whenever a reminder is dismissed or put off, so that a look begun before then does not bring it back. */
    let looks = 0;

    const zone = Intl.DateTimeFormat().resolvedOptions().timeZone;

    /** How long until it starts, or since it did, as a person would say it. */
    function awaySaid(minutes: number): string {
        const less = (n: number) => (n === 1 ? "a minute" : `${n} minutes`);
        // Within two hours the minutes over the hour are said: an hour and fifty minutes is not an hour.
        const span = (n: number) => (n >= 120 ? `${Math.round(n / 60)} hours` : n > 60 ? `an hour and ${less(n - 60)}` : n === 60 ? "an hour" : less(n));
        if (minutes > 0) return `in ${span(minutes)}`;
        if (minutes === 0) return "now";
        return `started ${span(-minutes)} ago`;
    }

    const whenSaid = (reminder: Reminder) =>
        reminder.event.is_all_day ? "All day" : `${timeSaid(span(reminder.event).start)} · ${awaySaid(reminder.starts_in_minutes)}`;

    async function look() {
        const mine = ++looks;
        const found: Reminder[] = [];
        const tasks: TaskReminder[] = [];
        for (const account of accounts) {
            try {
                const got = await execute<{ reminders: Omit<Reminder, "account_id">[] }>("calendar.reminders.due", { account_id: account.id, time_zone: zone });
                found.push(...got.reminders.map((each) => ({ ...each, account_id: account.id })));
            } catch {
                // Nothing to remind of until the client can be asked again.
            }
            try {
                const got = await execute<{ reminders: TaskReminder["task"][] }>("tasks.reminders.due", { account_id: account.id });
                tasks.push(...got.reminders.map((task) => ({ task, account_id: account.id })));
            } catch {
                // An account that does not reach To Do has no task reminders.
            }
        }
        if (mine !== looks) return;
        reminders = found;
        taskReminders = tasks;
        for (const reminder of tasks) {
            if (told.has(reminder.task.id)) continue;
            told.add(reminder.task.id);
            notify(reminder.task.title || "(No title)", reminder.task.due ? `Task due ${new Date(`${reminder.task.due}T00:00`).toLocaleDateString(undefined, { weekday: "short", day: "numeric", month: "short" })}` : "Task");
        }
        for (const reminder of found) {
            if (told.has(reminder.event.id)) continue;
            told.add(reminder.event.id);
            notify(reminder.event.subject || "(No subject)", [whenSaid(reminder), reminder.event.location].filter(Boolean).join(" · "));
        }
    }

    // Looked for every half minute. This asks only this computer, so it goes
    // on with no network.
    $effect(() => {
        void accounts;
        look();
        const timer = setInterval(look, 30_000);
        return () => clearInterval(timer);
    });

    /** Take a reminder off the list and tell the core what was done with it. True if the core took that. */
    async function put(reminder: Reminder, command: string, more: Record<string, unknown> = {}): Promise<boolean> {
        reminders = reminders.filter((each) => each !== reminder);
        looks += 1;
        try {
            await execute(command, { account_id: reminder.account_id, event_id: reminder.event.id, ...more });
            // A look begun while that was being done may have been answered before it was.
            looks += 1;
            return true;
        } catch {
            // It was not taken, so the reminder is still to be given: it is shown again.
            await look();
            return false;
        }
    }

    const dismiss = (reminder: Reminder) => put(reminder, "calendar.reminders.dismiss");

    /** Do something with a task's reminder, and take it off the list. */
    async function forTask(reminder: TaskReminder, command: string, more: Record<string, unknown> = {}) {
        taskReminders = taskReminders.filter((each) => each !== reminder);
        looks += 1;
        try {
            await execute(command, { account_id: reminder.account_id, task_id: reminder.task.id, ...more });
            looks += 1;
            if (command === "tasks.reminders.snooze") told.delete(reminder.task.id);
        } catch {
            await look();
        }
    }

    const snoozeTask = (reminder: TaskReminder, minutes: number) =>
        forTask(reminder, "tasks.reminders.snooze", { until: new Date(Date.now() + minutes * 60_000).toISOString() });

    async function snooze(reminder: Reminder, minutes: number) {
        // When it comes back, the desktop is told again.
        if (await put(reminder, "calendar.reminders.snooze", { minutes })) told.delete(reminder.event.id);
    }
</script>

{#if reminders.length > 0 || taskReminders.length > 0}
    <section class="reminders" aria-label="Reminders">
        {#each taskReminders as reminder (reminder.task.id)}
            <article>
                <ListTodo size={18} aria-hidden="true" />
                <div class="what">
                    <button type="button" class="subject" onclick={() => onOpenTask?.(reminder.account_id, reminder.task.id)}>{reminder.task.title || "(No title)"}</button>
                    <span class="muted">Task{reminder.task.due ? ` · due ${new Date(`${reminder.task.due}T00:00`).toLocaleDateString(undefined, { weekday: "short", day: "numeric", month: "short" })}` : ""}</span>
                </div>
                <select aria-label="Snooze {reminder.task.title}" onchange={(event) => event.currentTarget.value && snoozeTask(reminder, Number(event.currentTarget.value))}>
                    <option value="">Snooze…</option>
                    <option value="5">5 minutes</option>
                    <option value="15">15 minutes</option>
                    <option value="60">1 hour</option>
                    <option value="1440">Tomorrow</option>
                </select>
                <button type="button" onclick={() => forTask(reminder, "tasks.complete")}>Done</button>
                <button type="button" onclick={() => forTask(reminder, "tasks.reminders.dismiss")}>Dismiss</button>
            </article>
        {/each}
        {#each reminders as reminder (reminder.event.id)}
            <article>
                <BellRing size={18} aria-hidden="true" />
                <div class="what">
                    <button type="button" class="subject" onclick={onOpen}>{reminder.event.subject || "(No subject)"}</button>
                    <span class="muted">{whenSaid(reminder)}{reminder.event.location ? ` · ${reminder.event.location}` : ""}</span>
                </div>
                <select aria-label="Snooze {reminder.event.subject}" onchange={(event) => event.currentTarget.value && snooze(reminder, Number(event.currentTarget.value))}>
                    <option value="">Snooze…</option>
                    <option value="5">5 minutes</option>
                    <option value="15">15 minutes</option>
                    <option value="60">1 hour</option>
                    <option value="1440">Tomorrow</option>
                </select>
                <button type="button" onclick={() => dismiss(reminder)}>Dismiss</button>
            </article>
        {/each}
        {#if reminders.length > 1}
            <button type="button" class="all" onclick={() => [...reminders].forEach(dismiss)}>Dismiss all</button>
        {/if}
    </section>
{/if}

<style>
    .reminders {
        position: fixed;
        right: 1rem;
        bottom: 1rem;
        z-index: 40;
        width: min(26rem, calc(100vw - 2rem));
        max-height: 60vh;
        overflow-y: auto;
        display: flex;
        flex-direction: column;
        gap: 0.4rem;
    }

    article {
        display: flex;
        align-items: center;
        gap: 0.6rem;
        padding: 0.6rem 0.75rem;
        background: var(--surface);
        border: 1px solid var(--line);
        border-left: 3px solid var(--accent);
        border-radius: 0.5rem;
        box-shadow: 0 6px 20px color-mix(in srgb, CanvasText 22%, transparent);
    }

    .what {
        flex: 1;
        min-width: 0;
        display: flex;
        flex-direction: column;
    }

    .subject {
        border: 0;
        background: none;
        padding: 0;
        text-align: left;
        font-weight: 600;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    }

    .muted {
        font-size: 0.8rem;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    }

    select,
    article > button {
        flex: none;
        font-size: 0.8rem;
        padding: 0.2rem 0.5rem;
    }

    .all {
        align-self: flex-end;
        font-size: 0.8rem;
    }
</style>
