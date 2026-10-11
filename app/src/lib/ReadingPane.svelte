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

    /** The message's links, in order. In the frame each is known only by its place in this list. */
    let links: string[] = [];
    /** Bumped to draw the message again after a link has been followed. */
    let drawn = $state(0);
    /** How far down the message was scrolled, to go back there after it is drawn again. */
    let scrolled = 0;
    let goBackTo = 0;
    let watching: ReturnType<typeof setInterval> | undefined;

    // The message is shown in a frame that cannot run scripts, submit forms
    // or open windows. It has been cleaned by the core before it gets here;
    // the frame is the second line, not the first.
    //
    // Nothing of ours can run in that frame either: WebKit will not call an
    // event listener in a document that may not run scripts, whoever added
    // it. So links are handled from out here. Each link is pointed at a
    // blank page that names its place in `links`; following one takes the
    // frame to that page, which this pane sees as the frame loading, and the
    // real address is then handed to the core. The address a message gives
    // for a link never becomes somewhere the frame goes.
    const page = $derived.by(() => {
        if (!opened) return "";
        const body = new DOMParser().parseFromString(opened.body.content, "text/html").body;
        links = [...body.querySelectorAll("a[href]")].map((link, place) => {
            const address = link.getAttribute("href") ?? "";
            link.setAttribute("href", `about:blank#${place}`);
            link.setAttribute("data-place", String(place));
            link.removeAttribute("target");
            return address;
        });

        return `<!doctype html><html><head><meta charset="utf-8"><style>
            :root { color-scheme: light dark; }
            body { margin: 16px; font: 14px/1.5 system-ui, sans-serif; overflow-wrap: anywhere; }
            img { max-width: 100%; height: auto; }
            table { max-width: 100%; }
            pre { white-space: pre-wrap; font: inherit; }
            blockquote { margin: 0 0 0 4px; padding-left: 12px; border-left: 3px solid #8886; }
            </style></head><body>${body.innerHTML}</body></html>`;
    });

    /** True when a link's own words look like an address other than the one it goes to. */
    function looksLikeAnotherAddress(words: string, address: string): boolean {
        words = words.trim();
        if (!/^(https?:\/\/|www\.)\S+$/i.test(words)) return false;
        try {
            const shown = new URL(words.startsWith("www.") ? `https://${words}` : words).hostname;
            return shown.replace(/^www\./, "") !== new URL(address).hostname.replace(/^www\./, "");
        } catch {
            return true;
        }
    }

    /** The frame has loaded something: the message, or the blank page a followed link leads to. */
    function loaded() {
        const view = frame?.contentWindow;
        if (!view) return;
        let at = "";
        try {
            at = view.location.href;
        } catch {
            // Somewhere we cannot read is not somewhere we sent it. Draw the message again.
            at = "elsewhere";
        }

        if (at.startsWith("about:blank#")) {
            const address = links[Number(at.slice("about:blank#".length))];
            if (address) onOpenLink(address);
        }
        if (at !== "about:srcdoc") {
            goBackTo = scrolled;
            drawn += 1;
        } else if (goBackTo > 0) {
            view.scrollTo(0, goBackTo);
            goBackTo = 0;
        }
    }

    // While the pointer or the keyboard is in the message, look a few times
    // a second at which link it is on, to show where that link really goes.
    function watch() {
        clearInterval(watching);
        watching = setInterval(() => {
            const view = frame?.contentWindow;
            const over = frame?.contentDocument?.querySelector<HTMLAnchorElement>("a[data-place]:hover, a[data-place]:focus");
            const address = over ? links[Number(over.dataset.place)] : undefined;
            hover = address ?? null;
            mismatch = over && address ? looksLikeAnotherAddress(over.textContent ?? "", address) : false;
            if (view) scrolled = view.scrollY;
        }, 120);
    }

    function unwatch() {
        clearInterval(watching);
        hover = null;
        mismatch = false;
    }

    $effect(() => unwatch);

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

        {#key `${opened.message.id} ${imagesShown} ${drawn}`}
            <!-- The pointer and focus are only watched to show where a link goes;
                 the message itself is reached and read in the ordinary way. -->
            <!-- svelte-ignore a11y_no_static_element_interactions -->
            <iframe
                bind:this={frame}
                title="Message body"
                sandbox="allow-same-origin"
                srcdoc={page}
                onload={loaded}
                onmouseenter={watch}
                onmouseleave={unwatch}
                onfocus={watch}
                onblur={unwatch}
            ></iframe>
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
