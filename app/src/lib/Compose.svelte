<script lang="ts">
    import { Bold, Image, Italic, Link, List, ListOrdered, Paperclip, Send, Underline, X } from "@lucide/svelte";
    import { chooseFiles, say } from "./commands";
    import Recipients from "./Recipients.svelte";
    import { type Draft, type DraftFile, type Person, attachContent, attachFile, base64Of, discardDraft, editDraft, readFile, removeFile, sendDraft, size } from "./mail";

    interface Props {
        accountId: string;
        draft: Draft;
        /** The addresses it may go out under: the mailbox's own first, then any it may send as. One alone is not offered. */
        fromChoices?: string[];
        /** The draft was sent, thrown away, or put aside to come back to. */
        onDone: (how: "sent" | "discarded" | "closed") => void;
        /** What is written has just been kept, so a list of drafts can show it. */
        onKept: () => void;
    }

    let { accountId, draft, fromChoices = [], onDone, onKept }: Props = $props();

    // svelte-ignore state_referenced_locally
    let from = $state(draft.from ?? fromChoices[0] ?? "");

    /** Kept at once: who it is from is not something to lose to a slow save. */
    async function chooseFrom(address: string) {
        from = address;
        try {
            await editDraft(accountId, draft.id, { from: address === fromChoices[0] ? "" : address });
        } catch (failure) {
            problem = failure && typeof failure === "object" && "message" in failure ? String(failure.message) : String(failure);
        }
    }

    /** People as they are typed: `Sam Taylor <sam@example.test>, jo@example.test`. */
    const typed = (people: Person[]) => people.map((p) => (p.name && p.name !== p.address ? `${p.name} <${p.address}>` : p.address)).join(", ");
    const people = (text: string): Person[] =>
        text
            .split(/[,;\n]/)
            .map((each) => each.trim())
            .filter(Boolean)
            .map((each) => {
                const named = each.match(/^(.*)<([^<>]*)>$/);
                return named ? { name: named[1].trim().replace(/^"|"$/g, "") || null, address: named[2].trim() } : { name: null, address: each };
            });

    // The draft is this component's to edit from here on: a new draft is a
    // new component (it is keyed by the draft's id), not new props.
    // svelte-ignore state_referenced_locally
    const start = draft;
    let to = $state(typed(start.to));
    let cc = $state(typed(start.cc));
    let bcc = $state(typed(start.bcc));
    let subject = $state(start.subject);
    let body = $state(start.body);
    let format = $state<Draft["format"]>(start.format ?? "text");
    let editor = $state<HTMLDivElement>();

    const escape = (text: string) => text.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
    /** Plain words as the paragraphs a formatted message is made of. */
    const paragraphs = (text: string) =>
        text
            .split(/\n{2,}/)
            .map((each) => `<p>${escape(each).replace(/\n/g, "<br>") || "<br>"}</p>`)
            .join("");

    // The formatted body is put into the editor once, when it appears. It
    // comes from the core already cleaned, and after this the editor's own
    // contents are the truth, read back out as they change.
    $effect(() => {
        if (editor && editor.dataset.filled !== start.id) {
            editor.dataset.filled = start.id;
            editor.innerHTML = body;
            showPictures(editor);
        }
    });

    /**
     * A picture within the message is kept as `cid:` and a name, which is how
     * it is sent. That shows nothing here, so each is given its picture to
     * show, and keeps its name beside it to be written back by.
     */
    function showPictures(within: HTMLElement) {
        for (const image of within.querySelectorAll<HTMLImageElement>('img[src^="cid:"]')) {
            const name = image.getAttribute("src")?.slice(4) ?? "";
            const file = files.find((each) => each.content_id === name);
            image.dataset.cid = name;
            image.removeAttribute("src");
            if (!file) continue;
            readFile(accountId, start.id, file.id)
                .then((got) => (image.src = `data:${got.content_type};base64,${got.content_base64}`))
                .catch(() => {});
        }
    }

    /** The editor's contents as they are kept: each picture by its name again, not by its contents. */
    function written(): string {
        if (!editor) return body;
        const copy = editor.cloneNode(true) as HTMLElement;
        for (const image of copy.querySelectorAll<HTMLImageElement>("img")) {
            const name = image.dataset.cid;
            // A picture that is not one of the draft's own is not sent.
            if (!name) image.remove();
            else {
                image.setAttribute("src", `cid:${name}`);
                image.removeAttribute("data-cid");
            }
        }
        return copy.innerHTML;
    }

    function edited() {
        if (editor) body = written();
    }

    let picker = $state<HTMLInputElement>();
    /** Where the cursor was when the picture was asked for, since choosing a file takes the focus. */
    let placed: Range | null = null;

    function choosePicture() {
        const selection = window.getSelection();
        placed = selection && selection.rangeCount > 0 && editor?.contains(selection.anchorNode) ? selection.getRangeAt(0).cloneRange() : null;
        picker?.click();
    }

    /** Put pictures within the message, where the cursor was. */
    async function addPictures(chosen: FileList | File[] | null) {
        for (const file of chosen ?? []) {
            try {
                const content = await base64Of(file);
                const kept = await attachContent(accountId, start.id, file.name, content, true);
                files = [...files, kept];
                editor?.focus();
                if (placed) {
                    const selection = window.getSelection();
                    selection?.removeAllRanges();
                    selection?.addRange(placed);
                }
                document.execCommand("insertHTML", false, `<img src="data:${kept.content_type};base64,${content}" data-cid="${kept.content_id}" alt="">`);
                placed = null;
                edited();
                changed();
            } catch (failure) {
                problem = say(failure);
            }
        }
        if (picker) picker.value = "";
    }

    /** Pictures that are no longer in the message are no longer its files. */
    async function dropUnused() {
        for (const file of files.filter((each) => each.content_id && !body.includes(`cid:${each.content_id}`))) {
            await removeFile(accountId, start.id, file.id).catch(() => {});
            files = files.filter((each) => each.id !== file.id);
        }
    }

    const MOST = 25 * 1024 * 1024;
    let over = $state(false);

    /** Files dropped on the message are attached to it. */
    async function dropped(event: DragEvent) {
        event.preventDefault();
        over = false;
        for (const file of event.dataTransfer?.files ?? []) {
            if (file.size > MOST) {
                problem = `\`${file.name}\` is over 25 MB, which is more than most mail systems accept. Share it as a link instead.`;
                continue;
            }
            try {
                files = [...files, await attachContent(accountId, start.id, file.name, await base64Of(file))];
                onKept();
            } catch (failure) {
                problem = say(failure);
            }
        }
    }

    /** Bold, a list and the like, on what is selected. */
    function style(command: string, value?: string) {
        editor?.focus();
        document.execCommand(command, false, value);
        edited();
        changed();
    }

    /** What is pasted comes in as plain words: formatting from elsewhere is not carried into the message. */
    function paste(event: ClipboardEvent) {
        event.preventDefault();
        document.execCommand("insertText", false, event.clipboardData?.getData("text/plain") ?? "");
    }

    let linking = $state(false);
    let linkTo = $state("");
    /** The selection as it was before the link box took the focus. */
    let chosen: Range | null = null;

    function startLink() {
        const selection = window.getSelection();
        chosen = selection && selection.rangeCount > 0 ? selection.getRangeAt(0).cloneRange() : null;
        linkTo = "https://";
        linking = true;
    }

    function addLink() {
        linking = false;
        const address = linkTo.trim();
        if (!/^(https?:\/\/|mailto:)\S+$/i.test(address) || !chosen) return;
        const selection = window.getSelection();
        selection?.removeAllRanges();
        selection?.addRange(chosen);
        // With nothing selected, the address itself is what is shown.
        if (chosen.collapsed) document.execCommand("insertHTML", false, `<a href="${escape(address).replace(/"/g, "&quot;")}">${escape(address)}</a>`);
        else document.execCommand("createLink", false, address);
        edited();
        changed();
    }

    function switchFormat(plain: boolean) {
        if (plain) {
            body = editor?.innerText.replace(/\n{3,}/g, "\n\n").trimEnd() ?? body;
            format = "text";
        } else {
            body = paragraphs(body);
            format = "html";
        }
        try {
            localStorage.setItem("brolga.compose.plain", plain ? "1" : "0");
        } catch {
            // It holds for this message, at least.
        }
        changed();
    }
    let copies = $state(start.cc.length > 0 || start.bcc.length > 0);
    let files = $state<DraftFile[]>(start.files ?? []);

    /** Choose files with the system's dialog and attach each. One that cannot be attached is said, and the rest still are. */
    /** True while the file dialog is open, so a second click cannot open a second one. */
    let choosing = $state(false);

    async function attach() {
        if (choosing) return;
        choosing = true;
        try {
            const paths = await chooseFiles().catch(() => []);
            for (const path of paths) {
                try {
                    files = [...files, await attachFile(accountId, start.id, path)];
                    onKept();
                } catch (failure) {
                    problem = say(failure);
                }
            }
        } finally {
            choosing = false;
        }
    }

    async function detach(file: DraftFile) {
        try {
            await removeFile(accountId, start.id, file.id);
            files = files.filter((each) => each.id !== file.id);
            onKept();
        } catch (failure) {
            problem = say(failure);
        }
    }

    /** Whether what is on screen is what is kept, and if not why not. */
    let kept = $state<"kept" | "keeping" | "changed">("kept");
    let problem = $state<string | null>(start.state === "failed" ? `This was not sent. ${start.error ?? ""}` : null);
    let sending = $state(false);
    let sure = $state(false);
    let timer: ReturnType<typeof setTimeout> | undefined;
    let first = $state<HTMLInputElement>();
    let words = $state<HTMLTextAreaElement>();

    $effect(() => {
        // An answer is already addressed, so the place to start is the words.
        if (start.kind === "new" || start.to.length === 0) first?.focus();
        else (words ?? editor)?.focus();
        // Above the signature, where the writing goes.
        words?.setSelectionRange(0, 0);
    });

    function changed() {
        kept = "changed";
        sure = false;
        clearTimeout(timer);
        timer = setTimeout(keep, 800);
    }

    /** Keep what is written. True if it was kept. */
    async function keep(): Promise<boolean> {
        clearTimeout(timer);
        if (kept === "kept") return true;
        kept = "keeping";
        try {
            await editDraft(accountId, start.id, { to: people(to), cc: people(cc), bcc: people(bcc), subject, body, format });
            await dropUnused();
            if (kept === "keeping") kept = "kept";
            problem = null;
            onKept();
            return true;
        } catch (failure) {
            kept = "changed";
            problem = `Not kept yet. ${say(failure)}`;
            return false;
        }
    }

    async function send() {
        if (sending) return;
        sending = true;
        try {
            if (!(await keep())) return;
            await sendDraft(accountId, start.id);
            onDone("sent");
        } catch (failure) {
            problem = say(failure);
        } finally {
            sending = false;
        }
    }

    async function close() {
        if (await keep()) onDone("closed");
    }

    async function discard() {
        if (!sure) {
            sure = true;
            return;
        }
        clearTimeout(timer);
        try {
            await discardDraft(accountId, start.id);
            onDone("discarded");
        } catch (failure) {
            problem = say(failure);
        }
    }

    function keydown(event: KeyboardEvent) {
        if ((event.ctrlKey || event.metaKey) && event.key === "Enter") {
            event.preventDefault();
            send();
        } else if (event.key === "Escape") {
            event.preventDefault();
            close();
        }
    }

    const titles = { new: "New message", reply: "Reply", reply_all: "Reply to all", forward: "Forward" };
