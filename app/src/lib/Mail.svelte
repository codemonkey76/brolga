<script lang="ts">
    import { Archive, FilePen, Folder as FolderIcon, Inbox, OctagonAlert, Send, Star, Trash, X } from "@lucide/svelte";
    import type { Account } from "./accounts";
    import { type Asked, chooseSavePath, say } from "./commands";
    import { type Attachment, type Category, type Draft, type Edited, type FlagStatus, type Folder, type Message, type Opened, type Sort, type Starting, type View, SEARCH_MOST, canHold, createDraft, deletePermanently, editDraft, dismissRefused, flushOutbox, dueSaid, getConversation, conversationsInFolder, listAttachments, listCategories, listDrafts, listFolders, listMessages, moveFolder, moveMessages, openDraft, openLink, openMessage, refusedChanges, saveAttachment, searchMail, setCategories, setFavourite, setFlag, setRead, setTrustedSender, syncNow, undo } from "./mail";
    import Compose from "./Compose.svelte";
    import MessageList from "./MessageList.svelte";
    import MoveFolder from "./MoveFolder.svelte";
    import ReadingPane from "./ReadingPane.svelte";
    import Toolbar from "./Toolbar.svelte";
    import { type Answer, type Proposed, respond } from "./calendar";
    import { inDialog, typing } from "./typing";
    import { untrack } from "svelte";

    interface Props {
        accounts: Account[];
        /** False while another screen is shown. Mail is then kept as it is, out of sight, and does nothing. */
        shown?: boolean;
        /** Show a day in the calendar. */
        onCalendar: (day: Date) => void;
        /** A message a mailto: link asked to be started. */
        linked?: Asked | null;
        /** It has been started. */
        onLinked?: () => void;
    }

    let { accounts, shown = true, onCalendar, linked = null, onLinked }: Props = $props();

    const PAGE = 100;
    let accountId = $state<string | null>(null);
    let folders = $state<Folder[]>([]);
    let folderId = $state<string | null>(null);
    let total = $state(0);
    let rows = $state<(Message | undefined)[]>([]);
    /** Bumped whenever the list is replaced, so a slow answer for the old one is dropped. */
    let listing = 0;
    const asked = new Set<number>();
    /** The positions on screen, as the list last told of them. What is fetched again is what is near them. */
    let seen = { first: 0, last: 0 };

    let query = $state("");
    /** Whether a search looks in every folder or only the one shown. */
    let everywhere = $state(true);
    try {
        everywhere = localStorage.getItem("brolga.search.scope") !== "folder";
    } catch {
        // Every folder, then.
    }

    function scope(all: boolean) {
        everywhere = all;
        try {
            localStorage.setItem("brolga.search.scope", all ? "all" : "folder");
        } catch {
            // It holds for as long as the window is open.
        }
        if (query.trim()) search();
    }
    let searching = $state(false);
    /** The words last searched for. What is shown was found for these, whatever has been typed since. */
    let sought = $state("");
    /** The search found as many as it is asked for, so there may be more. */
    let capped = $state(false);
    /** Counts each search asked for, so that only the answer to the last is shown. */
    let seeking = 0;
    let opened = $state<Opened | null>(null);
    let selectedId = $state<string | null>(null);
    /** The message being read, which in a conversation need not be the one selected in the list. */
    let readingId = $state<string | null>(null);
    /** The lines chosen together, when there is more than one. `selectedId` is then the one the keys move from. */
    let picked = $state<string[]>([]);
    /** The whole conversation of the selected line, oldest first, when it has more than one message. */
    let thread = $state<Message[]>([]);
    /** Where a run of lines chosen with Shift begins. */
    let anchorId: string | null = null;
    const many = $derived(picked.length > 1);
    const chosen = $derived.by(() => {
        if (picked.length < 2) return [];
        const ids = new Set(picked);
        return rows.filter((row): row is Message => !!row && ids.has(row.id));
    });
    /**
     * The one message an action is for: the one being read. While it is
     * still opening the pane shows the one before it, so it is taken from its
     * line in the list, and what is done is done to what the keys have reached.
     */
    const current = $derived.by(() => {
        if (!readingId) return null;
        if (opened?.message.id === readingId) return opened.message;
        return rows.find((row) => row?.id === readingId) ?? thread.find((each) => each.id === readingId) ?? null;
    });
    /** What an action is for: the lines chosen, or else the message being read. */
    const targets = $derived(many ? chosen.map((each) => each.id) : current ? [current.id] : []);
    /** The core takes this many messages at a time. */
    const MOST = 500;
    /** The most lines chosen at once. */
    const MOST_PICKED = 5_000;

    /**
     * Do something to messages, as many at a time as the core takes. Every
     * part that changed anything can be undone. A part that fails stops it
     * there: `reached` is how many were done before it, and what was done
     * can still be undone.
     */
    async function inBatches(ids: string[], each: (some: string[]) => Promise<Edited>) {
        let changed = 0;
        let reached = 0;
        const undos: string[] = [];
        for (let at = 0; at < ids.length; at += MOST) {
            const some = ids.slice(at, at + MOST);
            try {
                const done = await each(some);
                changed += done.changed;
                if (done.changed > 0) undos.push(done.undo_id);
                reached += some.length;
            } catch (failure) {
                return { changed, undos, reached, failed: true, failure };
            }
        }
        return { changed, undos, reached, failed: false, failure: null };
    }

    /**
     * What was done, and to how many when it was more than one. Done to whole
     * conversations, it says how many conversations as well as messages.
     */
    const counted = (said: string, count: number, conversations = 0) =>
        conversations > 0 && count > conversations
            ? `${said}: ${conversations === 1 ? "1 conversation" : `${conversations.toLocaleString()} conversations`} (${count.toLocaleString()} messages)`
            : count > 1
              ? `${said}: ${count.toLocaleString()} messages`
              : said;

    /**
     * Which of these ids are lines of the conversation view: each stands for
     * every message of its conversation in this folder (TKT-01065). A message
     * opened within a conversation has no line of its own, and stands for
     * itself. Outside conversation view, and in a search, nothing is a line.
     */
    const linesOf = (ids: string[]) => {
        if (!view.conversations || searching) return [];
        const asked = new Set(ids);
        return rows.filter((row): row is Message => !!row && asked.has(row.id) && !!row.conversation_id);
    };

    /** How many conversations and messages an action on these lines covers, from the lines' own counts; null where none is a line. */
    const coverOf = (ids: string[]) => {
        const lines = linesOf(ids);
        if (lines.length === 0) return null;
        const messages = ids.length - lines.length + lines.reduce((sum, line) => sum + (line.conversation?.count ?? 1), 0);
        return { conversations: lines.length, messages };
    };
    /** What an action now would cover, for the toolbar to say before deleting for good. */
    const covers = $derived(coverOf(targets));

    /**
     * The messages an action on these ids is for, and how many conversations
     * that is. A line of the conversation view is every message of its
     * conversation in this folder, newest first; copies in other folders, such
     * as replies in Sent Items, are left alone. `flagged` keeps only those
     * with a flag, for clearing or completing one.
     */
    async function resolve(ids: string[], only: "all" | "flagged" = "all"): Promise<{ messages: Message[]; conversations: number }> {
        const known = (id: string) => rows.find((row) => row?.id === id) ?? thread.find((each) => each.id === id) ?? (opened?.message.id === id ? opened.message : undefined);
        const lines = linesOf(ids);
        const asLines = new Set(lines.map((line) => line.id));
        const messages: Message[] = ids.filter((id) => !asLines.has(id)).map((id) => known(id) ?? ({ id } as Message));
        const whole: Message[] = [];
        if (accountId && lines.length > 0) {
            const byFolder = new Map<string, string[]>();
            for (const line of lines) byFolder.set(line.folder_id, [...(byFolder.get(line.folder_id) ?? []), line.conversation_id as string]);
            for (const [folderId, conversations] of byFolder) {
                for (let at = 0; at < conversations.length; at += MOST) {
                    whole.push(...(await conversationsInFolder(accountId, folderId, conversations.slice(at, at + MOST))));
                }
            }
        }
        // Of a whole conversation, only those with a flag have one to take off; a message asked for by itself is as asked.
        const found = [...messages, ...(only === "flagged" ? whole.filter((each) => each.flag_status === "flagged") : whole)];
        return { messages: [...new Map(found.map((each) => [each.id, each])).values()], conversations: lines.length };
    }
    /** Why something stopped, and how far it had got if it had got anywhere. */
    const stopped = (failure: unknown, reached: number, of: number) =>
        reached > 0 ? `${reached.toLocaleString()} of ${of.toLocaleString()} messages were done before this went wrong. ${say(failure)}` : say(failure);

    /** Something is being done to messages. Until it has been, the keys and the toolbar start nothing else. */
    let working = $state(false);

    /** Do one thing to messages at a time. What is asked for while another is being done is left. */
    async function alone(what: () => Promise<void>) {
        if (working) return;
        working = true;
        changing = accountId;
        try {
            await what();
        } finally {
            working = false;
        }
    }

    /** What is attached to the message being read, apart from pictures within its body. */
    let attachments = $state<Attachment[]>([]);
    /** What became of the last attachment saved. */
    let saved = $state<string | null>(null);

    /** Save to the downloads folder, or first ask where. */
    async function save(attachment: Attachment, ask = false) {
        if (!accountId || !opened) return;
        saved = null;
        const messageId = opened.message.id;
        try {
            const path = ask ? await chooseSavePath(attachment.name || "attachment") : undefined;
            if (ask && !path) return;
            const result = await saveAttachment(accountId, messageId, attachment.id, path ?? undefined);
            // Said on the message it came from, or beside the list if another has been opened since.
            if (opened?.message.id === messageId) saved = `Saved to ${result.path}`;
            else tell(`Saved to ${result.path}`);
        } catch (failure) {
            if (opened?.message.id === messageId) saved = say(failure);
            else problem = say(failure);
        }
    }

    /** The messages being written on this computer, and those waiting to go. */
    let drafts = $state<Draft[]>([]);
    /** The one being written now, which takes the reading pane's place. */
    let writing = $state<Draft | null>(null);
    let notice = $state<string | null>(null);
    let noticeTimer: ReturnType<typeof setTimeout> | undefined;

    async function loadDrafts() {
        const of = accountId;
        if (!of) return;
        const found = await listDrafts(of).catch(() => null);
        if (found && accountId === of) drafts = found;
    }

    function tell(what: string) {
        clearTimeout(noticeTimer);
        notice = what;
        noticeTimer = setTimeout(() => (notice = null), 6_000);
    }

    /** Make the draft for a new message, or an answer to the one being read. Nothing if it cannot be. */
    async function begin(kind: Starting): Promise<Draft | null> {
        if (!accountId) return null;
        // An archive has no server to send through.
        if (account?.archive) {
            problem = "An archive can be read and searched, but nothing can be sent from it. Choose one of your accounts to write.";
            return null;
        }
        const about = kind === "new" ? undefined : current?.id;
        if (kind !== "new" && !about) return null;
        // Formatted unless the last message written was switched to plain text.
        let plain = false;
        try {
            plain = localStorage.getItem("brolga.compose.plain") === "1";
        } catch {
            // Formatted, then.
        }
        return createDraft(accountId, kind, plain ? "text" : "html", about);
    }

    /** A message is being started, so the keys for one pressed again do not start another. */
    let starting = false;

    /** Start a new message, or an answer to the one being read. */
    async function write(kind: Starting) {
        if (starting) return;
        starting = true;
        const of = accountId;
        try {
            const started = await begin(kind);
            if (!started || accountId !== of) return;
            writing = started;
            await loadDrafts();
        } catch (failure) {
            problem = say(failure);
        } finally {
            starting = false;
        }
    }

    // A mailto: link starts a new message with what it names filled in. The
    // words are put above the signature, as if they had been typed.
    $effect(() => {
        if (!linked || !accountId) return;
        const { to, cc, bcc, subject, body } = linked;
        onLinked?.();
        (async () => {
            try {
                const started = await begin("new");
                if (!started || !accountId) return;
                const escaped = body.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
                const words = !body ? started.body : started.format === "html" ? `<p>${escaped.replace(/\n/g, "<br>")}</p>${started.body}` : `${body}\n${started.body}`;
                writing = (await editDraft(accountId, started.id, { to, cc, bcc, subject, body: words })).draft;
                await loadDrafts();
            } catch (failure) {
                problem = say(failure);
            }
        })();
    });

    /**
     * Sending happens in the background. The drafts are looked at again
     * until this one has gone, or come back unsent, and then the list is
     * drawn again: a draft that has gone is no longer in Drafts.
     */
    async function afterSending(id: string) {
        const of = accountId;
        for (const wait of [1_500, 3_000, 6_000, 12_000]) {
            await new Promise((resolve) => setTimeout(resolve, wait));
            if (accountId !== of) return;
            await loadDrafts();
            if (!drafts.some((each) => each.id === id && each.state === "queued")) break;
        }
        if (accountId === of) await refresh();
    }

    async function doneWriting(how: "sent" | "discarded" | "closed") {
        const written = writing?.id;
        writing = null;
        // A line chosen while this was being written is opened now that there is somewhere to show it.
        const waiting = rows.find((row) => row?.id === selectedId);
        if (waiting && !waiting.is_draft && !many && readingId !== waiting.id) void open(waiting);
        if (how === "sent") tell("Sending. It will be in Sent Items once it has gone.");
        await loadDrafts();
        if (how === "sent" && written) void afterSending(written);
        // A draft put aside is saved to the mailbox now, so that other mail
        // programs have it; then the list is drawn again, for Drafts may have changed.
        if (accountId && how !== "sent") {
            flushOutbox(accountId)
                .then(refresh)
                .catch(() => {});
        }
    }

    /** The categories the mailbox has, to show each in its colour and to choose from. */
    let categories = $state<Category[]>([]);

    /** Give the messages a category, or take it from them. Each keeps whatever others it had. */
    async function categorise(name: string, on: boolean) {
        if (!accountId || targets.length === 0) return;
        const account = accountId;
        const asked = targets;
        await alone(async () => {
            const { messages: of, conversations } = await resolve(asked);
            // The core sets a message's categories whole, so those that end up alike are sent together.
            const alike = new Map<string, { names: string[]; ids: string[] }>();
            const now = new Map<string, string[]>();
            for (const each of of) {
                const had = each.categories ?? [];
                if (had.includes(name) === on) continue;
                const names = on ? [...had, name] : had.filter((n) => n !== name);
                now.set(each.id, names);
                const key = JSON.stringify(names);
                alike.set(key, { names, ids: [...(alike.get(key)?.ids ?? []), each.id] });
            }
            if (now.size === 0) return;
            const undos: string[] = [];
            /** Those it was done to, which is all of them unless a part failed. */
            const did = new Map<string, string[]>();
            let why: string | null = null;
            for (const { names, ids } of alike.values()) {
                const done = await inBatches(ids, (some) => setCategories(account, some, names));
                undos.push(...done.undos);
                for (const id of ids.slice(0, done.reached)) did.set(id, names);
                if (done.failed) {
                    why = stopped(done.failure, did.size, now.size);
                    break;
                }
            }
            try {
                const mine = opened && did.get(opened.message.id);
                if (opened && mine) opened = { ...opened, message: { ...opened.message, categories: mine } };
                if (searching) rows = rows.map((row) => (row && did.has(row.id) ? { ...row, categories: did.get(row.id) ?? [] } : row));
                else await refresh();
            } catch (failure) {
                why ??= say(failure);
            }
            const cleared = [...did.values()].every((names) => names.length === 0);
            if (undos.length > 0) offerUndo(counted(cleared ? "Categories cleared" : "Categories changed", did.size, why ? 0 : conversations), undos);
            if (why) problem = why;
        });
    }

    /** The last action, while it can still be taken back. */
    let undoable = $state<{ said: string; ids: string[] } | null>(null);
    /** The account whose messages are being changed, for an undo to be offered only while it is still the one shown. */
    let changing: string | null = null;
    let undoTimer: ReturnType<typeof setTimeout> | undefined;
    const UNDO_FOR = 10_000;

    /** Show a change that has been made: in the list, in the folder counts and on the message being read. */
    async function redraw(change: { is_read?: boolean; flag_status?: string; flag_due?: string | null }, ids: string[]) {
        if (opened && ids.includes(opened.message.id)) opened = { ...opened, message: { ...opened.message, ...change } };
        thread = thread.map((each) => (ids.includes(each.id) ? { ...each, ...change } : each));
        await relist();
        if (searching) rows = rows.map((row) => (row && ids.includes(row.id) ? { ...row, ...change } : row));
        else await refresh();
    }

    /**
     * Change some messages. The core makes the change at once and sends it
     * in the background, so there is nothing to wait for but the redraw.
     * `said` is offered with an Undo for ten seconds; without it the change
     * is made quietly, as reading a message marks it read, whatever else is
     * being done.
     */
    async function change(ids: string[], what: { is_read: boolean } | { flag_status: FlagStatus; due?: string }, said?: string) {
        if (!accountId || ids.length === 0) return;
        const account = accountId;
        const make = async () => {
            // A flag goes on a conversation's newest message, which is its line: one flag to a conversation.
            // Read state, and taking a flag off or marking it complete, is for the whole conversation.
            const setting = "flag_status" in what && what.flag_status === "flagged";
            const quiet = !said;
            const whole = setting || quiet ? { messages: ids.map((id) => ({ id }) as Message), conversations: 0 } : await resolve(ids, "is_read" in what ? "all" : "flagged");
            ids = whole.messages.map((each) => each.id);
            if (ids.length === 0) return;
            const done = await inBatches(ids, (some) => ("is_read" in what ? setRead(account, some, what.is_read) : setFlag(account, some, what.flag_status, what.due)));
            let why = done.failed ? stopped(done.failure, done.reached, ids.length) : null;
            // Drawn again even when a part failed, to show those it did reach.
            const did = ids.slice(0, done.reached);
            try {
                if ("is_read" in what) {
                    await redraw(what, did);
                } else {
                    // Only a flagged message has a day; one not mentioned keeps the day it had.
                    const day = what.flag_status !== "flagged" ? { flag_due: null } : what.due === undefined ? {} : { flag_due: what.due || null };
                    await redraw({ flag_status: what.flag_status, ...day }, did);
                }
            } catch (failure) {
                why ??= say(failure);
            }
            if (said && done.changed > 0) offerUndo(counted(said, did.length, done.failed ? 0 : whole.conversations), done.undos);
            if (why) problem = why;
        };
        if (said) await alone(make);
        else await make();
    }

    /** Fetch the list again as it now is: the folder's pages, or what the search finds. */
    async function again() {
        if (searching) await search(sought);
        else await refresh();
    }

    async function takeBack() {
        if (!accountId || !undoable) return;
        const account = accountId;
        const { said, ids } = undoable;
        await alone(async () => {
            clearTimeout(undoTimer);
            undoable = null;
            let left = ids;
            let why: string | null = null;
            try {
                for (const id of ids) {
                    await undo(account, id);
                    left = left.slice(1);
                }
            } catch (failure) {
                why = say(failure);
            }
            // Drawn again even when a part failed, to show what did come back.
            try {
                await again();
                await relist();
                // The message being read is drawn again from what the list now says of it.
                const now = rows.find((row) => row && row.id === opened?.message.id);
                if (opened && now) {
                    opened = {
                        ...opened,
                        message: { ...opened.message, is_read: now.is_read, flag_status: now.flag_status, flag_due: now.flag_due, categories: now.categories },
                    };
                }
            } catch (failure) {
                why ??= say(failure);
            }
            if (why) problem = why;
            // What was not taken back can still be.
            if (why && left.length > 0) offerUndo(said, left);
        });
    }

    const markRead = (isRead: boolean) => change(targets, { is_read: isRead }, isRead ? "Marked as read" : "Marked as unread");
    const flag = (status: FlagStatus, due?: string) =>
        change(targets, { flag_status: status, due }, status === "complete" ? "Marked complete" : status === "notFlagged" ? "Flag cleared" : due ? `Flagged for ${dueSaid(due)}` : "Flagged");

    function offerUndo(said: string, ids: string[]) {
        // Another account has been chosen since: its list is not where to undo this.
        if (changing !== accountId) return;
        clearTimeout(undoTimer);
        undoable = { said, ids };
        undoTimer = setTimeout(() => (undoable = null), UNDO_FOR);
    }

    /**
     * Move the lines chosen, or the message being read, to another folder.
     * Deleting and archiving are this. The next message in the list takes
     * their place, so a run of mail can be dealt with one key at a time.
     */
    async function moveTo(destination: string, said: string, ids = targets) {
        if (!accountId || ids.length === 0) return;
        const account = accountId;
        await alone(async () => {
            const at = place(ids);
            const { messages, conversations } = await resolve(ids);
            const all = messages.map((each) => each.id);
            const done = await inBatches(all, (some) => moveMessages(account, some, destination));
            let why = done.failed ? stopped(done.failure, done.reached, all.length) : null;
            try {
                await gone(all.slice(0, done.reached), at);
            } catch (failure) {
                why ??= say(failure);
            }
            if (done.changed > 0) offerUndo(counted(said, done.reached, done.failed ? 0 : conversations), done.undos);
            if (why) problem = why;
        });
    }

    /**
     * Where in the list the first of these is. One message of a conversation
     * being read has no line of its own: it is where the selected line is.
     */
    function place(ids: string[]) {
        const going = new Set(ids);
        const at = rows.findIndex((row) => row && going.has(row.id));
        return at >= 0 ? at : rows.findIndex((row) => row?.id === selectedId);
    }

    /**
     * Draw the list without what has left it, and open what is now where the
     * first of them was. Nothing is opened if what went was neither being
     * read nor the lines chosen: a line's own bin, or a part that failed.
     */
    async function gone(ids: string[], at: number) {
        if (!accountId) return;
        const going = new Set(ids);
        const kept = picked.filter((id) => !going.has(id));
        // Other lines are still chosen, or something else was being read and still is.
        const stays = kept.length > 1 || (!many && !!current && !going.has(current.id));
        picked = kept.length > 1 ? kept : [];
        if (searching) {
            rows = rows.filter((row) => !row || !going.has(row.id));
            total = rows.length;
        } else {
            await refresh();
        }
        await relist();
        if (stays) return;
        const next = at >= 0 ? rows[Math.min(at, total - 1)] : undefined;
        if (next) {
            // Not waited for: the next thing can be done to it while it opens.
            void open(next);
        } else {
            opened = null;
            selectedId = null;
            readingId = null;
            thread = [];
        }
    }

    /** What a button on a line of the list is for: every line chosen if it is one of them, or else that line alone. */
    const lineOf = (message: Message) => (many && picked.includes(message.id) ? targets : [message.id]);

    function flagLine(message: Message) {
        const ids = lineOf(message);
        const clear = message.flag_status === "flagged";
        void change(ids, { flag_status: clear ? "notFlagged" : "flagged" }, clear ? "Flag cleared" : "Flagged");
    }

    const removeLine = (message: Message) => moveTo("deleteditems", "Moved to Deleted Items", lineOf(message));

    /** Delete for good the lines chosen, or the message being read. There is no undo, so there is no offer of one. */
    async function purge() {
        const asked = targets;
        if (!accountId || asked.length === 0) return;
        const account = accountId;
        await alone(async () => {
            const at = place(asked);
            const { messages, conversations } = await resolve(asked);
            const ids = messages.map((each) => each.id);
            let reached = 0;
            let why: string | null = null;
            for (let from = 0; from < ids.length && !why; from += MOST) {
                const some = ids.slice(from, from + MOST);
                try {
                    await deletePermanently(account, some);
                    reached += some.length;
                } catch (failure) {
                    why = stopped(failure, reached, ids.length);
                }
            }
            try {
                await gone(ids.slice(0, reached), at);
            } catch (failure) {
                why ??= say(failure);
            }
            if (why) problem = why;
            else if (conversations > 0 && ids.length > conversations) tell(`${counted("Deleted permanently", ids.length, conversations)}.`);
            else tell(ids.length > 1 ? `${ids.length.toLocaleString()} messages deleted permanently.` : "Deleted permanently.");
        });
    }

    /** The standard folder the message being read is in, if it is in one. For several chosen, the one they are all in. */
    const within = $derived.by(() => {
        const from = many ? (chosen.every((each) => each.folder_id === chosen[0]?.folder_id) ? chosen[0]?.folder_id : undefined) : current?.folder_id;
        return folders.find((f) => f.id === from)?.well_known ?? null;
    });
    const remove = () => within !== "deleteditems" && moveTo("deleteditems", "Moved to Deleted Items");
    const junk = () => within !== "junkemail" && folders.some((f) => f.well_known === "junkemail") && moveTo("junkemail", "Moved to Junk Email");
    const notJunk = () => within === "junkemail" && moveTo("inbox", "Moved to the inbox");
    const archive = () => within !== "archive" && folders.some((f) => f.well_known === "archive") && moveTo("archive", "Archived");

    /** The changes Microsoft 365 would not take, in words, until the notice of them is closed. */
    let refusal = $state<string | null>(null);

    /** Tell of any change Microsoft 365 would not take. Each has already been put back. */
    async function tellOfRefusals() {
        const of = accountId;
        if (!of) return;
        const refused = await refusedChanges(of);
        if (accountId !== of) return;
        const first = refused[0];
        const others = refused.length > 1 ? `, and ${refused.length - 1} more` : "";
        refusal = first ? `Microsoft 365 would not change the ${first.change} of “${first.subject || "(No subject)"}”${others}, so it has been put back. ${first.error}` : null;
    }

    /** The notice has been read and closed: only now does the core forget what was refused. */
    function refusalSeen() {
        refusal = null;
        if (accountId) dismissRefused(accountId).catch((failure) => (problem = say(failure)));
    }

    /** How the list is laid out. This window's business only. */
    let view = $state<View>({ sort: "newest", conversations: false, plain: false, pane: "right" });
    const SORTS: Sort[] = ["newest", "oldest", "sender", "subject"];
    try {
        const kept: unknown = JSON.parse(localStorage.getItem("brolga.view") ?? "null");
        if (kept && typeof kept === "object") {
            const { sort, conversations, plain, pane } = kept as Partial<View>;
            view = {
                sort: SORTS.includes(sort as Sort) ? (sort as Sort) : "newest",
                conversations: conversations === true,
                plain: plain === true,
                pane: pane === "bottom" ? "bottom" : "right",
            };
        }
    } catch {
        // Nothing kept: newest first, a line for each message.
    }

    async function lay(next: View) {
        const relist = next.sort !== view.sort || next.conversations !== view.conversations;
        const reread = next.plain !== view.plain;
        view = next;
        try {
            localStorage.setItem("brolga.view", JSON.stringify(next));
        } catch {
            // It still holds for as long as the window is open.
        }
        if (reread && opened) void read(opened.message);
        if (relist && folderId && !searching) {
            reset();
            await need(0);
        }
    }

    /** Answer the invitation that is open. A failure is for the card to say. */
    async function answer(given: Answer, comment: string, organiserTold: boolean, proposed?: Proposed) {
        const meeting = opened?.meeting;
        if (!accountId || !opened || !meeting?.event_id) return;
        const id = opened.message.id;
        const result = await respond(accountId, meeting.event_id, given, comment, organiserTold, proposed);
        // A declined meeting has left the calendar: there is nothing more to answer.
        if (opened?.message.id === id) {
            opened = { ...opened, meeting: { ...meeting, response: result.response, event_id: given === "decline" ? null : meeting.event_id } };
        }
        tell(given === "decline" ? "Declined" : given === "accept" ? "Accepted" : "Accepted tentatively");
    }

    async function trust(address: string, trusted: boolean) {
        if (!accountId || !opened) return;
        try {
            await setTrustedSender(accountId, address, trusted);
            await read(opened.message);
        } catch (failure) {
            readingProblem = say(failure);
        }
    }
    let opening = $state(false);
    /** Counts each message asked for, so that only the last of them says the opening is over. */
    let reads = 0;
    let readingProblem = $state<string | null>(null);
    let imagesShown = $state(false);
    let syncing = $state(false);
    /** Why the last sync failed. It goes when a sync next works, or when it is dismissed. */
    let syncProblem = $state<string | null>(null);
    /** Why something else that was asked for could not be done. It stays until it is dismissed. */
    let problem = $state<string | null>(null);
    let searchBox = $state<HTMLInputElement>();

    /** A shape for each of the folders every mailbox has. */
    const FOLDER_ICONS: Record<string, typeof FolderIcon> = {
        inbox: Inbox,
        drafts: FilePen,
        sentitems: Send,
        deleteditems: Trash,
        archive: Archive,
        junkemail: OctagonAlert,
    };

    const account = $derived(accounts.find((a) => a.id === accountId) ?? null);
    const folder = $derived(folders.find((f) => f.id === folderId) ?? null);
    const favourites = $derived(folders.filter((f) => f.favourite));

    /** Folders closed up in the tree. Which ones is this window's business only. */
    let collapsed = $state<string[]>([]);
    const collapsedKey = (id: string) => `brolga.collapsed.${id}`;

    /** What was kept under a name, where it is a list of ids. */
    function keptIds(key: string): string[] {
        try {
            const kept: unknown = JSON.parse(localStorage.getItem(key) ?? "[]");
            if (Array.isArray(kept)) return kept.filter((each) => typeof each === "string");
        } catch {
            // Nothing kept, or nowhere to keep it: everything starts open.
        }
        return [];
    }

    function keep(key: string, ids: string[]) {
        try {
            localStorage.setItem(key, JSON.stringify(ids));
        } catch {
            // It still works for as long as the window is open.
        }
    }

    const without = (ids: string[], id: string) => (ids.includes(id) ? ids.filter((each) => each !== id) : [...ids, id]);

    function recallCollapsed() {
        collapsed = accountId ? keptIds(collapsedKey(accountId)) : [];
    }

    function toggle(id: string) {
        collapsed = without(collapsed, id);
        if (accountId) keep(collapsedKey(accountId), collapsed);
    }

    /** A tree as it shows: nothing inside a closed folder, and which folders have something inside. */
    function treeOf(all: Folder[], closed: string[]) {
        const shown: { folder: Folder; parent: boolean; open: boolean }[] = [];
        let closedAt: number | null = null;
        all.forEach((each, index) => {
            if (closedAt !== null && each.depth > closedAt) return;
            const open = !closed.includes(each.id);
            closedAt = open ? null : each.depth;
            shown.push({ folder: each, parent: (all[index + 1]?.depth ?? 0) > each.depth, open });
        });
        return shown;
    }

    const tree = $derived(treeOf(folders, collapsed));

    async function pin(each: Folder) {
        if (!accountId) return;
        try {
            await setFavourite(accountId, each.id, !each.favourite);
            await relist();
        } catch (failure) {
            problem = say(failure);
        }
    }

    /** A folder of one's own making, in a mailbox that is at Microsoft 365, can be put inside another. */
    const movable = (each: Folder) => !each.well_known && !account?.archive;

    /** The folder being dragged, and the one it is over that could take it. */
    let dragging = $state<Folder | null>(null);
    let over = $state<string | null>(null);
    /** The folder to be moved, once dropped or picked from its menu: into where it was dropped, or to where is then chosen. */
    let moving = $state<{ folder: Folder; into: Folder | null; busy: boolean; problem: string | null } | null>(null);
    /** The menu for a folder, where it was asked for. */
    let menu = $state<{ folder: Folder; x: number; y: number } | null>(null);

    function dragStart(event: DragEvent, each: Folder) {
        if (!movable(each) || !event.dataTransfer) return event.preventDefault();
        event.dataTransfer.effectAllowed = "move";
        // WebKit starts no drag that carries nothing.
        event.dataTransfer.setData("text/plain", each.name);
        dragging = each;
    }

    function dragOver(event: DragEvent, each: Folder) {
        if (!dragging || !canHold(folders, dragging, each)) return;
        event.preventDefault();
        if (event.dataTransfer) event.dataTransfer.dropEffect = "move";
        over = each.id;
    }

    /** The folder list, and how far it moves each frame while a folder is held near its top or foot. */
    let list = $state<HTMLElement | null>(null);
    let drift = 0;
    let drifting = 0;
    /** How near an edge, in pixels, a dragged folder starts the list scrolling. */
    const EDGE = 48;

    /** A long list scrolls under a folder dragged to its edge, faster the nearer the edge, so a folder out of sight can be dropped on. */
    function dragNear(event: DragEvent) {
        if (!dragging || !list) return;
        const box = list.getBoundingClientRect();
        const above = box.top + EDGE - event.clientY;
        const below = event.clientY - (box.bottom - EDGE);
        drift = above > 0 ? -Math.min(above, EDGE) / 4 : below > 0 ? Math.min(below, EDGE) / 4 : 0;
        if (drift && !drifting) drifting = requestAnimationFrame(scrollOn);
    }

    function scrollOn() {
        drifting = 0;
        if (!dragging || !drift || !list) return;
        list.scrollTop += drift;
        drifting = requestAnimationFrame(scrollOn);
    }

    function dragLeave(event: DragEvent, each: Folder) {
        const to = event.relatedTarget;
        if (over === each.id && !(to instanceof Node && (event.currentTarget as Node).contains(to))) over = null;
    }

    /** Dropped on a folder that can take it: nothing moves until it is agreed to. */
    function drop(event: DragEvent, each: Folder) {
        const folder = dragging;
        dragging = over = null;
        if (!folder || !canHold(folders, folder, each)) return;
        event.preventDefault();
        moving = { folder, into: each, busy: false, problem: null };
    }

    function showMenu(event: MouseEvent, each: Folder) {
        event.preventDefault();
        // Asked for with the keyboard, it opens by the folder and not at a pointer that is somewhere else.
        const box = (event.currentTarget as HTMLElement).getBoundingClientRect();
        const pointed = event.clientX > 0 || event.clientY > 0;
        menu = { folder: each, x: pointed ? event.clientX : box.left + 24, y: pointed ? event.clientY : box.bottom };
    }

    async function moveFolderTo(destination: string, merge: boolean) {
        if (!accountId || !moving || moving.busy) return;
        const { folder } = moving;
        moving = { ...moving, busy: true, problem: null };
        try {
            const done = await moveFolder(accountId, folder.id, destination, merge);
            moving = null;
            // What it went into is opened, to show it there.
            if (collapsed.includes(destination)) toggle(destination);
            // After a merge the folder shown may be one that is gone, or one that has gained mail.
            await loadFolders();
            if (done.merged) await refresh();
            if (done.kept?.length) problem = `Not everything could be moved out of ${done.kept.join(", ")}, so ${done.kept.length > 1 ? "they were" : "it was"} left where ${done.kept.length > 1 ? "they were" : "it was"}.`;
        } catch (failure) {
            if (moving) moving = { ...moving, busy: false, problem: say(failure) };
        }
    }

    /**
     * The other mailboxes that go with the one shown: its account, and those
     * shared with it. Each has its folders under this one's, closed up as
     * they were left when it was the one shown.
     */
    let beside = $state<{ mailbox: Account; folders: Folder[]; collapsed: string[] }[]>([]);

    /** Mailboxes whose folders are closed up altogether while another is shown. */
    const SHUT = "brolga.beside.shut";
    let shut = $state<string[]>(keptIds(SHUT));

    function shutOrOpen(mailbox: string) {
        shut = without(shut, mailbox);
        keep(SHUT, shut);
    }

    /** Close or open a folder of a mailbox that is not the one shown. It is as it was left when that mailbox is shown. */
    function toggleIn(mailbox: string, id: string) {
        beside = beside.map((each) => {
            if (each.mailbox.id !== mailbox) return each;
            const collapsed = without(each.collapsed, id);
            keep(collapsedKey(mailbox), collapsed);
            return { ...each, collapsed };
        });
    }

    async function loadBeside() {
        const here = accounts.findIndex((each) => each.id === accountId);
        const mine = accounts[here];
        if (!mine) return;
        const root = mine.shared_with ?? mine.id;
        const family = accounts.map((each) => ({ each })).filter(({ each }) => (each.shared_with ?? each.id) === root && each.id !== mine.id);
        const found = await Promise.all(
            family.map(async ({ each }) => ({ mailbox: each, folders: await listFolders(each.id).catch(() => []), collapsed: keptIds(collapsedKey(each.id)) })),
        );
        if (accountId === mine.id) beside = found;
    }

    /** Open a folder of another mailbox: that mailbox becomes the one shown. */
    async function showIn(mailbox: Account, folder: string) {
        await chooseAccount(mailbox.id);
        await showFolder(folder);
    }

    /**
     * Ask again what folders the mailbox shown has. An answer that comes
     * once another mailbox has been chosen is not about the one on screen,
     * and is left: false is given for it.
     */
    async function relist() {
        const of = accountId;
        if (!of) return false;
        const found = await listFolders(of);
        if (accountId !== of) return false;
        folders = found;
        return true;
    }

    async function loadFolders() {
        if (!accountId) return;
        loadBeside();
        if (!(await relist())) return;
        if (!folders.some((f) => f.id === folderId)) {
            const inbox = folders.find((f) => f.well_known === "inbox") ?? folders[0];
            if (inbox) await showFolder(inbox.id);
        }
    }

    function reset() {
        listing += 1;
        seeking += 1;
        seen = { first: 0, last: 0 };
        asked.clear();
        picked = [];
        rows = [];
        total = 0;
    }

    async function showFolder(id: string) {
        folderId = id;
        query = "";
        searching = false;
        reset();
        await need(0);
    }

    /** Load the page of the folder that holds this position, once. */
    async function need(index: number) {
        if (!accountId || !folderId || searching) return;
        const offset = Math.floor(index / PAGE) * PAGE;
        if (asked.has(offset)) return;
        asked.add(offset);

        const mine = listing;
        try {
            const page = await listMessages(accountId, folderId, offset, PAGE, view);
            // Left if the list has been replaced, or the page was let go again while it was on its way.
            if (mine !== listing || !asked.has(offset)) return;
            total = page.total;
            const next = rows.slice();
            page.messages.forEach((message, i) => (next[offset + i] = message));
            rows = next;
        } catch (failure) {
            asked.delete(offset);
            problem = say(failure);
        }
    }

    /** The fetching again that is under way, if one is, and whether another has been asked for since it began. */
    let refreshing: Promise<void> | null = null;
    let stale = false;

    /**
     * Fetch again what is on screen, in place, so new mail appears without
     * the list jumping. One at a time: asked for while one is under way, it
     * is done once more when that is over, and both end together.
     */
    function refresh(): Promise<void> {
        if (refreshing) {
            stale = true;
            return refreshing;
        }
        const turns = (async () => {
            try {
                do {
                    stale = false;
                    await fetchAgain();
                } while (stale);
            } finally {
                refreshing = null;
            }
        })();
        refreshing = turns;
        return turns;
    }

    /**
     * Fetch again the pages near what is on screen and the page the selected
     * line is on. A page further off may no longer start where it did, so it
     * is let go, to be fetched when it is scrolled to. While lines are chosen
     * across pages, or being gathered to be, every page is fetched: which
     * lines are chosen is told from the rows.
     */
    async function fetchAgain() {
        const [account, within] = [accountId, folderId];
        if (!account || !within || searching) return;
        const mine = listing;
        const had = [...asked].sort((a, b) => a - b);
        const whole = () => many || gathering > 0;
        const at = rows.findIndex((row) => row?.id === selectedId);
        const near = (offset: number) => (offset + PAGE > seen.first - PAGE && offset < seen.last + PAGE) || (at >= offset && at < offset + PAGE);
        const fetched = new Map<number, Message[]>();
        let now = total;
        for (const offset of had) {
            if (!whole() && !near(offset)) continue;
            const page = await listMessages(account, within, offset, PAGE, view);
            if (mine !== listing) return;
            now = page.total;
            fetched.set(offset, page.messages);
        }
        // Put into the rows as they are now, not as they were, so that a page loaded meanwhile is kept.
        const next = rows.slice();
        for (const offset of had) {
            const page = fetched.get(offset);
            if (page) {
                for (let i = 0; i < PAGE; i++) next[offset + i] = page[i];
            } else if (whole()) {
                // Wanted after all, since lines began to be chosen: fetched on the turn after this.
                stale = true;
            } else {
                for (let i = 0; i < PAGE; i++) next[offset + i] = undefined;
                asked.delete(offset);
            }
        }
        next.length = Math.min(next.length, now);
        rows = next;
        total = now;
    }

    /** Search for the words in the box, or for those given. With none, the folder is shown again. */
    async function search(words = query) {
        clearTimeout(searchSoon);
        if (!accountId) return;
        // A search still under way is no longer the one wanted: its answer is left.
        const mine = ++seeking;
        if (!words.trim()) {
            if (searching && folderId) await showFolder(folderId);
            return;
        }
        try {
            const found = await searchMail(accountId, words, everywhere ? undefined : (folderId ?? undefined));
            if (mine !== seeking) return;
            // The list is replaced when the answer is in, not emptied first, so
            // it does not flicker with every letter typed. Only now is the
            // folder forgotten: a search that fails leaves it as it was.
            listing += 1;
            asked.clear();
            searching = true;
            sought = words;
            capped = found.length >= SEARCH_MOST;
            picked = [];
            rows = found;
            total = found.length;
        } catch (failure) {
            if (mine === seeking) problem = say(failure);
        }
    }

    /** Search as the words are typed, a moment after the typing pauses. */
    let searchSoon: ReturnType<typeof setTimeout> | undefined;
    function typedSearch() {
        clearTimeout(searchSoon);
        searchSoon = setTimeout(() => search(), query.trim() ? 150 : 0);
    }

    /** How many runs of lines are being gathered to be chosen. Until they are, no page is let go. */
    let gathering = 0;

    /** Load every page of the folder that holds a position from one to another. */
    async function load(from: number, to: number) {
        gathering += 1;
        try {
            for (let offset = Math.floor(from / PAGE) * PAGE; offset <= to; offset += PAGE) await need(offset);
        } finally {
            gathering -= 1;
        }
    }

    /** Choose these lines. One alone is opened to be read; several are held to be acted on together. */
    function pick(ids: string[], cursor: string) {
        if (ids.length === 1) {
            const only = rows.find((row) => row?.id === ids[0]);
            if (only && (many || readingId !== only.id)) void open(only);
            return;
        }
        picked = ids;
        selectedId = ids.length === 0 ? null : cursor;
        readingId = null;
        reads += 1;
        opening = false;
        opened = null;
        thread = [];
        attachments = [];
    }

    /** A line of the list is chosen: alone, added to or taken from those chosen, or with all between it and where the choosing began. */
    async function select(message: Message, how: "one" | "toggle" | "range") {
        if (how === "one") return open(message);
        const mine = listing;
        if (how === "toggle") {
            // The selected line counts only if it is in this list: it may be of a folder since left.
            const now = many ? picked : selectedId && rows.some((row) => row?.id === selectedId) ? [selectedId] : [];
            anchorId = message.id;
            return pick(now.includes(message.id) ? now.filter((id) => id !== message.id) : [...now, message.id], message.id);
        }
        const to = rows.findIndex((row) => row?.id === message.id);
        const began = rows.findIndex((row) => row?.id === anchorId);
        if (to < 0) return;
        let from = began < 0 ? to : began;
        if (Math.abs(to - from) >= MOST_PICKED) {
            from = to > from ? to - MOST_PICKED + 1 : to + MOST_PICKED - 1;
            tell(`${MOST_PICKED.toLocaleString()} are selected, which is the most at once.`);
        }
        const [low, high] = [Math.min(from, to), Math.max(from, to)];
        await load(low, high);
        if (mine !== listing) return;
        const start = anchorId;
        pick(rows.slice(low, high + 1).flatMap((row) => (row ? [row.id] : [])), message.id);
        // Opening one alone starts the choosing again from it; a run keeps where it began.
        if (began >= 0) anchorId = start;
    }

    /** Choose every line of the folder, or of what a search found. */
    async function selectAll() {
        if (total === 0) return;
        const mine = listing;
        const upTo = Math.min(total, MOST_PICKED);
        await load(0, upTo - 1);
        if (mine !== listing) return;
        const ids = rows.slice(0, upTo).flatMap((row) => (row ? [row.id] : []));
        pick(ids, selectedId && ids.includes(selectedId) ? selectedId : (ids[0] ?? ""));
        if (total > upTo) tell(`The first ${upTo.toLocaleString()} are selected, which is the most at once.`);
    }

    /** Go back from several lines chosen to the one the keys were on. */
    function selectOne() {
        const at = rows.find((row) => row?.id === selectedId);
        if (at) void open(at);
        else pick([], "");
    }

    /** Choose a line of the list. For a conversation, its other messages are fetched to read beside it. */
    async function open(message: Message) {
        // A draft is opened to be carried on with, wherever it was started.
        picked = [];
        anchorId = message.id;
        if (message.is_draft && accountId) {
            selectedId = message.id;
            const of = accountId;
            try {
                const draft = await openDraft(of, message.id);
                if (accountId !== of) return;
                writing = draft;
                await loadDrafts();
            } catch (failure) {
                problem = say(failure);
            }
            return;
        }
        selectedId = message.id;
        // A message being written has the pane. The line is chosen, and is opened when the writing is put aside,
        // so that nothing is marked read that has not been shown.
        if (writing) return;
        thread = [];
        const reading = read(message);
        if (accountId && view.conversations && !searching && message.conversation_id) {
            try {
                const all = await getConversation(accountId, message.conversation_id);
                if (selectedId === message.id && all.length > 1) thread = all;
            } catch {
                // The message itself still opens; it is only shown on its own.
            }
        }
        await reading;
    }

    async function read(message: Message, withImages = false) {
        const of = accountId;
        if (!of) return;
        readingId = message.id;
        imagesShown = withImages;
        if (opened?.message.id !== message.id) {
            attachments = [];
            saved = null;
        }
        opening = true;
        const mine = ++reads;
        readingProblem = null;
        try {
            const result = await openMessage(of, message.id, withImages, view.plain);
            // Another message has been asked for since, or another account chosen.
            if (readingId !== message.id || accountId !== of) return;
            opened = result;
            imagesShown = result.remote_content_allowed;
            // Reading it is what marks it read. Quietly: there is nothing to undo about having read something.
            if (!result.message.is_read) void change([message.id], { is_read: true });
            if (result.message.has_attachments && attachments.length === 0) {
                try {
                    const all = await listAttachments(of, message.id);
                    if (readingId === message.id && accountId === of) attachments = all.filter((each) => !each.is_inline);
                } catch (failure) {
                    if (readingId === message.id && accountId === of) saved = say(failure);
                }
            }
        } catch (failure) {
            if (readingId === message.id && accountId === of) {
                opened = null;
                readingProblem = say(failure);
            }
        } finally {
            if (mine === reads) opening = false;
        }
    }

    async function sync() {
        if (syncing) return;
        syncing = true;
        // A first sync of a large mailbox takes minutes. What has arrived so
        // far is shown as it comes, not held back until the last of it.
        const showing = setInterval(async () => {
            try {
                await loadFolders();
                await refresh();
            } catch {
                // The sync itself says what is wrong, when it ends.
            }
        }, 2_000);
        try {
            const results = await syncNow();
            const failed = results.find((r) => r.account_id === accountId && !r.ok) ?? results.find((r) => !r.ok);
            syncProblem = failed ? failed.error : null;
            await loadFolders();
            await refresh();
            if (!failed) await tellOfRefusals();
            await loadDrafts();
        } catch (failure) {
            syncProblem = say(failure);
        } finally {
            clearInterval(showing);
            syncing = false;
            lastSync = Date.now();
        }
    }

    // New mail is fetched every two minutes while the window is open on
    // mail, and when it is come back to after half a minute or more away.
    // Come back to sooner from another screen, what is on this computer is
    // shown as it now is, without a sync.
    const EVERY = 2 * 60_000;
    let lastSync = 0;
    $effect(() => {
        if (!shown) return;
        const back = () => Date.now() - lastSync > 30_000 && sync();
        untrack(() => {
            // Not at the start, when the first account is still to be chosen and is then synced.
            if (!accountId || back()) return;
            loadFolders()
                .then(refresh)
                .then(loadDrafts)
                .catch(() => {});
        });
        const timer = setInterval(sync, EVERY);
        window.addEventListener("focus", back);

        return () => {
            clearInterval(timer);
            window.removeEventListener("focus", back);
        };
    });

    function keydown(event: KeyboardEvent) {
        if (menu && event.key === "Escape") {
            event.preventDefault();
            menu = null;
            return;
        }
        // Another screen is shown, or a question or the palette over the window has the keys, not the mail behind it.
        if (!shown || inDialog(event)) return;
        if (event.key === "F9") {
            event.preventDefault();
            sync();
        } else if (event.key === "F3" || ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "e")) {
            event.preventDefault();
            searchBox?.focus();
            searchBox?.select();
        } else if (writing) {
            // The keys below act on the message being read, which is not what is on screen.
        } else if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "n") {
            event.preventDefault();
            write("new");
        } else if ((event.ctrlKey || event.metaKey) && (event.key === "." || event.key === ",")) {
            // The next message, or the one before: as the arrow keys do in the list.
            event.preventDefault();
            document.querySelector('[role="listbox"][aria-label="Messages"]')?.dispatchEvent(new KeyboardEvent("keydown", { key: event.key === "." ? "ArrowDown" : "ArrowUp", bubbles: true }));
        } else if (targets.length > 0 && (event.ctrlKey || event.metaKey) && event.shiftKey && event.key.toLowerCase() === "v") {
            event.preventDefault();
            const move = document.querySelector<HTMLSelectElement>('select[aria-label="Move to folder"]');
            move?.focus();
            try {
                move?.showPicker();
            } catch {
                // Focused, then, to be opened with the keyboard.
            }
        } else if (current && (event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "r") {
            event.preventDefault();
            write(event.shiftKey ? "reply_all" : "reply");
        } else if (current && (event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "f") {
            event.preventDefault();
            write("forward");
        } else if (targets.length > 0 && !typing(event)) {
            // With Shift these are other things: Ctrl+Shift+Q is a new meeting.
            const ctrl = (event.ctrlKey || event.metaKey) && !event.shiftKey;
            if (ctrl && event.key.toLowerCase() === "q") {
                event.preventDefault();
                markRead(true);
            } else if (ctrl && event.key.toLowerCase() === "u") {
                event.preventDefault();
                markRead(false);
            } else if (event.key === "Insert") {
                event.preventDefault();
                // Several are flagged unless every one of them already is.
                flag((many ? chosen.every((each) => each.flag_status === "flagged") : current?.flag_status === "flagged") ? "notFlagged" : "flagged");
            } else if (event.key === "Delete") {
                event.preventDefault();
                remove();
            } else if (event.key === "Backspace") {
                event.preventDefault();
                archive();
            } else if (event.key === "Escape" && many) {
                event.preventDefault();
                selectOne();
            }
        }
        if (undoable && !writing && (event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "z" && !typing(event)) {
            event.preventDefault();
            takeBack();
        }
    }

    /** Show another account. Nothing of the one before is kept, and what it answers late is left. */
    async function chooseAccount(id: string) {
        accountId = id;
        recallCollapsed();
        folderId = null;
        opened = null;
        selectedId = null;
        readingId = null;
        anchorId = null;
        writing = null;
        thread = [];
        attachments = [];
        saved = null;
        categories = [];
        drafts = [];
        problem = null;
        refusal = null;
        readingProblem = null;
        reads += 1;
        opening = false;
        clearTimeout(undoTimer);
        undoable = null;
        clearTimeout(noticeTimer);
        notice = null;
        reset();
        await loadFolders().catch((failure) => (problem = say(failure)));
        await loadDrafts();
        // Asked of the server, which may be slow or out of reach: not waited for.
        listCategories(id)
            .then((found) => accountId === id && (categories = found))
            .catch(() => {});
    }

    // Show what is already on this computer at once, then fetch what is new.
    // The same when the account shown has been removed: the first of those left is.
    $effect(() => {
        const firstAccount = accounts[0];
        if (firstAccount && !accounts.some((each) => each.id === accountId)) {
            chooseAccount(firstAccount.id).then(sync);
        }
    });
</script>

<svelte:window onkeydown={keydown} onclick={() => (menu = null)} onblur={() => (menu = null)} />

<div class="mailview" hidden={!shown}>
<Toolbar
    message={writing || many ? null : current}
    several={writing ? [] : chosen}
    {covers}
    {folders}
    {within}
    {categories}
    {syncing}
    {working}
    onWrite={write}
    onDelete={remove}
    onPurge={purge}
    onJunk={junk}
    onNotJunk={notJunk}
    onArchive={archive}
    onMove={(folder) => moveTo(folder.id, `Moved to ${folder.name}`)}
    onSetRead={markRead}
    onSetFlag={flag}
    onCategorise={categorise}
    onSync={sync}
/>
<div class="mail" class:bottom={view.pane === "bottom"}>
    <nav
        class="folders"
        aria-label="Folders"
        bind:this={list}
        ondragover={dragNear}
        ondragleave={(event) => event.relatedTarget instanceof Node && list?.contains(event.relatedTarget) ? null : (drift = 0)}
        ondrop={() => (drift = 0)}
    >
        {#if accounts.length > 1}
            <select aria-label="Account" value={accountId} onchange={(event) => chooseAccount(event.currentTarget.value)}>
                {#each accounts as each (each.id)}
                    <option value={each.id}>{each.archive ? `${each.username} (archive)` : each.shared_with ? `↳ ${each.username} (shared)` : each.username}</option>
                {/each}
            </select>
        {:else if account}
            <p class="account" title={account.username}>{account.username}</p>
        {/if}

        {#if drafts.length > 0}
            <h2 id="drafts-heading">Being written</h2>
            <ul aria-labelledby="drafts-heading">
                {#each drafts as each (each.id)}
                    <li>
                        <span class="twist"></span>
                        <button
                            type="button"
                            class="folder"
                            class:current={writing?.id === each.id}
                            disabled={each.state === "queued"}
                            title={each.state === "failed" ? (each.error ?? "") : undefined}
                            onclick={() => (writing = each)}
                        >
                            <span class="name">{each.subject || "(No subject)"}</span>
                            {#if each.state === "queued"}<span class="count">Sending…</span>{/if}
                            {#if each.state === "failed"}<span class="count warn">Not sent</span>{/if}
                        </button>
                    </li>
                {/each}
            </ul>
        {/if}

        {#snippet row(each: Folder, parent: boolean, open: boolean, indent: number)}
            {@const Shape = FOLDER_ICONS[each.well_known ?? ""] ?? FolderIcon}
            <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
            <li
                style:padding-left="{indent * 0.9}rem"
                class:into={over === each.id}
                draggable={movable(each)}
                ondragstart={(event) => dragStart(event, each)}
                ondragover={(event) => dragOver(event, each)}
                ondragleave={(event) => dragLeave(event, each)}
                ondrop={(event) => drop(event, each)}
                ondragend={() => (dragging = over = null)}
                oncontextmenu={(event) => showMenu(event, each)}
            >
                {#if parent}
                    <button type="button" class="twist" aria-expanded={open} aria-label="{open ? 'Collapse' : 'Expand'} {each.name}" onclick={() => toggle(each.id)}>
                        {open ? "▾" : "▸"}
                    </button>
                {:else}
                    <span class="twist"></span>
                {/if}
                <button type="button" class="folder" class:unread={each.unread > 0} class:current={each.id === folderId && !searching} aria-current={each.id === folderId && !searching} onclick={() => showFolder(each.id)}>
                    <Shape size={15} aria-hidden="true" />
                    <span class="name">{each.name}</span>
                    {#if each.unread > 0}<span class="count" aria-label="{each.unread} unread">{each.unread}</span>{/if}
                </button>
                <button type="button" class="pin" class:pinned={each.favourite} aria-pressed={each.favourite} aria-label="{each.favourite ? 'Unpin' : 'Pin'} {each.name}" title={each.favourite ? "Remove from favourites" : "Add to favourites"} onclick={() => pin(each)}>
                    <Star size={13} aria-hidden="true" fill={each.favourite ? "currentColor" : "none"} />
                </button>
            </li>
        {/snippet}

        {#if favourites.length > 0}
            <h2 id="favourites-heading">Favourites</h2>
            <ul aria-labelledby="favourites-heading">
                {#each favourites as each (each.id)}
                    {@render row(each, false, true, 0)}
                {/each}
            </ul>
            <h2 id="folders-heading">Folders</h2>
        {:else}
            <h2 id="folders-heading" class:unseen={drafts.length === 0}>Folders</h2>
        {/if}
        <ul aria-labelledby="folders-heading">
            {#each tree as each (each.folder.id)}
                {@render row(each.folder, each.parent, each.open, each.folder.depth)}
            {/each}
        </ul>

        {#each beside.filter((each) => each.folders.length > 0) as each (each.mailbox.id)}
            {@const open = !shut.includes(each.mailbox.id)}
            <h2 class="mailbox" id="beside-{each.mailbox.id}" title={each.mailbox.username}>
                <button type="button" aria-expanded={open} onclick={() => shutOrOpen(each.mailbox.id)}>
                    <span class="twist" aria-hidden="true">{open ? "▾" : "▸"}</span>{each.mailbox.username}
                </button>
            </h2>
            {#if open}
                <ul aria-labelledby="beside-{each.mailbox.id}">
                    {#each treeOf(each.folders, each.collapsed) as { folder, parent, open: unfolded } (folder.id)}
                        {@const Shape = FOLDER_ICONS[folder.well_known ?? ""] ?? FolderIcon}
                        <li style:padding-left="{(folder.depth ?? 0) * 0.9}rem">
                            {#if parent}
                                <button type="button" class="twist" aria-expanded={unfolded} aria-label="{unfolded ? 'Collapse' : 'Expand'} {folder.name}" onclick={() => toggleIn(each.mailbox.id, folder.id)}>
                                    {unfolded ? "▾" : "▸"}
                                </button>
                            {:else}
                                <span class="twist"></span>
                            {/if}
                            <button type="button" class="folder" class:unread={folder.unread > 0} onclick={() => showIn(each.mailbox, folder.id)}>
                                <Shape size={15} aria-hidden="true" />
                                <span class="name">{folder.name}</span>
                                {#if folder.unread > 0}<span class="count" aria-label="{folder.unread} unread">{folder.unread}</span>{/if}
                            </button>
                        </li>
                    {/each}
                </ul>
            {/if}
        {/each}
    </nav>

    {#if menu}
        {@const each = menu.folder}
        <ul class="menu" role="menu" aria-label="Folder {each.name}" style:left="{menu.x}px" style:top="{menu.y}px">
            <li role="none">
                <button
                    type="button"
                    role="menuitem"
                    disabled={!movable(each)}
                    title={movable(each) ? undefined : `${each.name} stays where it is`}
                    onclick={() => (moving = { folder: each, into: null, busy: false, problem: null })}
                    {@attach (node) => node.focus()}
                >
                    Move to…
                </button>
            </li>
        </ul>
    {/if}
    {#if moving}
        <MoveFolder folder={moving.folder} {folders} into={moving.into} busy={moving.busy} problem={moving.problem} onMove={moveFolderTo} onCancel={() => (moving = null)} />
    {/if}

    <div class="list">
        <div class="bar">
            <form role="search" onsubmit={(event) => (event.preventDefault(), search())}>
                <input
                    bind:this={searchBox}
                    bind:value={query}
                    type="search"
                    aria-label="Search mail"
                    placeholder="Search mail"
                    oninput={typedSearch}
                />
            </form>
            <select aria-label="Search in" title="Where to search" value={everywhere ? "all" : "folder"} onchange={(event) => scope(event.currentTarget.value === "all")}>
                <option value="all">All folders</option>
                <option value="folder">This folder</option>
            </select>
        </div>

        {#snippet wrong(words: string, dismiss: () => void)}
            <p class="problem" role="alert">
                <span>{words}</span>
                <button type="button" aria-label="Dismiss" title="Dismiss" onclick={dismiss}><X size={14} aria-hidden="true" /></button>
            </p>
        {/snippet}
        {#if syncProblem}{@render wrong(syncProblem, () => (syncProblem = null))}{/if}
        {#if refusal}{@render wrong(refusal, refusalSeen)}{/if}
        {#if problem}{@render wrong(problem, () => (problem = null))}{/if}

        <p class="heading">
            {#if searching}
                {capped ? `The first ${total} results` : total === 1 ? "1 result" : `${total} results`} for “{sought}” {everywhere ? "in all folders" : `in ${folder?.name ?? "this folder"}`}
            {:else if folder}
                {@const things = view.conversations ? "conversation" : "message"}
                {folder.name} <span class="muted">· {total === 1 ? `1 ${things}` : `${total.toLocaleString()} ${things}s`}</span>
            {/if}
        </p>

        {#if !searching}
            <div class="view">
                <label>
                    <input type="checkbox" checked={view.conversations} onchange={(event) => lay({ ...view, conversations: event.currentTarget.checked })} />
                    Conversations
                </label>
                <label>
                    Sort
                    <select
                        value={view.conversations ? "newest" : view.sort}
                        disabled={view.conversations}
                        title={view.conversations ? "Conversations are always newest first" : undefined}
                        onchange={(event) => lay({ ...view, sort: event.currentTarget.value as Sort })}
                    >
                        <option value="newest">Newest first</option>
                        <option value="oldest">Oldest first</option>
                        <option value="sender">By sender</option>
                        <option value="subject">By subject</option>
                    </select>
                </label>
                <label>
                    <input type="checkbox" checked={view.plain} onchange={(event) => lay({ ...view, plain: event.currentTarget.checked })} />
                    Plain text
                </label>
                <label>
                    Reading pane
                    <select value={view.pane} onchange={(event) => lay({ ...view, pane: event.currentTarget.value === "bottom" ? "bottom" : "right" })}>
                        <option value="right">Right</option>
                        <option value="bottom">Bottom</option>
                    </select>
                </label>
            </div>
        {/if}

        {#if total === 0}
            <p class="empty muted">
                {#if searching}
                    Nothing matches.
                {:else if folder?.synced === false && !account?.archive}
                    This folder has not been fetched yet. Mail is fetched a folder at a time, and the bar along the bottom says which it has reached.
                {:else}
                    {syncing ? "Fetching your mail…" : "Nothing here."}
                {/if}
            </p>
        {:else}
            <div class="rows">
                <MessageList {total} {rows} {selectedId} {picked} {categories} onSelect={select} onAll={selectAll} onFlag={flagLine} onDelete={folder?.well_known === "deleteditems" && !searching ? undefined : removeLine} onNeed={need} onSeen={(first, last) => (seen = { first, last })} byDate={!searching && (view.conversations || view.sort === "newest" || view.sort === "oldest")} />
            </div>
        {/if}
        {#if notice}
            <p class="undo" role="status">{notice}</p>
        {/if}
        {#if undoable}
            <p class="undo" role="status">
                {undoable.said}
                <button type="button" onclick={takeBack} title="Undo (Ctrl+Z)">Undo</button>
            </p>
        {/if}
    </div>

    {#if writing && accountId}
        {#key writing.id}
            <Compose
                {accountId}
                draft={writing}
                fromChoices={account && !account.shared_with ? [account.username, ...accounts.filter((each) => each.shared_with === account.id).map((each) => each.username)] : []}
                onDone={doneWriting}
                onKept={loadDrafts}
            />
        {/key}
    {:else if many}
        <section class="picked" aria-label="Messages selected">
            <p class="count" role="status">{chosen.length.toLocaleString()} messages selected</p>
            <p class="muted">Mark, flag, categorise, move, archive or delete them together from the toolbar.</p>
            <button type="button" title="Escape" onclick={selectOne}>Clear the selection</button>
        </section>
    {:else}
        <ReadingPane
            {accountId}
            {opened}
            loading={opening}
            problem={readingProblem}
            {imagesShown}
            {thread}
            {attachments}
            {saved}
            onSave={save}
            onTrust={trust}
        {onCalendar}
        onAnswer={answer}
        {categories}
            onRead={(message) => read(message)}
            onShowImages={() => opened && read(opened.message, true)}
            onOpenLink={(url) => openLink(url).catch((failure) => (readingProblem = say(failure)))}
        />
    {/if}
</div>
</div>

<style>
    .picked {
        height: 100%;
        min-width: 0;
        display: flex;
        flex-direction: column;
        align-items: center;
        justify-content: center;
        gap: 0.5rem;
        padding: 1rem;
        text-align: center;
    }

    .picked p {
        margin: 0;
    }

    .picked .count {
        font-size: 1.1rem;
        font-weight: 600;
    }

    .mailview {
        height: 100%;
        min-height: 0;
        display: flex;
        flex-direction: column;
    }

    .mailview[hidden] {
        display: none;
    }

    .mail {
        flex: 1;
        height: 100%;
        display: grid;
        grid-template-columns: 13rem minmax(16rem, 24rem) minmax(0, 1fr);
        min-height: 0;
    }

    .folders {
        border-right: 1px solid var(--line);
        background: var(--raised);
        overflow-y: auto;
        padding: 0.5rem;
        display: flex;
        flex-direction: column;
        gap: 0.5rem;
    }

    .account {
        margin: 0;
        padding: 0.25rem 0.5rem;
        font-size: 0.8rem;
        font-weight: 600;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    }

    ul {
        margin: 0;
        padding: 0;
        list-style: none;
    }

    h2 {
        margin: 0;
        padding: 0.25rem 0.5rem 0;
        font-size: 0.7rem;
        font-weight: 600;
        letter-spacing: 0.04em;
        text-transform: uppercase;
        color: var(--muted);
    }

    h2.mailbox {
        /* It hides what overflows, so it would otherwise be squeezed to nothing in a full list. */
        flex: none;
        margin-top: 0.6rem;
        padding-top: 0.5rem;
        border-top: 1px solid var(--line);
        padding-left: 0;
        font-size: 0.8rem;
        letter-spacing: 0;
        text-transform: none;
    }

    h2.mailbox button {
        display: flex;
        align-items: center;
        width: 100%;
        padding: 0.2rem 0.4rem 0.2rem 0;
        font: inherit;
        font-weight: 600;
        color: var(--text);
        text-align: left;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    }

    h2.unseen {
        position: absolute;
        width: 1px;
        height: 1px;
        overflow: hidden;
        clip-path: inset(50%);
    }

    li {
        display: flex;
        align-items: center;
    }

    .folders button {
        border: 0;
        background: none;
        font-size: 0.9rem;
    }

    .twist {
        flex: none;
        width: 1.1rem;
        padding: 0;
        color: var(--muted);
    }

    .folder {
        flex: 1;
        min-width: 0;
        display: flex;
        align-items: center;
        gap: 0.4rem;
        padding: 0.35rem 0.4rem;
        text-align: left;
    }

    /* A folder with nothing unread stands back a little; one with unread mail is bold, as its count is. */
    .folder .name {
        color: color-mix(in srgb, var(--text) 78%, var(--surface));
    }

    .folder.unread .name {
        color: var(--text);
        font-weight: 700;
    }

    .folder.current {
        background: color-mix(in srgb, var(--accent) 18%, var(--surface));
    }

    .folder.current .name {
        color: var(--text);
    }

    /* The pin keeps out of the way until the row is pointed at or reached by keyboard. */
    .pin {
        flex: none;
        padding: 0 0.3rem;
        color: var(--muted);
        opacity: 0;
    }

    .pin.pinned,
    li:hover .pin,
    .pin:focus-visible {
        opacity: 1;
    }

    /* Where a folder being dragged would go if it were let go here. */
    li.into {
        outline: 2px solid var(--accent);
        outline-offset: -2px;
        border-radius: 0.3rem;
    }

    .menu {
        position: fixed;
        z-index: 40;
        min-width: 10rem;
        margin: 0;
        padding: 0.3rem;
        list-style: none;
        background: var(--surface);
        border: 1px solid var(--line);
        border-radius: 0.4rem;
        box-shadow: 0 6px 18px color-mix(in srgb, black 25%, transparent);
    }

    .menu button {
        width: 100%;
        border: 0;
        background: none;
        padding: 0.3rem 0.5rem;
        text-align: left;
    }

    .menu button:hover:not(:disabled),
    .menu button:focus-visible {
        background: var(--raised);
    }

    .name {
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    }

    .count {
        font-weight: 700;
        font-size: 0.8rem;
    }

    .mail.bottom {
        grid-template-columns: 13rem minmax(0, 1fr);
        grid-template-rows: minmax(8rem, 40%) minmax(0, 1fr);
    }

    .mail.bottom .folders {
        grid-row: 1 / 3;
    }

    .mail.bottom .list {
        border-right: 0;
        border-bottom: 1px solid var(--line);
    }

    .view {
        display: flex;
        flex-wrap: wrap;
        justify-content: space-between;
        align-items: center;
        gap: 0.5rem;
        padding: 0.4rem 0.75rem;
        font-size: 0.8rem;
        color: var(--muted);
    }

    .view label {
        display: flex;
        align-items: center;
        gap: 0.3rem;
    }

    .view select {
        font-size: 0.8rem;
        padding: 0.1rem 0.25rem;
    }

    .folder :global(svg) {
        flex: none;
        color: var(--muted);
    }

    .folder .name {
        flex: 1;
    }

    .warn {
        color: var(--danger);
    }

    .undo {
        flex: none;
        margin: 0;
        padding: 0.4rem 0.75rem;
        display: flex;
        justify-content: space-between;
        align-items: center;
        border-top: 1px solid var(--line);
        background: var(--raised);
        font-size: 0.85rem;
    }

    .list {
        border-right: 1px solid var(--line);
        display: flex;
        flex-direction: column;
        min-height: 0;
        min-width: 0;
    }

    .bar {
        display: flex;
        gap: 0.5rem;
        padding: 0.5rem;
        border-bottom: 1px solid var(--line);
    }

    .bar form {
        flex: 1;
        min-width: 0;
    }

    .bar input {
        width: 100%;
        box-sizing: border-box;
    }

    .heading {
        margin: 0;
        padding: 0.4rem 0.75rem;
        font-size: 0.85rem;
        font-weight: 600;
        border-bottom: 1px solid var(--line);
    }

    .heading .muted {
        font-weight: 400;
    }

    .rows {
        flex: 1;
        min-height: 0;
        display: flex;
        flex-direction: column;
    }

    .empty {
        margin: 2rem auto;
        padding: 0 1rem;
        text-align: center;
    }

    .problem {
        margin: 0.5rem;
        padding: 0.5rem 0.75rem;
        display: flex;
        align-items: flex-start;
        gap: 0.5rem;
        border-radius: 0.4rem;
        background: color-mix(in srgb, var(--danger) 12%, var(--surface));
        font-size: 0.85rem;
    }

    .problem span {
        flex: 1;
        min-width: 0;
    }

    .problem button {
        flex: none;
        display: grid;
        place-items: center;
        padding: 0.15rem;
        border: 0;
        background: none;
        color: inherit;
    }
</style>
