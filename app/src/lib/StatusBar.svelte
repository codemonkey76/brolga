<script lang="ts">
    import { ArrowUpCircle, Check, RefreshCw, TriangleAlert } from "@lucide/svelte";
    import { getVersion } from "@tauri-apps/api/app";
    import { invoke } from "@tauri-apps/api/core";
    import { listen } from "@tauri-apps/api/event";
    import type { Account } from "./accounts";
    import { execute, notify, say } from "./commands";

    /** What a sync is busy with, as `app.sync.status` says it. */
    interface Now {
        doing: "sending" | "folders" | "mail" | "bodies" | "markers" | "calendar" | "contacts";
        folder?: string;
        fetched?: number | null;
        total?: number | null;
    }

    interface Status {
        account_id: string;
        username: string;
        needs_sign_in: boolean;
        syncing: boolean;
        now?: Now | null;
        folders: number;
        folders_synced: number;
        messages: number;
        synced_at: string | null;
    }

    let { accounts }: { accounts: Account[] } = $props();
    let statuses = $state<Status[]>([]);
    /** Which Brolga this is. */
    let version = $state("");
    getVersion()
        .then((found) => (version = found))
        .catch(() => {});

    /** A newer Brolga, as the app says of it. */
    interface Newer {
        version: string;
        /** It can be put in from here. Otherwise it is fetched from its page by hand. */
        installs: boolean;
        page: string;
    }

    let newer = $state<Newer | null>(null);
    /** The newer one is being fetched and put in. */
    let installing = $state(false);
    /** What came of asking, or of installing, when it is worth saying. */
    let said = $state<string | null>(null);
    let saidTimer: ReturnType<typeof setTimeout> | undefined;

    function tell(what: string) {
        clearTimeout(saidTimer);
        said = what;
        saidTimer = setTimeout(() => (said = null), 8_000);
    }

    /** Look for a newer Brolga. Looked for unasked, nothing is said unless one is found. */
    async function look(asked: boolean) {
        try {
            const found = await invoke<Newer | null>("check_update", { asked });
            // Said on the desktop once for each version, not at every look.
            if (found && found.version !== newer?.version) notify(`Brolga ${found.version} is available`, "It is offered along the bottom of Brolga's window.");
            newer = found;
            if (asked && !found) tell("This is the newest version.");
        } catch (failure) {
            if (asked) tell(say(failure));
        }
    }

    // When the window opens, and once a day for as long as it stays open.
    $effect(() => {
        look(false);
        const daily = setInterval(() => look(false), 24 * 60 * 60_000);
        // The tray's menu looks too, and says here what it found.
        const heard = listen<Newer>("update-found", (event) => (newer = event.payload)).catch(() => null);

        return () => {
            clearInterval(daily);
            heard.then((stop) => stop?.());
        };
    });

    async function install() {
        if (!newer || installing) return;
        installing = newer.installs;
        try {
            // Where it can be put in from here the app starts again as the new one, and this never answers.
            await invoke("install_update");
        } catch (failure) {
            tell(say(failure));
        } finally {
            installing = false;
        }
    }

    // Asked often while something is being fetched, so the numbers move, and
    // every few seconds otherwise, so a sync that starts is soon seen.
    $effect(() => {
        void accounts.length;
        let timer: ReturnType<typeof setTimeout> | undefined;
        let over = false;
        const ask = async () => {
            try {
                statuses = (await execute<{ accounts: Status[] }>("app.sync.status")).accounts;
            } catch {
                // What is wrong with a sync is said where the sync was asked for.
            }
            if (!over) timer = setTimeout(ask, statuses.some((each) => each.syncing) ? 1_500 : 4_000);
        };
        ask();

        return () => {
            over = true;
            clearTimeout(timer);
        };
    });

    const count = (n: number) => n.toLocaleString();
    const busy = $derived(statuses.find((each) => each.syncing) ?? null);
    const locked = $derived(statuses.find((each) => each.needs_sign_in) ?? null);
    /** A first sync that has not reached every folder, running or not. */
    const partial = $derived(busy ?? statuses.find((each) => each.folders_synced < each.folders) ?? null);
    const messages = $derived(statuses.reduce((sum, each) => sum + each.messages, 0));
    /** Whose it is, where there is more than one to tell apart. */
    const whose = (status: Status) => (statuses.length > 1 ? `${status.username}: ` : "");

    /** How far a folder being fetched whole has got. */
    const progress = $derived.by(() => {
        const now = busy?.now;
        if (now?.doing !== "mail" || now.fetched == null || now.total == null) return null;
        // The server's count can lag behind what it then sends.
        return { fetched: now.fetched, total: Math.max(now.total, now.fetched) };
    });

    function doing(status: Status): string {
        const now = status.now;
        // Another copy of Brolga is doing the syncing, and does not say what it has reached.
        if (!now) return "Syncing…";
        switch (now.doing) {
            case "sending":
                return "Sending your changes…";
            case "folders":
                return "Reading the folder list…";
            case "mail":
                return progress ? `Fetching ${now.folder || "a folder"}: ${count(progress.fetched)} of ${count(progress.total)} messages` : `Checking ${now.folder || "a folder"} for new mail…`;
            case "bodies":
                return "Fetching recent messages to read offline…";
            case "markers":
                return "Checking which messages you have answered…";
            case "calendar":
                return "Fetching the calendar…";
            case "contacts":
                return "Fetching contacts…";
        }
    }

    /** When everything was last brought up to date, as a person would say it. */
    function asOf(iso: string): string {
        const at = new Date(iso);
        const time = at.toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" });
        return at.toDateString() === new Date().toDateString() ? time : `${at.toLocaleDateString(undefined, { day: "numeric", month: "short" })}, ${time}`;
    }

    const oldest = $derived(
        statuses
            .map((each) => each.synced_at)
            .filter((each): each is string => !!each)
            .sort()[0] ?? null,
    );
