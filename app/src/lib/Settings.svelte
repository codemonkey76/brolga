<script lang="ts">
    import { Archive, CalendarRange, CircleUser, Laptop, ListFilter, PenLine, Reply, Share2, Signature as SignatureIcon, Sparkles, Tag, Users } from "@lucide/svelte";
    import { type Account, listAccounts, listShared, mailboxes, type SharedMailbox } from "./accounts";
    import Accounts from "./Accounts.svelte";
    import Archives from "./Archives.svelte";
    import AutoReplies from "./AutoReplies.svelte";
    import { WEEK_STARTS } from "./calendar";
    import CalendarSharing from "./CalendarSharing.svelte";
    import Categories from "./Categories.svelte";
    import Desktop from "./Desktop.svelte";
    import Instructions from "./Instructions.svelte";
    import Rules from "./Rules.svelte";
    import SharedMailboxes from "./SharedMailboxes.svelte";
    import Signature from "./Signature.svelte";
    import { setWeekStart, week } from "./week.svelte";

    interface Props {
        /** Told whenever the list of accounts, or what is shared with them, may have changed. */
        onChanged: (accounts: Account[]) => void;
        /** Told whenever an archive is added, finishes or goes. */
        onArchives?: () => void;
    }

    let { onChanged, onArchives }: Props = $props();

    type Id = "accounts" | "shared" | "signature" | "replies" | "rules" | "instructions" | "categories" | "writing" | "calendar-sharing" | "week" | "computer" | "archives";

    interface Section {
        id: Id;
        name: string;
        icon: typeof CircleUser;
        /** Kept by a mailbox, so it is shown for one account at a time. */
        ofAccount?: boolean;
        /** Read from or written to Microsoft 365, so the account has to be signed in. */
        signedIn?: boolean;
        /** Kept by mailboxes shared with an account as well, which are offered beside the accounts. */
        sharedToo?: boolean;
    }

    /** Everything there is to set, under the part of Brolga it is about. */
    const GROUPS: { name: string; sections: Section[] }[] = [
        {
            name: "Accounts",
            sections: [
                { id: "accounts", name: "Accounts", icon: CircleUser },
                { id: "shared", name: "Shared mailboxes", icon: Users, ofAccount: true, signedIn: true },
            ],
        },
        {
            name: "Mail",
            sections: [
                { id: "signature", name: "Signature", icon: SignatureIcon, ofAccount: true },
                { id: "replies", name: "Automatic replies", icon: Reply, ofAccount: true },
                { id: "rules", name: "Rules", icon: ListFilter, ofAccount: true, signedIn: true },
                { id: "instructions", name: "Instructions", icon: Sparkles, ofAccount: true, signedIn: true, sharedToo: true },
                { id: "categories", name: "Categories", icon: Tag, ofAccount: true, signedIn: true },
                { id: "writing", name: "Writing", icon: PenLine },
            ],
        },
        {
            name: "Calendar",
            sections: [
                { id: "calendar-sharing", name: "Calendar sharing", icon: Share2, ofAccount: true, signedIn: true },
                { id: "week", name: "Week", icon: CalendarRange },
            ],
        },
        {
            name: "This computer",
            sections: [
                { id: "computer", name: "Start-up and updates", icon: Laptop },
                { id: "archives", name: "Archives", icon: Archive },
            ],
        },
    ];
    const SECTIONS = GROUPS.flatMap((each) => each.sections);

    const KEY = "brolga.settings.section";
    let shown = $state<Id>("accounts");
    try {
        const kept = localStorage.getItem(KEY);
        if (SECTIONS.some((each) => each.id === kept)) shown = kept as Id;
    } catch {
        // Accounts, then.
    }

    function show(id: Id) {
        shown = id;
        said = null;
        try {
            localStorage.setItem(KEY, id);
        } catch {
            // It holds for as long as the window is open.
        }
    }

    let accounts = $state<Account[]>([]);
    /** The mailboxes shared with the accounts, for the sections they have their own settings in. */
    let shared = $state<SharedMailbox[]>([]);
    let loaded = $state(false);
    /** The account whose settings are shown, where a section is a mailbox's. */
    let chosen = $state<string | null>(null);
    listAccounts()
        .then((listed) => (accounts = listed))
        .catch(() => {})
        .finally(() => (loaded = true));
    listShared()
        .then((listed) => (shared = listed))
        .catch(() => {});

    function changed(now: Account[]) {
        accounts = now;
        onChanged(now);
    }

    const section = $derived(SECTIONS.find((each) => each.id === shown) ?? SECTIONS[0]);
    /** The mailboxes the section can be shown for. */
    const choices = $derived(section.sharedToo ? mailboxes(accounts, shared) : accounts);
    const account = $derived(choices.find((each) => each.id === chosen) ?? choices[0] ?? null);
    /** A mailbox's settings need an account, and most need it signed in. Until then the section is not offered. */
    const offered = (each: Section) => !each.ofAccount || accounts.length > 0;
    // With no account to set anything for, there is only the accounts themselves to see.
    $effect(() => {
        if (loaded && !offered(section)) shown = "accounts";
    });

    /** What a section said as it was done with: that automatic replies are now on, say. */
    let said = $state<string | null>(null);
    /** Bumped to read a section afresh once it has saved and closed itself. */
    let again = $state(0);

    let plain = $state(false);
    try {
        plain = localStorage.getItem("brolga.compose.plain") === "1";
    } catch {
        // Formatted, then.
    }
    function writePlain(on: boolean) {
        plain = on;
        try {
            localStorage.setItem("brolga.compose.plain", on ? "1" : "0");
        } catch {
            // It holds for as long as the window is open.
        }
    }