</script>

<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<form
    class="compose"
    class:over
    aria-label={titles[start.kind]}
    onsubmit={(event) => (event.preventDefault(), send())}
    onkeydown={keydown}
    oninput={changed}
    ondragover={(event) => {
        if (event.dataTransfer?.types.includes("Files")) {
            event.preventDefault();
            over = true;
        }
    }}
    ondragleave={() => (over = false)}
    ondrop={dropped}
>
    <div class="bar">
        <button type="submit" class="primary" disabled={sending} title="Ctrl+Enter"><Send size={16} aria-hidden="true" />{sending ? "Sending…" : "Send"}</button>
        <button type="button" disabled={choosing} onclick={attach}><Paperclip size={16} aria-hidden="true" />{choosing ? "Choosing…" : "Attach…"}</button>
        <button type="button" onclick={close} title="Escape">Close</button>
        <button type="button" class:danger={sure} onclick={discard}>{sure ? "Discard for good?" : "Discard"}</button>
        <span class="kept muted" role="status">{kept === "kept" ? "Draft kept on this computer" : kept === "keeping" ? "Keeping…" : ""}</span>
    </div>

    {#if problem}
        <p class="problem" role="alert">{problem}</p>
    {/if}

    {#if fromChoices.length > 1}
        <label>
            <span>From</span>
            <select aria-label="From" value={from} onchange={(event) => chooseFrom(event.currentTarget.value)}>
                {#each fromChoices as each (each)}
                    <option value={each}>{each}</option>
                {/each}
            </select>
        </label>
    {/if}
    <label>
        <span>To</span>
        <Recipients {accountId} label="To" bind:value={to} bind:input={first} placeholder="name@example.com, another@example.com" />
        {#if !copies}
            <button type="button" class="link" onclick={() => (copies = true)}>Cc Bcc</button>
        {/if}
    </label>
    {#if copies}
        <label><span>Cc</span><Recipients {accountId} label="Cc" bind:value={cc} /></label>
        <label><span>Bcc</span><Recipients {accountId} label="Bcc" bind:value={bcc} /></label>
    {/if}
    <label><span>Subject</span><input bind:value={subject} type="text" autocomplete="off" /></label>

    {#if files.some((file) => !file.content_id)}
        <ul class="files" aria-label="Attached files">
            {#each files.filter((file) => !file.content_id) as each (each.id)}
                <li>
                    📎 {each.name} <span class="muted">{size(each.size)}</span>
                    <button type="button" class="link" aria-label="Remove {each.name}" title="Remove" onclick={() => detach(each)}><X size={13} aria-hidden="true" /></button>
                </li>
            {/each}
        </ul>
    {/if}

    <div class="tools" role="toolbar" aria-label="Formatting">
        {#if format === "html"}
            <button type="button" title="Bold (Ctrl+B)" aria-label="Bold" onclick={() => style("bold")}><Bold size={15} aria-hidden="true" /></button>
            <button type="button" title="Italic (Ctrl+I)" aria-label="Italic" onclick={() => style("italic")}><Italic size={15} aria-hidden="true" /></button>
            <button type="button" title="Underline (Ctrl+U)" aria-label="Underline" onclick={() => style("underline")}><Underline size={15} aria-hidden="true" /></button>
            <button type="button" title="Bulleted list" aria-label="Bulleted list" onclick={() => style("insertUnorderedList")}><List size={15} aria-hidden="true" /></button>
            <button type="button" title="Numbered list" aria-label="Numbered list" onclick={() => style("insertOrderedList")}><ListOrdered size={15} aria-hidden="true" /></button>
            <button type="button" title="Link" aria-label="Link" onmousedown={(event) => event.preventDefault()} onclick={startLink}><Link size={15} aria-hidden="true" /></button>
            <button type="button" title="Put a picture in the message" aria-label="Picture" onmousedown={(event) => event.preventDefault()} onclick={choosePicture}><Image size={15} aria-hidden="true" /></button>
            <input bind:this={picker} class="unseen" type="file" accept="image/png,image/jpeg,image/gif,image/webp" multiple aria-label="Choose pictures" tabindex="-1" onchange={(event) => addPictures(event.currentTarget.files)} />
            <button type="button" title="Remove formatting" aria-label="Remove formatting" onclick={() => style("removeFormat")}>Clear</button>
        {/if}
        <label class="plainly"><input type="checkbox" checked={format === "text"} onchange={(event) => switchFormat(event.currentTarget.checked)} /> Plain text</label>
    </div>
    {#if linking}
        <!-- Not a form of its own: it is inside the message's. Enter adds the link and does not send. -->
        <div class="linking">
            <!-- svelte-ignore a11y_autofocus -->
            <input
                type="url"
                aria-label="Link address"
                bind:value={linkTo}
                autofocus
                onkeydown={(event) => {
                    if (event.key === "Enter") {
                        event.preventDefault();
                        event.stopPropagation();
                        addLink();
                    } else if (event.key === "Escape") {
                        event.preventDefault();
                        event.stopPropagation();
                        linking = false;
                    }
                }}
            />
            <button type="button" onclick={addLink}>Add link</button>
            <button type="button" onclick={() => (linking = false)}>Cancel</button>
        </div>
    {/if}

    {#if format === "html"}
        <div
            class="words"
            bind:this={editor}
            contenteditable="true"
            role="textbox"
            aria-multiline="true"
            aria-label="Message"
            tabindex="0"
            oninput={edited}
            onpaste={paste}
        ></div>
    {:else}
        <textarea bind:this={words} bind:value={body} aria-label="Message"></textarea>
    {/if}

    {#if start.kind !== "new"}
        <p class="muted quoted">The original message is added underneath when this is sent.</p>
    {/if}
</form>

<style>
    .compose {
        height: 100%;
        min-width: 0;
        display: flex;
        flex-direction: column;
    }

    .bar button,
    .tools button {
        display: inline-flex;
        align-items: center;
        gap: 0.35rem;
    }

    .bar {
        flex: none;
        display: flex;
        align-items: center;
        gap: 0.5rem;
        padding: 0.5rem 1rem;
        border-bottom: 1px solid var(--line);
    }

    .kept {
        margin-left: auto;
        font-size: 0.8rem;
    }

    label {
        flex: none;
        display: flex;
        align-items: center;
        gap: 0.5rem;
        padding: 0.25rem 1rem;
        border-bottom: 1px solid var(--line);
    }

    label span {
        width: 3.5rem;
        color: var(--muted);
        font-size: 0.85rem;
    }

    label input {
        flex: 1;
        min-width: 0;
        border: 0;
        background: none;
        padding: 0.35rem 0;
    }

    .files {
        flex: none;
        margin: 0;
        padding: 0.4rem 1rem;
        list-style: none;
        display: flex;
        flex-wrap: wrap;
        gap: 0.4rem;
        border-bottom: 1px solid var(--line);
    }

    .files li {
        padding: 0.15rem 0.3rem 0.15rem 0.6rem;
        border: 1px solid var(--line);
        border-radius: 0.4rem;
        background: var(--raised);
        font-size: 0.85rem;
    }

    .link {
        border: 0;
        background: none;
        color: var(--muted);
        font-size: 0.8rem;
    }

    .tools,
    .linking {
        flex: none;
        display: flex;
        flex-wrap: wrap;
        align-items: center;
        gap: 0.25rem;
        padding: 0.3rem 1rem;
        border-bottom: 1px solid var(--line);
    }

    .tools button {
        min-width: 2rem;
        padding: 0.15rem 0.5rem;
        font-size: 0.85rem;
    }

    .plainly {
        margin-left: auto;
        padding: 0;
        border: 0;
        font-size: 0.8rem;
        color: var(--muted);
        box-shadow: none !important;
    }

    .linking input {
        flex: 1;
        min-width: 10rem;
        padding: 0.25rem 0.5rem;
        font: inherit;
    }

    .words {
        flex: 1;
        min-height: 0;
        overflow-y: auto;
        padding: 1rem;
        font-size: 0.95rem;
        line-height: 1.5;
        overflow-wrap: anywhere;
    }

    .words:focus {
        outline: none;
    }

    .words :global(p) {
        margin: 0 0 0.75em;
    }

    .words :global(img) {
        max-width: 100%;
        height: auto;
    }

    .unseen {
        position: absolute;
        width: 1px;
        height: 1px;
        opacity: 0;
        pointer-events: none;
    }

    .compose.over {
        outline: 2px dashed var(--accent);
        outline-offset: -6px;
    }

    .words :global(a) {
        color: var(--accent-text);
    }

    textarea {
        flex: 1;
        min-height: 0;
        resize: none;
        border: 0;
        background: none;
        color: inherit;
        padding: 1rem;
        font: inherit;
        font-size: 0.95rem;
        line-height: 1.5;
    }

    textarea:focus,
    label input:focus {
        outline: none;
    }

    label:focus-within {
        box-shadow: inset 3px 0 0 var(--accent);
    }

    .problem {
        flex: none;
        margin: 0;
        padding: 0.5rem 1rem;
        background: color-mix(in srgb, var(--danger) 14%, var(--surface));
        font-size: 0.85rem;
    }

    .quoted {
        flex: none;
        margin: 0;
        padding: 0.4rem 1rem;
        border-top: 1px solid var(--line);
        font-size: 0.8rem;
    }

    .danger {
        color: var(--danger);
        border-color: var(--danger);
    }
</style>