</script>

<!-- Not a live region: the numbers change every second or two while mail is fetched. -->
{#if statuses.length > 0}
    <footer class="status" aria-label="Sync status">
        {#if busy}
            <span class="turning"><RefreshCw size={13} aria-hidden="true" /></span>
            <span class="doing">{whose(busy)}{doing(busy)}</span>
            {#if progress}
                <progress value={progress.fetched} max={Math.max(1, progress.total)} aria-label="How much of the folder has been fetched"></progress>
            {/if}
        {:else if locked}
            <TriangleAlert size={13} aria-hidden="true" />
            <span class="doing">{locked.username} needs signing in again, in Settings under Accounts.</span>
        {:else if partial}
            <span class="doing">{whose(partial)}Not all of your mail has been fetched yet. The rest comes with the next sync.</span>
        {:else}
            <Check size={13} aria-hidden="true" />
            <span class="doing">Up to date{oldest ? ` as of ${asOf(oldest)}` : ""}</span>
        {/if}

        <span class="spring"></span>
        {#if partial && partial.folders_synced < partial.folders}
            <!-- Why a folder is empty: its turn has not come. -->
            <span title="Mail is fetched a folder at a time, the inbox first. A folder is empty here until its turn comes.">
                {count(partial.folders_synced)} of {count(partial.folders)} folders fetched; the rest are empty until their turn
            </span>
            <span class="gap" aria-hidden="true">·</span>
        {/if}
        <span>{messages === 1 ? "1 message" : `${count(messages)} messages`} on this computer</span>
        {#if said}
            <span class="gap" aria-hidden="true">·</span>
            <span role="status">{said}</span>
        {/if}
        {#if newer}
            <span class="gap" aria-hidden="true">·</span>
            <button type="button" class="newer" disabled={installing} title={newer.installs ? "Fetch it, check it is Brolga's own, put it in and start again" : "Open the page it is downloaded from"} onclick={install}>
                <ArrowUpCircle size={13} aria-hidden="true" />
                {installing ? `Fetching Brolga ${newer.version}…` : newer.installs ? `Update to ${newer.version} and restart` : `Brolga ${newer.version} is available`}
            </button>
        {/if}
        {#if version}
            <span class="gap" aria-hidden="true">·</span>
            <button type="button" class="version" title="Check for a new version" onclick={() => look(true)}>Brolga {version}</button>
        {/if}
    </footer>
{/if}

<style>
    .status {
        grid-column: 1 / -1;
        display: flex;
        align-items: center;
        gap: 0.4rem;
        min-width: 0;
        padding: 0.2rem 0.75rem;
        border-top: 1px solid var(--line);
        background: var(--raised);
        font-size: 0.75rem;
        white-space: nowrap;
    }

    .doing {
        overflow: hidden;
        text-overflow: ellipsis;
    }

    .spring {
        flex: 1;
    }

    .status button {
        display: inline-flex;
        align-items: center;
        gap: 0.3rem;
        padding: 0 0.4rem;
        font-size: inherit;
    }

    .version {
        border-color: transparent;
        background: none;
    }

    .newer {
        border-color: transparent;
        background: var(--accent-fill);
        color: var(--on-accent);
        font-weight: 600;
    }

    progress {
        width: 8rem;
        height: 0.4rem;
        accent-color: var(--accent);
    }

    .turning {
        display: grid;
        animation: turn 1s linear infinite;
    }

    @keyframes turn {
        to {
            transform: rotate(360deg);
        }
    }

    @media (prefers-reduced-motion: reduce) {
        .turning {
            animation: none;
        }
    }
</style>
