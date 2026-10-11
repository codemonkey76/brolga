<script lang="ts">
    import { chooseFiles, say } from "./commands";
    import { type Draft, type DraftFile, type Person, attachFile, discardDraft, editDraft, removeFile, sendDraft, size } from "./mail";

    interface Props {
        accountId: string;
        draft: Draft;
        /** The draft was sent, thrown away, or put aside to come back to. */
        onDone: (how: "sent" | "discarded" | "closed") => void;
        /** What is written has just been kept, so a list of drafts can show it. */
        onKept: () => void;
    }

    let { accountId, draft, onDone, onKept }: Props = $props();

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
    let copies = $state(start.cc.length > 0 || start.bcc.length > 0);
    let files = $state<DraftFile[]>(start.files ?? []);

    /** Choose files with the system's dialog and attach each. One that cannot be attached is said, and the rest still are. */
    async function attach() {
        const paths = await chooseFiles().catch(() => []);
        for (const path of paths) {
            try {
                files = [...files, await attachFile(accountId, start.id, path)];
                onKept();
            } catch (failure) {
                problem = say(failure);
            }
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
        else words?.focus();
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
            await editDraft(accountId, start.id, { to: people(to), cc: people(cc), bcc: people(bcc), subject, body });
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
<form class="compose" aria-label={titles[start.kind]} onsubmit={(event) => (event.preventDefault(), send())} onkeydown={keydown} oninput={changed}>
    <div class="bar">
        <button type="submit" class="primary" disabled={sending} title="Ctrl+Enter">{sending ? "Sending…" : "Send"}</button>
        <button type="button" onclick={attach}>Attach…</button>
        <button type="button" onclick={close} title="Escape">Close</button>
        <button type="button" class:danger={sure} onclick={discard}>{sure ? "Discard for good?" : "Discard"}</button>
        <span class="kept muted" role="status">{kept === "kept" ? "Draft kept on this computer" : kept === "keeping" ? "Keeping…" : ""}</span>
    </div>

    {#if problem}
        <p class="problem" role="alert">{problem}</p>
    {/if}

    <label>
        <span>To</span>
        <input bind:this={first} bind:value={to} type="text" autocomplete="off" spellcheck="false" placeholder="name@example.com, another@example.com" />
        {#if !copies}
            <button type="button" class="link" onclick={() => (copies = true)}>Cc Bcc</button>
        {/if}
    </label>
    {#if copies}
        <label><span>Cc</span><input bind:value={cc} type="text" autocomplete="off" spellcheck="false" /></label>
        <label><span>Bcc</span><input bind:value={bcc} type="text" autocomplete="off" spellcheck="false" /></label>
    {/if}
    <label><span>Subject</span><input bind:value={subject} type="text" autocomplete="off" /></label>

    {#if files.length > 0}
        <ul class="files" aria-label="Attached files">
            {#each files as each (each.id)}
                <li>
                    📎 {each.name} <span class="muted">{size(each.size)}</span>
                    <button type="button" class="link" aria-label="Remove {each.name}" title="Remove" onclick={() => detach(each)}>✕</button>
                </li>
            {/each}
        </ul>
    {/if}

    <textarea bind:this={words} bind:value={body} aria-label="Message"></textarea>

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
