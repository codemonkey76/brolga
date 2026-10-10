<script lang="ts">
    import { execute, isCommandFailure } from "./lib/commands";

    interface Version {
        name: string;
        version: string;
    }

    const version = execute<Version>("app.version");
</script>

<main>
    {#await version}
        <p class="muted" role="status">Starting…</p>
    {:then about}
        <h1>{about.name}</h1>
        <p class="muted">Version {about.version}</p>
        <p>There is no account yet. Signing in arrives with the next milestone.</p>
    {:catch failure}
        <h1>Brolga could not start</h1>
        <p role="alert">{isCommandFailure(failure) ? failure.message : String(failure)}</p>
    {/await}
</main>

<style>
    main {
        height: 100%;
        display: flex;
        flex-direction: column;
        align-items: center;
        justify-content: center;
        gap: 0.5rem;
        padding: 1rem;
        box-sizing: border-box;
        text-align: center;
    }

    h1 {
        margin: 0;
        font-size: 2rem;
        font-weight: 600;
    }

    p {
        margin: 0;
    }

    .muted {
        color: var(--muted);
    }
</style>
