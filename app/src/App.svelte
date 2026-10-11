<script lang="ts">
    import { type Account, listAccounts } from "./lib/accounts";
    import Accounts from "./lib/Accounts.svelte";
    import Agents from "./lib/Agents.svelte";
    import Questions from "./lib/Questions.svelte";
    import Mail from "./lib/Mail.svelte";

    type View = "mail" | "accounts" | "agents";

    let accounts = $state<Account[]>([]);
    let view = $state<View>("accounts");
    let started = $state(false);

    // With an account, the window opens on its mail. Without one, on adding one.
    listAccounts()
        .then((found) => {
            accounts = found;
            view = found.length > 0 ? "mail" : "accounts";
        })
        .catch(() => {})
        .finally(() => (started = true));

    function changed(now: Account[]) {
        const first = accounts.length === 0 && now.length > 0;
        accounts = now;
        if (first) view = "mail";
    }

    function keydown(event: KeyboardEvent) {
        if (!(event.ctrlKey || event.metaKey)) return;
        if (event.key === "1" && accounts.length > 0) view = "mail";
        else if (event.key === ",") view = "accounts";
        else return;
        event.preventDefault();
    }
</script>

<svelte:window onkeydown={keydown} />

<Questions />

<div class="shell">
    <nav class="rail" aria-label="Sections">
        <button type="button" class:current={view === "mail"} aria-current={view === "mail"} disabled={accounts.length === 0} onclick={() => (view = "mail")} title="Mail (Ctrl+1)">
            Mail
        </button>
        <button type="button" class:current={view === "accounts"} aria-current={view === "accounts"} onclick={() => (view = "accounts")} title="Accounts (Ctrl+,)">
            Accounts
        </button>
        <button type="button" class:current={view === "agents"} aria-current={view === "agents"} onclick={() => (view = "agents")} title="What AI agents have asked for">
            Agents
        </button>
    </nav>

    <main>
        {#if !started}
            <p class="muted starting" role="status">Starting…</p>
        {:else if view === "mail" && accounts.length > 0}
            <Mail {accounts} />
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
        border: 0;
        background: none;
        padding: 0.5rem 0.6rem;
        font-size: 0.8rem;
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
