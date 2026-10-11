<script lang="ts">
    import { type Attachment, type Category, type FlagStatus, type Folder, type Message, type Opened, categoryColour, dueSaid, flagDays, size, when, who } from "./mail";

    interface Props {
        opened: Opened | null;
        loading: boolean;
        problem: string | null;
        /** True once the person has asked for this message's remote images. */
        imagesShown: boolean;
        /** The conversation this message is part of, oldest first. Empty when it is on its own. */
        thread: Message[];
        onRead: (message: Message) => void;
        attachments: Attachment[];
        /** What became of the last attachment saved: where it went, or why it did not. */
        saved: string | null;
        /** Save to the downloads folder, or with `ask` to a place chosen in a dialog. */
        onSave: (attachment: Attachment, ask?: boolean) => void;
        /** Start an answer to the message. */
        onWrite: (kind: "reply" | "reply_all" | "forward" | "forward_as_attachment") => void;
        onSetRead: (isRead: boolean) => void;
        /** `due` is a day, `YYYY-MM-DD`; empty for no day; left out to leave the day as it is. */
        onSetFlag: (status: FlagStatus, due?: string) => void;
        /** The mailbox's categories, to choose from. */
        categories: Category[];
        /** Give the message exactly these categories. */
        onCategorise: (names: string[]) => void;
        /** Every folder, to move the message to one. */
        folders: Folder[];
        /** The standard folder the message is in, if it is in one. */
        within: string | null;
        onDelete: () => void;
        /** Delete for good. Offered only where a message has already been deleted once. */
        onPurge: () => void;
        onJunk: () => void;
        onNotJunk: () => void;
        onArchive: () => void;
        onMove: (folder: Folder) => void;
        /** Always show this sender's remote pictures, or stop. */
        onTrust: (address: string, trusted: boolean) => void;
        onShowImages: () => void;
        onOpenLink: (url: string) => void;
    }

    let { opened, loading, problem, imagesShown, thread, onRead, attachments, saved, onSave, onTrust, onWrite, onSetRead, onSetFlag, categories, onCategorise, folders, within, onDelete, onPurge, onJunk, onNotJunk, onArchive, onMove, onShowImages, onOpenLink }: Props = $props();
    let frame = $state<HTMLIFrameElement>();
    let dayMenu = $state<HTMLDetailsElement>();

    function due(day: string) {
        // Closed outright: the menu is the browser's own, and is not left
        // open behind a choice made from the date box.
        if (dayMenu) dayMenu.open = false;
        onSetFlag("flagged", day);
    }
    /** The message being asked about, once "Delete permanently" has been pressed once. */
    let sureOf = $state<string | null>(null);
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
        {#if thread.length > 1}
            <ol class="thread" aria-label="Conversation">
                {#each thread as each (each.id)}
                    <li>
                        <button type="button" class:unread={!each.is_read} aria-current={each.id === opened.message.id} onclick={() => onRead(each)}>
                            <span class="who">{who(each.from)}</span>
                            <span class="said">{each.preview}</span>
                            <span class="muted">{when(each.received_at)}</span>
                        </button>
                    </li>
                {/each}
            </ol>
        {/if}

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
            <div class="actions" role="toolbar" aria-label="Message actions">
                <button type="button" title="Ctrl+R" onclick={() => onWrite("reply")}>Reply</button>
                <button type="button" title="Ctrl+Shift+R" onclick={() => onWrite("reply_all")}>Reply all</button>
                <button type="button" title="Ctrl+F" onclick={() => onWrite("forward")}>Forward</button>
                <button type="button" title="Forward this message whole, as an attached file" onclick={() => onWrite("forward_as_attachment")}>Forward as attachment</button>
                {#if within !== "deleteditems"}
                    <button type="button" title="Delete" onclick={onDelete}>Delete</button>
                {:else if sureOf === opened.message.id}
                    <button type="button" class="danger" onclick={() => ((sureOf = null), onPurge())}>Delete for good? It cannot be undone</button>
                    <button type="button" onclick={() => (sureOf = null)}>Keep</button>
                {:else}
                    <button type="button" onclick={() => (sureOf = opened.message.id)}>Delete permanently</button>
                {/if}
                {#if within === "junkemail"}
                    <button type="button" onclick={onNotJunk}>Not junk</button>
                {:else if within !== "deleteditems" && within !== "sentitems" && within !== "drafts" && folders.some((f) => f.well_known === "junkemail")}
                    <button type="button" onclick={onJunk}>Junk</button>
                {/if}
                {#if within !== "archive" && folders.some((f) => f.well_known === "archive")}
                    <button type="button" title="Backspace" onclick={onArchive}>Archive</button>
                {/if}
                <select
                    aria-label="Move to folder"
                    value=""
                    onchange={(event) => {
                        const to = folders.find((f) => f.id === event.currentTarget.value);
                        event.currentTarget.value = "";
                        if (to) onMove(to);
                    }}
                >
                    <option value="" disabled>Move to…</option>
                    {#each folders.filter((f) => f.id !== opened.message.folder_id) as each (each.id)}
                        <option value={each.id}>{"\u00a0\u00a0".repeat(each.depth)}{each.name}</option>
                    {/each}
                </select>
                {#if opened.message.is_read}
                    <button type="button" title="Ctrl+U" onclick={() => onSetRead(false)}>Mark as unread</button>
                {:else}
                    <button type="button" title="Ctrl+Q" onclick={() => onSetRead(true)}>Mark as read</button>
                {/if}
                {#if opened.message.flag_status === "flagged"}
                    <button type="button" onclick={() => onSetFlag("complete")}>Mark complete</button>
                    <button type="button" title="Insert" onclick={() => onSetFlag("notFlagged")}>Clear flag</button>
                {:else if opened.message.flag_status === "complete"}
                    <button type="button" onclick={() => onSetFlag("notFlagged")}>Clear flag</button>
                {:else}
                    <button type="button" title="Insert" onclick={() => onSetFlag("flagged")}>Flag</button>
                {/if}
                {#if opened.message.flag_status !== "complete"}
                    <details class="menu" bind:this={dayMenu}>
                        <summary>{opened.message.flag_status === "flagged" ? "Due…" : "Flag for…"}</summary>
                        <ul aria-label="Follow up by">
                            {#each flagDays() as each (each.label)}
                                <li><button type="button" class="plain" onclick={() => due(each.day)}>{each.label} <span class="muted">{dueSaid(each.day)}</span></button></li>
                            {/each}
                            <li>
                                <label>On <input type="date" aria-label="Another day" onchange={(event) => event.currentTarget.value && due(event.currentTarget.value)} /></label>
                            </li>
                            {#if opened.message.flag_due}
                                <li><button type="button" class="plain" onclick={() => due("")}>No day</button></li>
                            {/if}
                        </ul>
                    </details>
                {/if}
                {#if categories.length > 0}
                    <details class="menu">
                        <summary>Categories</summary>
                        <ul aria-label="Categories">
                            {#each categories as each (each.name)}
                                {@const has = (opened.message.categories ?? []).includes(each.name)}
                                <li>
                                    <label>
                                        <input
                                            type="checkbox"
                                            checked={has}
                                            onchange={() =>
                                                onCategorise(has ? opened.message.categories.filter((n) => n !== each.name) : [...(opened.message.categories ?? []), each.name])}
                                        />
                                        <span class="dot" style:background={categoryColour(each.color)}></span>
                                        {each.name}
                                    </label>
                                </li>
                            {/each}
                        </ul>
                    </details>
                {/if}
                {#if opened.message.flag_status === "flagged"}
                    <span class="flagged">🚩 {opened.message.flag_due ? `Due ${dueSaid(opened.message.flag_due)}` : "Flagged"}</span>
                {/if}
                {#if opened.message.flag_status === "complete"}<span class="muted">✓ Complete</span>{/if}
            </div>
            {#if (opened.message.categories ?? []).length > 0}
                <p class="chips" aria-label="This message's categories">
                    {#each opened.message.categories as name (name)}
                        <span class="chip"><span class="dot" style:background={categoryColour(categories.find((c) => c.name === name)?.color)}></span>{name}</span>
                    {/each}
                </p>
            {/if}
        </header>

        {#if attachments.length > 0}
            <ul class="attachments" aria-label="Attachments">
                {#each attachments as each (each.id)}
                    <li>
                        {#if each.kind === "reference"}
                            <span class="attachment" title="A link to a file kept elsewhere. It cannot be saved from here.">
                                🔗 {each.name || "Linked file"}
                            </span>
                        {:else}
                            <button type="button" class="attachment" title="Save to your downloads folder" onclick={() => onSave(each)}>
                                📎 {each.name || "Attachment"} <span class="muted">{size(each.size)}</span>
                            </button>
                            <button type="button" class="attachment as" aria-label="Save {each.name || 'attachment'} as…" title="Choose where to save it" onclick={() => onSave(each, true)}>
                                Save as…
                            </button>
                        {/if}
                    </li>
                {/each}
            </ul>
        {/if}
        {#if saved}
            <p class="note" role="status">{saved}</p>
        {/if}

        {#if opened.body.blocked_remote_content > 0}
            <p class="note" role="status">
                {#if imagesShown}
                    {opened.body.blocked_remote_content === 1 ? "1 picture" : `${opened.body.blocked_remote_content} pictures`} could not be fetched.
                {:else}
                    {opened.body.blocked_remote_content === 1 ? "1 remote picture was" : `${opened.body.blocked_remote_content} remote pictures were`}
                    not loaded, so the sender cannot tell you opened this.
                    <button type="button" onclick={onShowImages}>Show pictures</button>
                    {#if opened.message.from.address}
                        {@const address = opened.message.from.address}
                        <button type="button" onclick={() => onTrust(address, true)}>Always show from {address}</button>
                    {/if}
                {/if}
            </p>
        {/if}
        {#if opened.sender_trusted && opened.message.from.address}
            {@const address = opened.message.from.address}
            <p class="trusted muted">
                Pictures from {address} are always shown.
                <button type="button" onclick={() => onTrust(address, false)}>Stop</button>
            </p>
        {/if}

        {#if opened.body.format === "text"}
            <!-- Plain words need no frame: they are put on the page as text and nothing else. -->
            <pre class="plain" aria-label="Message body">{opened.body.content}</pre>
        {:else}
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
        {/if}

        <p class="status" class:warn={mismatch} aria-live="polite">
            {#if hover}{mismatch ? "This link goes somewhere other than it says: " : ""}{hover}{/if}
        </p>
    {/if}
</section>

<style>
    .actions {
        display: flex;
        flex-wrap: wrap;
        align-items: center;
        gap: 0.4rem;
        margin-top: 0.5rem;
        font-size: 0.85rem;
    }

    .actions button,
    .actions select {
        padding: 0.2rem 0.6rem;
        font-size: 0.85rem;
    }

    .flagged {
        color: var(--danger);
    }

    .dot {
        display: inline-block;
        width: 0.65rem;
        height: 0.65rem;
        border-radius: 50%;
    }

    .chips {
        display: flex;
        flex-wrap: wrap;
        gap: 0.35rem;
        margin: 0.5rem 0 0;
    }

    .chip {
        display: inline-flex;
        align-items: center;
        gap: 0.35rem;
        padding: 0.1rem 0.55rem;
        border: 1px solid var(--line);
        border-radius: 999px;
        font-size: 0.8rem;
    }

    .menu {
        position: relative;
    }

    .menu summary {
        list-style: none;
        cursor: default;
        padding: 0.2rem 0.6rem;
        border: 1px solid var(--line);
        border-radius: 0.4rem;
        background: var(--surface);
    }

    .menu summary::-webkit-details-marker {
        display: none;
    }

    .menu ul {
        position: absolute;
        z-index: 5;
        top: 100%;
        left: 0;
        min-width: 12rem;
        max-height: 16rem;
        overflow-y: auto;
        margin: 0.2rem 0 0;
        padding: 0.3rem;
        list-style: none;
        background: var(--surface);
        border: 1px solid var(--line);
        border-radius: 0.4rem;
        box-shadow: 0 6px 18px color-mix(in srgb, CanvasText 18%, transparent);
    }

    .menu .plain {
        width: 100%;
        display: flex;
        justify-content: space-between;
        gap: 1rem;
        border: 0;
        background: none;
        padding: 0.25rem 0.4rem;
        text-align: left;
    }

    .menu .plain:hover {
        background: var(--raised);
    }

    .menu label {
        display: flex;
        align-items: center;
        gap: 0.4rem;
        padding: 0.25rem 0.4rem;
        white-space: nowrap;
    }

    .danger {
        color: var(--danger);
        border-color: var(--danger);
    }

    .plain {
        flex: 1;
        min-height: 0;
        margin: 0;
        padding: 1rem;
        overflow: auto;
        white-space: pre-wrap;
        overflow-wrap: anywhere;
        font: inherit;
        font-size: 0.9rem;
    }

    .trusted {
        flex: none;
        margin: 0;
        padding: 0.3rem 1rem;
        font-size: 0.8rem;
    }

    .trusted button,
    .note button {
        margin-left: 0.4rem;
    }

    .attachments {
        flex: none;
        margin: 0;
        padding: 0.5rem 1rem;
        list-style: none;
        display: flex;
        flex-wrap: wrap;
        gap: 0.4rem;
        border-bottom: 1px solid var(--line);
    }

    .attachments li {
        display: flex;
    }

    .attachments li .attachment:first-child:not(:last-child) {
        border-right: 0;
        border-radius: 0.4rem 0 0 0.4rem;
    }

    .attachment.as {
        border-radius: 0 0.4rem 0.4rem 0;
        color: var(--muted);
    }

    .attachment {
        display: inline-block;
        max-width: 20rem;
        padding: 0.25rem 0.6rem;
        border: 1px solid var(--line);
        border-radius: 0.4rem;
        background: var(--raised);
        font-size: 0.85rem;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    }

    .thread {
        flex: none;
        margin: 0;
        padding: 0.25rem;
        list-style: none;
        max-height: 30%;
        overflow-y: auto;
        border-bottom: 1px solid var(--line);
        background: var(--raised);
    }

    .thread button {
        width: 100%;
        display: grid;
        grid-template-columns: 9rem minmax(0, 1fr) auto;
        gap: 0.75rem;
        padding: 0.3rem 0.5rem;
        border: 0;
        background: none;
        text-align: left;
        font-size: 0.85rem;
    }

    .thread button[aria-current="true"] {
        background: color-mix(in srgb, var(--accent) 16%, var(--surface));
    }

    .thread button.unread .who {
        font-weight: 700;
    }

    .thread span {
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    }

    .said {
        color: var(--muted);
    }

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