</script>

<div class="settings">
    <nav aria-label="Settings">
        <h1>Settings</h1>
        {#each GROUPS as group (group.name)}
            <h2>{group.name}</h2>
            <ul>
                {#each group.sections as each (each.id)}
                    {@const Icon = each.icon}
                    <li>
                        <button type="button" class:current={shown === each.id} aria-current={shown === each.id ? "page" : undefined} disabled={!offered(each)} onclick={() => show(each.id)}>
                            <Icon size={17} aria-hidden="true" />
                            <span>{each.name}</span>
                        </button>
                    </li>
                {/each}
            </ul>
        {/each}
    </nav>

    <section class="pane" aria-labelledby="settings-heading">
        <header>
            <h2 id="settings-heading">{section.name}</h2>
            {#if section.ofAccount && choices.length > 1}
                <select aria-label="Account" value={account?.id} onchange={(event) => ((chosen = event.currentTarget.value), (said = null))}>
                    {#each choices as each (each.id)}
                        <option value={each.id}>{each.username}</option>
                    {/each}
                </select>
            {/if}
        </header>

        <div class="panel">
            {#if shown === "accounts"}
                <Accounts onChanged={changed} />
            {:else if shown === "writing"}
                <label class="choice">
                    <input type="checkbox" checked={plain} onchange={(event) => writePlain(event.currentTarget.checked)} />
                    <span>
                        Write new messages in plain text
                        <small class="muted">With no bold, lists, links or pictures within the message. It can be changed for one message where it is written.</small>
                    </span>
                </label>
            {:else if shown === "week"}
                <label class="pick">
                    Week starts on
                    <select aria-label="Week starts on" value={week.first} onchange={(event) => setWeekStart(Number(event.currentTarget.value))}>
                        {#each WEEK_STARTS as each (each.day)}
                            <option value={each.day}>{each.name}</option>
                        {/each}
                    </select>
                </label>
                <p class="muted">The week, the month and the small month in the calendar begin on this day, and so does “this week” in a list of mail. The working week is Monday to Friday whichever is chosen.</p>
            {:else if shown === "computer"}
                <Desktop />
            {:else if shown === "archives"}
                <Archives onChanged={onArchives} />
            {:else if account}
                {#if section.signedIn && account.needs_sign_in}
                    <p class="muted" role="status">{account.username} has to sign in again before this can be read. That is done under Accounts.</p>
                {:else}
                    {#key `${account.id} ${again}`}
                        {#if shown === "shared"}
                            <SharedMailboxes accountId={account.id} username={account.username} onShared={() => onChanged(accounts)} />
                        {:else if shown === "signature"}
                            <Signature accountId={account.id} username={account.username} />
                        {:else if shown === "replies"}
                            <AutoReplies
                                accountId={account.id}
                                username={account.username}
                                onClose={(closed) => {
                                    said = closed ?? null;
                                    again += 1;
                                }}
                            />
                        {:else if shown === "rules"}
                            <Rules accountId={account.id} username={account.username} />
                        {:else if shown === "instructions"}
                            <Instructions accountId={account.id} username={account.username} />
                        {:else if shown === "categories"}
                            <Categories accountId={account.id} username={account.username} />
                        {:else if shown === "calendar-sharing"}
                            <CalendarSharing accountId={account.id} username={account.username} />
                        {/if}
                    {/key}
                {/if}
            {/if}
            {#if said}
                <p class="said muted" role="status">{said}</p>
            {/if}
        </div>
    </section>
</div>

<style>
    .settings {
        height: 100%;
        display: grid;
        grid-template-columns: 15rem minmax(0, 1fr);
    }

    nav {
        overflow-y: auto;
        padding: 1.25rem 0.6rem;
        border-right: 1px solid var(--line);
        background: var(--raised);
    }

    h1,
    h2,
    ul,
    p {
        margin: 0;
    }

    h1 {
        padding: 0 0.6rem 0.75rem;
        font-size: 1.25rem;
        font-weight: 600;
    }

    nav h2 {
        padding: 0.9rem 0.6rem 0.3rem;
        font-size: 0.7rem;
        font-weight: 600;
        letter-spacing: 0.06em;
        text-transform: uppercase;
        color: var(--muted);
    }

    ul {
        padding: 0;
        list-style: none;
    }

    nav button {
        width: 100%;
        display: flex;
        align-items: center;
        gap: 0.6rem;
        border: 0;
        border-radius: 0.4rem;
        background: none;
        padding: 0.4rem 0.6rem;
        text-align: left;
        font-size: 0.9rem;
    }

    nav button:hover:not(:disabled) {
        background: color-mix(in srgb, CanvasText 7%, transparent);
    }

    nav button.current,
    nav button.current:hover {
        background: color-mix(in srgb, var(--accent) 20%, var(--surface));
        font-weight: 600;
    }

    .pane {
        min-width: 0;
        overflow-y: auto;
        padding: 1.25rem 2rem 2rem;
    }

    header {
        display: flex;
        align-items: center;
        justify-content: space-between;
        gap: 1rem;
        padding-bottom: 0.9rem;
        margin-bottom: 1.1rem;
        border-bottom: 1px solid var(--line);
    }

    header h2 {
        font-size: 1.25rem;
        font-weight: 600;
    }

    header select {
        max-width: 22rem;
        font: inherit;
        font-size: 0.9rem;
        padding: 0.3rem 0.5rem;
    }

    /* Each section was made to sit under an account's row, set in from it. Here it has the pane to itself. */
    .settings .pane .panel > :global(*) {
        margin-top: 0;
        margin-left: 0;
    }

    /* Named by the pane's own heading here, so its own is for a screen reader alone. */
    .panel :global(#archives-heading) {
        position: absolute;
        width: 1px;
        height: 1px;
        overflow: hidden;
        clip-path: inset(50%);
    }

    .choice {
        display: flex;
        align-items: flex-start;
        gap: 0.6rem;
        max-width: 36rem;
    }

    .choice span {
        display: flex;
        flex-direction: column;
        gap: 0.15rem;
    }

    .pick {
        display: flex;
        align-items: center;
        gap: 0.6rem;
        font-weight: 600;
    }

    .panel > p {
        max-width: 36rem;
        font-size: 0.85rem;
    }

    .pick + p {
        margin-top: 0.6rem;
    }

    .said {
        margin-top: 0.9rem;
    }
</style>
