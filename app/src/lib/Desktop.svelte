<script lang="ts">
    import { invoke } from "@tauri-apps/api/core";
    import { execute, say } from "./commands";

    interface Choices {
        keep_running: boolean;
        start_at_login: boolean;
    }

    let choices = $state<Choices>({ keep_running: false, start_at_login: false });
    /** Whether this desktop has a system tray for Brolga to sit in. Unknown until asked. */
    let tray = $state(true);
    let problem = $state<string | null>(null);

    /** Have the app act on what is recorded, and say whether it has a tray. */
    const apply = async () => (tray = await invoke<boolean>("apply_desktop"));

    execute<Choices>("app.desktop.get")
        .then((found) => (choices = found))
        .then(apply)
        .catch(() => {});

    async function change(what: Partial<Choices>) {
        problem = null;
        try {
            choices = await execute<Choices>("app.desktop.set", what);
            await apply();
        } catch (failure) {
            problem = say(failure);
        }
    }
</script>

<section class="desktop" aria-labelledby="desktop-heading">
    <h2 id="desktop-heading">This computer</h2>
    <label>
        <input type="checkbox" checked={choices.keep_running} onchange={(event) => change({ keep_running: event.currentTarget.checked })} />
        <span>
            Keep running when the window is closed
            <small class="muted">Mail goes on arriving and reminders are still given. Brolga sits in the system tray, where it is opened again or quit.</small>
        </span>
    </label>
    {#if choices.keep_running && !tray}
        <p class="muted" role="status">This desktop has no system tray for Brolga to sit in, so closing the window still quits it.</p>
    {/if}
    <label>
        <input type="checkbox" checked={choices.start_at_login} onchange={(event) => change({ start_at_login: event.currentTarget.checked })} />
        <span>
            Start when I sign in to this computer
            <small class="muted">With no window, where it is also kept running; otherwise with one.</small>
        </span>
    </label>
    {#if problem}
        <p role="alert">{problem}</p>
    {/if}
</section>

<style>
    .desktop {
        margin-top: 1rem;
        padding: 1rem 1.25rem;
        border: 1px solid var(--line);
        border-radius: 0.6rem;
        background: var(--raised);
    }

    h2 {
        margin: 0 0 0.6rem;
        font-size: 1rem;
    }

    label {
        display: flex;
        align-items: flex-start;
        gap: 0.6rem;
        padding: 0.3rem 0;
    }

    input {
        margin-top: 0.3rem;
    }

    small {
        display: block;
    }

    p {
        margin: 0.2rem 0 0.4rem 1.7rem;
        font-size: 0.9rem;
    }
</style>
