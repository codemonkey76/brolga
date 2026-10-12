<script lang="ts">
    import { Bot, CalendarDays, CircleUser, Mail as MailIcon, Users } from "@lucide/svelte";
    import { type Account, type Archive, type SharedMailbox, listAccounts, listArchives, listShared, mailboxes } from "./lib/accounts";
    import Accounts from "./lib/Accounts.svelte";
    import { type Asked, nextRequest } from "./lib/commands";
    import Agents from "./lib/Agents.svelte";
    import Calendar from "./lib/Calendar.svelte";
    import Questions from "./lib/Questions.svelte";
    import Reminders from "./lib/Reminders.svelte";
    import Mail from "./lib/Mail.svelte";
    import NewMail from "./lib/NewMail.svelte";
    import StatusBar from "./lib/StatusBar.svelte";
    import Palette, { type Action } from "./lib/Palette.svelte";
    import People from "./lib/People.svelte";
    import { inDialog } from "./lib/typing";
    import { tick } from "svelte";

    type View = "mail" | "calendar" | "people" | "accounts" | "agents";

    let accounts = $state<Account[]>([]);
    let view = $state<View>("accounts");
    let started = $state(false);
    /** A day the calendar is to open on, when it is come to from a message about a meeting. */
    let calendarDay = $state<Date | null>(null);

    /** Mailboxes other people have shared, which are read beside the accounts' own. */
    let shared = $state<SharedMailbox[]>([]);
    /** Mail read in from files, which is read beside the accounts' own. */
    let archives = $state<Archive[]>([]);
    const reload = () => {
        listShared()
            .then((found) => (shared = found))
            .catch(() => {});
        listArchives()
            .then((found) => (archives = found))
            .catch(() => {});
    };

    // With an account, the window opens on its mail. Without one, on adding one.
    listAccounts()
        .then((found) => {
            accounts = found;
            reload();
            view = found.length > 0 ? "mail" : "accounts";
        })
        .catch(() => {})
        .finally(() => (started = true));

    /** A message a mailto: link asked for, until Mail has started it. */
    let asked = $state<Asked | null>(null);
    // Each link clicked is waited for in turn, for as long as the window is open.
    (async () => {
        for (;;) {
            let next: Asked | null;
            try {
                next = await nextRequest();
            } catch {
                return;
            }
            if (!next) return;
            asked = next;
            if (accounts.length > 0) view = "mail";
        }
    })();

    function changed(now: Account[]) {
        const first = accounts.length === 0 && now.length > 0;
        accounts = now;
        reload();
        if (first) view = "mail";
    }

    /** The calendar is to open on a new event being written. */
    let newEvent = $state(false);
    let palette = $state(false);

    function go(to: View) {
        if (to !== "accounts" && to !== "agents" && accounts.length === 0) return;
        newEvent = false;
        view = to;
    }

    function meeting() {
        if (accounts.length === 0) return;
        // Made afresh, so that it opens on a new event even from the calendar itself.
        view = "accounts";
        tick().then(() => {
            newEvent = true;
            view = "calendar";
        });
    }

    /** Press a key for the screen that is showing, once it is. */
    const press = (to: View, key: string, with_: { ctrl?: boolean; shift?: boolean } = {}) => () => {
        if (view !== to) go(to);
        tick().then(() => window.dispatchEvent(new KeyboardEvent("keydown", { key, ctrlKey: with_.ctrl, shiftKey: with_.shift, bubbles: true })));
    };

    /** Everything the window does by a key, for the palette to list and to do. */
    const actions: Action[] = [
        { title: "Go to Mail", keys: "Ctrl+1", run: () => go("mail") },
        { title: "Go to Calendar", keys: "Ctrl+2", run: () => go("calendar") },
        { title: "Go to People (address book)", keys: "Ctrl+3 or Ctrl+Shift+B", run: () => go("people") },
        { title: "Go to Accounts", keys: "", run: () => go("accounts") },
        { title: "Go to Agents", keys: "", run: () => go("agents") },
        { title: "New message", keys: "Ctrl+N", run: press("mail", "n", { ctrl: true }) },
        { title: "New meeting", keys: "Ctrl+Shift+Q", run: meeting },
        { title: "Reply", keys: "Ctrl+R", run: press("mail", "r", { ctrl: true }) },
        { title: "Reply all", keys: "Ctrl+Shift+R", run: press("mail", "r", { ctrl: true, shift: true }) },
        { title: "Forward", keys: "Ctrl+F", run: press("mail", "f", { ctrl: true }) },
        { title: "Delete the message", keys: "Delete", run: press("mail", "Delete") },
        { title: "Archive the message", keys: "Backspace", run: press("mail", "Backspace") },
        { title: "Move the message to a folder", keys: "Ctrl+Shift+V", run: press("mail", "v", { ctrl: true, shift: true }) },
        { title: "Mark as read", keys: "Ctrl+Q", run: press("mail", "q", { ctrl: true }) },
        { title: "Mark as unread", keys: "Ctrl+U", run: press("mail", "u", { ctrl: true }) },
        { title: "Flag or clear the flag", keys: "Insert", run: press("mail", "Insert") },
        { title: "Next message", keys: "Ctrl+.", run: press("mail", ".", { ctrl: true }) },
        { title: "Previous message", keys: "Ctrl+,", run: press("mail", ",", { ctrl: true }) },
        { title: "Search mail", keys: "Ctrl+E or F3", run: press("mail", "F3") },
        { title: "Undo", keys: "Ctrl+Z", run: press("mail", "z", { ctrl: true }) },
        { title: "Sync now", keys: "F9", run: () => window.dispatchEvent(new KeyboardEvent("keydown", { key: "F9", bubbles: true })) },
        { title: "Calendar: go to today", keys: "T", run: press("calendar", "t") },
    ];

    function keydown(event: KeyboardEvent) {
        if (!(event.ctrlKey || event.metaKey)) return;
        const key = event.key.toLowerCase();
        // The palette is put away by the keys that brought it. Nothing else is done behind it, or behind a question.
        if (key === "k" && (palette || !inDialog(event))) palette = !palette;
        else if (inDialog(event)) return;
        else if (key === "1") go("mail");
        else if (key === "2") go("calendar");
        else if (key === "3" || (event.shiftKey && key === "b")) go("people");
        else if (event.shiftKey && key === "q") meeting();
        else return;
        event.preventDefault();
    }
