<script lang="ts">
    import { execute, say } from "./commands";

    interface Server {
        id: string;
        name: string;
        url: string;
        signed_in: "token" | "browser" | null;
        tools: number | null;
        tools_read_at: string | null;
    }

    interface Tool {
        name: string;
        title: string | null;
        description: string | null;
    }

    let servers = $state<Server[] | null>(null);
    let problem = $state<string | null>(null);
    let adding = $state<{ name: string; url: string } | null>(null);
    /** The server a token is being typed for. */
    let typing = $state<{ id: string; token: string } | null>(null);
    /** The server being signed in to through the browser. */
    let signing = $state<string | null>(null);
    let removing = $state<string | null>(null);
    let said = $state<Record<string, string>>({});
    let listed = $state<Record<string, Tool[]>>({});

    async function read() {
        try {
            servers = (await execute<{ servers: Server[] }>("mcp_servers.list")).servers;
        } catch (failure) {
            problem = say(failure);
        }
    }

    $effect(() => {
        read();
    });

    async function act(run: () => Promise<unknown>) {
        problem = null;
        try {
            await run();
        } catch (failure) {
            problem = say(failure);
        }
        await read();
    }

    async function add(submitted: SubmitEvent) {
        submitted.preventDefault();
        if (!adding) return;
        const { name, url } = adding;
        await act(async () => {
            await execute("mcp_servers.add", { name: name.trim(), url: url.trim() });
            adding = null;
        });
    }

    async function signIn(server: Server) {
        signing = server.id;
        await act(() => execute("mcp_servers.sign_in", { server_id: server.id }));
        signing = null;
    }

    async function saveToken(submitted: SubmitEvent) {
        submitted.preventDefault();
        if (!typing) return;
        const { id, token } = typing;
        await act(async () => {
            await execute("mcp_servers.set_token", { server_id: id, token: token.trim() });
            typing = null;
        });
    }

    async function test(server: Server) {
        said = { ...said, [server.id]: "Trying…" };
        try {
            const tried = await execute<{ ok: boolean; tools: number | null; seconds: number; error: string | null }>("mcp_servers.test", { server_id: server.id });
            said = { ...said, [server.id]: tried.ok ? `It works: ${tried.tools} tools, in ${tried.seconds.toFixed(1)} seconds.` : `It did not work: ${tried.error}` };
        } catch (failure) {
            said = { ...said, [server.id]: say(failure) };
        }
        await read();
    }

    async function showTools(server: Server) {
        try {
            listed = { ...listed, [server.id]: (await execute<{ tools: Tool[] }>("mcp_servers.tools", { server_id: server.id })).tools };
        } catch (failure) {
            problem = say(failure);
        }
    }

    const signedSaid = (server: Server) => (server.signed_in === "browser" ? "Signed in through the browser" : server.signed_in === "token" ? "Signed in with an access token" : "Not signed in");
</script>

<section class="outside" aria-label="Outside tool servers">
    <p class="muted">
        Tools on other services, such as a helpdesk, that a mailbox's instructions can be allowed to use. Add a service's MCP server here, on the computer that carries the instructions out, and sign in to it. What you sign in with stays on this computer.
    </p>
    {#if problem}
        <p class="problem" role="alert">{problem}</p>
    {/if}

    {#if servers}
        <ul aria-label="Servers">
            {#each servers as server (server.id)}
                <li>
                    <div class="what">
                        <span class="name">{server.name}</span>
                        <span class="muted">{server.url}</span>
                        <span class="muted">{signedSaid(server)}{server.tools !== null ? ` · ${server.tools} tools` : ""}</span>
                        {#if said[server.id]}<span role="status">{said[server.id]}</span>{/if}
                    </div>
                    <div class="buttons">
                        {#if signing === server.id}
                            <span role="status">Waiting for the browser…</span>
                        {:else if removing === server.id}
                            <button type="button" class="danger" onclick={() => act(() => execute("mcp_servers.remove", { server_id: server.id }))}>Remove it?</button>
                            <button type="button" onclick={() => (removing = null)}>Keep</button>
                        {:else}
                            <button type="button" onclick={() => signIn(server)}>Sign in through the browser</button>
                            <button type="button" onclick={() => (typing = { id: server.id, token: "" })}>Use an access token</button>
                            {#if server.signed_in}
                                <button type="button" onclick={() => test(server)}>Try it</button>
                                <button type="button" onclick={() => showTools(server)}>Its tools</button>
                            {/if}
                            <button type="button" onclick={() => (removing = server.id)}>Remove</button>
                        {/if}
                    </div>
                    {#if typing?.id === server.id}
                        <form class="token" aria-label={`Access token for ${server.name}`} onsubmit={saveToken}>
                            <input type="password" autocomplete="off" aria-label="Access token" bind:value={typing.token} placeholder="Paste the token the service gave you" />
                            <button type="submit" class="primary" disabled={!typing.token.trim()}>Save</button>
                            <button type="button" onclick={() => (typing = null)}>Cancel</button>
                        </form>
                    {/if}
                    {#if listed[server.id]}
                        <ul class="tools" aria-label={`Tools of ${server.name}`}>
                            {#each listed[server.id] as tool (tool.name)}
                                <li><strong>{tool.title ?? tool.name}</strong>{tool.description ? ` — ${tool.description}` : ""}</li>
                            {/each}
                        </ul>
                    {/if}
                </li>
            {:else}
                <li class="muted">No servers yet.</li>
            {/each}
        </ul>

        {#if adding}
            <form aria-label="Add a server" onsubmit={add}>
                <label class="stacked">Name <input type="text" bind:value={adding.name} maxlength="100" placeholder="Helpdesk" /></label>
                <label class="stacked">Address of its MCP server <input type="text" bind:value={adding.url} placeholder="https://helpdesk.example.com/mcp/helpdesk" /></label>
                <div class="buttons">
                    <button type="submit" class="primary" disabled={!adding.name.trim() || !adding.url.trim()}>Add</button>
                    <button type="button" onclick={() => (adding = null)}>Cancel</button>
                </div>
            </form>
        {:else}
            <div class="buttons"><button type="button" onclick={() => (adding = { name: "", url: "" })}>Add a server</button></div>
        {/if}
    {/if}
</section>

<style>
    .outside {
        margin: 0.75rem 0 0 3rem;
        display: flex;
        flex-direction: column;
        gap: 0.6rem;
        max-width: 42rem;
    }

    p {
        margin: 0;
        font-size: 0.85rem;
    }

    ul {
        margin: 0;
        padding: 0;
        list-style: none;
    }

    li {
        padding: 0.5rem 0;
        border-bottom: 1px solid var(--line);
        display: flex;
        flex-direction: column;
        gap: 0.4rem;
    }

    .what {
        display: flex;
        flex-direction: column;
        font-size: 0.85rem;
    }

    .name {
        font-weight: 600;
    }

    .tools li {
        padding: 0.2rem 0;
        border: 0;
        font-size: 0.85rem;
    }

    form {
        display: flex;
        flex-direction: column;
        gap: 0.4rem;
    }

    form.token {
        flex-direction: row;
    }

    .stacked {
        display: flex;
        flex-direction: column;
        gap: 0.2rem;
        font-size: 0.9rem;
    }

    input {
        font: inherit;
        padding: 0.35rem;
    }

    .buttons {
        display: flex;
        flex-wrap: wrap;
        gap: 0.5rem;
    }

    .problem {
        padding: 0.5rem 0.75rem;
        background: color-mix(in srgb, var(--danger) 14%, var(--surface));
    }
</style>
