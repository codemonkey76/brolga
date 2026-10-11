<script lang="ts">
    import { Bot, CalendarDays, CircleUser, Mail as MailIcon, Users } from "@lucide/svelte";
    import { type Account, type SharedMailbox, listAccounts, listShared, mailboxes } from "./lib/accounts";
    import Accounts from "./lib/Accounts.svelte";
    import Agents from "./lib/Agents.svelte";
    import Calendar from "./lib/Calendar.svelte";
    import Questions from "./lib/Questions.svelte";
    import Reminders from "./lib/Reminders.svelte";
    import Mail from "./lib/Mail.svelte";
    import People from "./lib/People.svelte";

    type View = "mail" | "calendar" | "people" | "accounts" | "agents";

    let accounts = $state<Account[]>([]);
    let view = $state<View>("accounts");
    let started = $state(false);
    /** A day the calendar is to open on, when it is come to from a message about a meeting. */
    let calendarDay = $state<Date | null>(null);

    /** Mailboxes other people have shared, which are read beside the accounts' own. */
    let shared = $state<SharedMailbox[]>([]);
    const reload = () =>
        listShared()
            .then((found) => (shared = found))
            .catch(() => {});

    // With an account, the window opens on its mail. Without one, on adding one.
    listAccounts()
        .then((found) => {
            accounts = found;
            reload();
            view = found.length > 0 ? "mail" : "accounts";
        })
        .catch(() => {})
        .finally(() => (started = true));

    function changed(now: Account[]) {
        const first = accounts.length === 0 && now.length > 0;
        accounts = now;
        reload();
        if (first) view = "mail";
    }

    function keydown(event: KeyboardEvent) {
        if (!(event.ctrlKey || event.metaKey)) return;
        if (event.key === "1" && accounts.length > 0) view = "mail";
        else if (event.key === "2" && accounts.length > 0) view = "calendar";
        else if (event.key === "3" && accounts.length > 0) view = "people";
        else if (event.key === ",") view = "accounts";
        else return;
        event.preventDefault();
    }
</script>

<svelte:window onkeydown={keydown} />

<Questions />
<Reminders {accounts} onOpen={() => (view = "calendar")} />

<div class="shell">
    <nav class="rail" aria-label="Sections">
        <button type="button" class:current={view === "mail"} aria-current={view === "mail"} disabled={accounts.length === 0} onclick={() => (view = "mail")} title="Mail (Ctrl+1)">
            <MailIcon size={20} aria-hidden="true" />
            <span>Mail</span>
        </button>
        <button type="button" class:current={view === "calendar"} aria-current={view === "calendar"} disabled={accounts.length === 0} onclick={() => (view = "calendar")} title="Calendar (Ctrl+2)">
            <CalendarDays size={20} aria-hidden="true" />
            <span>Calendar</span>
        </button>
        <button type="button" class:current={view === "people"} aria-current={view === "people"} disabled={accounts.length === 0} onclick={() => (view = "people")} title="People (Ctrl+3)">
            <Users size={20} aria-hidden="true" />
            <span>People</span>
        </button>
        <button type="button" class:current={view === "accounts"} aria-current={view === "accounts"} onclick={() => (view = "accounts")} title="Accounts (Ctrl+,)">
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
                accounts={mailboxes(accounts, shared)}
                onCalendar={(day) => {
                    calendarDay = day;
                    view = "calendar";
                }}
            />
        {:else if view === "calendar" && accounts.length > 0}
            <Calendar accounts={mailboxes(accounts, shared)} startOn={calendarDay} />
        {:else if view === "people" && accounts.length > 0}
            <People {accounts} />
        {:else if view === "agents"}
            <div class="scroll"><Agents /></div>
        {:else}
            <div class="scroll"><Accounts onChanged={changed} /></div>
        {/if}
    </main>
</div>

<style>
    .shell {
        height: 100vh;
        display: grid;
        grid-template-columns: auto minmax(0, 1fr);
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
        height: 100vh;
    }

    .scroll {
        height: 100%;
        overflow-y: auto;
    }

    .starting {
        margin: 2rem;
    }
</style>
