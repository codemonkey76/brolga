<script lang="ts">
    import { type Opened, who } from "./mail";

    interface Props {
        opened: Opened | null;
        loading: boolean;
        problem: string | null;
        /** True once the person has asked for this message's remote images. */
        imagesShown: boolean;
        onShowImages: () => void;
        onOpenLink: (url: string) => void;
    }

    let { opened, loading, problem, imagesShown, onShowImages, onOpenLink }: Props = $props();
    let frame = $state<HTMLIFrameElement>();
    /** Where the link under the pointer really goes. */
    let hover = $state<string | null>(null);
    let mismatch = $state(false);

    // The message is shown in a frame that cannot run scripts, submit forms
    // or open windows. It has been cleaned by the core before it gets here;
    // the frame is the second line, not the first.
    const page = $derived(
        opened
            ? `<!doctype html><html><head><meta charset="utf-8"><style>
                :root { color-scheme: light dark; }
                body { margin: 16px; font: 14px/1.5 system-ui, sans-serif; overflow-wrap: anywhere; }
                img { max-width: 100%; height: auto; }
                table { max-width: 100%; }
                pre { white-space: pre-wrap; font: inherit; }
                blockquote { margin: 0 0 0 4px; padding-left: 12px; border-left: 3px solid #8886; }
                </style></head><body>${opened.body.content}</body></html>`
            : "",
    );

    /** True when a link's own words look like an address other than the one it goes to. */
    function looksLikeAnotherAddress(link: HTMLAnchorElement): boolean {
        const words = (link.textContent ?? "").trim();
        if (!/^(https?:\/\/|www\.)\S+$/i.test(words)) return false;
        try {
            const shown = new URL(words.startsWith("www.") ? `https://${words}` : words).hostname;
            return shown.replace(/^www\./, "") !== new URL(link.href).hostname.replace(/^www\./, "");
        } catch {
            return true;
        }
    }

    // Links never open inside the app. A click is caught here and handed to
    // the core, which passes a web address to the browser and nothing else.
    function wire() {
        const document = frame?.contentDocument;
        if (!document) return;
        const link = (event: Event) => (event.target as Element | null)?.closest?.("a[href]") as HTMLAnchorElement | null;

        document.addEventListener("click", (event) => {
            const target = link(event);
            event.preventDefault();
            if (target) onOpenLink(target.getAttribute("href") ?? "");
        });
        document.addEventListener("mouseover", (event) => {
            const target = link(event);
            hover = target ? (target.getAttribute("href") ?? "") : null;
            mismatch = target ? looksLikeAnotherAddress(target) : false;
        });
        document.addEventListener("mouseleave", () => (hover = null));
    }

    const people = (list: Opened["message"]["to"]) => list.map(who).join(", ");
</script>

<section class="pane" aria-label="Message">
    {#if problem}
        <p class="note problem" role="alert">{problem}</p>
    {:else if loading && !opened}
        <p class="empty muted" role="status">Opening…</p>
    {:else if !opened}
        <p class="empty muted">Select a message to read it.</p>
    {:else}
        <header>
            <h2>{opened.message.subject || "(No subject)"}</h2>
            <p>
                <strong>{who(opened.message.from)}</strong>
                {#if opened.message.from.name && opened.message.from.address}
                    <span class="muted">&lt;{opened.message.from.address}&gt;</span>
                {/if}
            </p>
            <p class="muted">
                To {people(opened.message.to) || "nobody"}{#if opened.message.cc.length}, cc {people(opened.message.cc)}{/if}
                · {new Date(opened.message.received_at).toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" })}
            </p>
        </header>

        {#if opened.body.blocked_remote_content > 0}
            <p class="note" role="status">
                {#if imagesShown}
                    {opened.body.blocked_remote_content === 1 ? "1 picture" : `${opened.body.blocked_remote_content} pictures`} could not be fetched.
                {:else}
                    {opened.body.blocked_remote_content === 1 ? "1 remote picture was" : `${opened.body.blocked_remote_content} remote pictures were`}
                    not loaded, so the sender cannot tell you opened this.
                    <button type="button" onclick={onShowImages}>Show pictures</button>
                {/if}
            </p>
        {/if}

        {#key opened.message.id + String(imagesShown)}
            <iframe bind:this={frame} title="Message body" sandbox="allow-same-origin" srcdoc={page} onload={wire}></iframe>
        {/key}

        <p class="status" class:warn={mismatch} aria-live="polite">
            {#if hover}{mismatch ? "This link goes somewhere other than it says: " : ""}{hover}{/if}
        </p>
    {/if}
</section>

<style>
    .pane {
        height: 100%;
        display: flex;
        flex-direction: column;
        min-width: 0;
    }

    header {
        padding: 0.75rem 1rem;
        border-bottom: 1px solid var(--line);
        display: flex;
        flex-direction: column;
        gap: 0.15rem;
        font-size: 0.9rem;
    }

    h2,
    p {
        margin: 0;
    }

    h2 {
        font-size: 1.15rem;
        font-weight: 600;
    }

    iframe {
        flex: 1;
        width: 100%;
        min-height: 0;
        border: 0;
        background: Canvas;
    }

    .empty {
        margin: auto;
    }

    .note {
        margin: 0.5rem 1rem 0;
        padding: 0.5rem 0.75rem;
        border-radius: 0.4rem;
        background: color-mix(in srgb, var(--warn) 18%, var(--surface));
        font-size: 0.85rem;
        display: flex;
        align-items: center;
        gap: 0.75rem;
        flex-wrap: wrap;
    }

    .problem {
        background: color-mix(in srgb, var(--danger) 12%, var(--surface));
    }

    .status {
        min-height: 1.4rem;
        padding: 0.1rem 1rem;
        border-top: 1px solid var(--line);
        font-size: 0.75rem;
        color: var(--muted);
        white-space: nowrap;
        overflow: hidden;
        text-overflow: ellipsis;
    }

    .status.warn {
        /* A warning is shown whole, however long the address. */
        white-space: normal;
        overflow-wrap: anywhere;
        color: var(--text);
        background: color-mix(in srgb, var(--danger) 16%, var(--surface));
    }
</style>