</script>

<svelte:window onkeydown={keydown} />

{#if palette}
    <Palette {actions} onClose={() => (palette = false)} />
{/if}
<Questions />
<Reminders {accounts} onOpen={() => (view = "calendar")} />
<NewMail {accounts} fetched={view === "mail" || view === "calendar"} />

<div class="shell">
    <nav class="rail" aria-label="Sections">
        <button type="button" class:current={view === "mail"} aria-current={view === "mail"} disabled={accounts.length === 0} onclick={() => (view = "mail")} title="Mail (Ctrl+1)">
            <MailIcon size={20} aria-hidden="true" />
            <span>Mail</span>
        </button>
        <button type="button" class:current={view === "calendar"} aria-current={view === "calendar"} disabled={accounts.length === 0} onclick={() => go("calendar")} title="Calendar (Ctrl+2)">
            <CalendarDays size={20} aria-hidden="true" />
            <span>Calendar</span>
        </button>
        <button type="button" class:current={view === "people"} aria-current={view === "people"} disabled={accounts.length === 0} onclick={() => (view = "people")} title="People (Ctrl+3)">
            <Users size={20} aria-hidden="true" />
            <span>People</span>
        </button>
        <button type="button" class:current={view === "accounts"} aria-current={view === "accounts"} onclick={() => (view = "accounts")} title="Accounts">
            <CircleUser size={20} aria-hidden="true" />
            <span>Accounts</span>
        </button>
        <button type="button" class:current={view === "agents"} aria-current={view === "agents"} onclick={() => (view = "agents")} title="What AI agents have asked for">
            <Bot size={20} aria-hidden="true" />
            <span>Agents</span>
        </button>
    </nav>

    <main>
        {#if !started}
            <p class="muted starting" role="status">Starting…</p>
        {:else if view === "mail" && accounts.length > 0}
            <Mail
                accounts={mailboxes(accounts, shared, false, archives)}
                linked={asked}
                onLinked={() => (asked = null)}
                onCalendar={(day) => {
                    calendarDay = day;
                    view = "calendar";
                }}
            />
        {:else if view === "calendar" && accounts.length > 0}
            <Calendar accounts={mailboxes(accounts, shared, true)} startOn={calendarDay} writing={newEvent} />
        {:else if view === "people" && accounts.length > 0}
            <People {accounts} />
        {:else if view === "agents"}
            <div class="scroll"><Agents /></div>
        {:else}
            <div class="scroll"><Accounts onChanged={changed} onArchives={reload} /></div>
        {/if}
    </main>

    {#if started && accounts.length > 0}
        <StatusBar {accounts} />
    {/if}
</div>

<style>
    .shell {
        height: 100vh;
        display: grid;
        grid-template-columns: auto minmax(0, 1fr);
        grid-template-rows: minmax(0, 1fr) auto;
    }

    .rail {
        display: flex;
        flex-direction: column;
        gap: 0.25rem;
        padding: 0.5rem 0.35rem;
        border-right: 1px solid var(--line);
        background: var(--raised);
    }

    .rail button {
        display: flex;
        flex-direction: column;
        align-items: center;
        gap: 0.2rem;
        border: 0;
        background: none;
        padding: 0.5rem 0.6rem;
        font-size: 0.7rem;
    }

    .rail button.current {
        background: color-mix(in srgb, var(--accent) 20%, var(--surface));
        font-weight: 600;
    }

    main {
        min-width: 0;
        min-height: 0;
    }

    .scroll {
        height: 100%;
        overflow-y: auto;
    }

    .starting {
        margin: 2rem;
    }
</style>
