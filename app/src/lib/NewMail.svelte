<script lang="ts">
    import type { Account } from "./accounts";
    import { execute, notify } from "./commands";
    import { type Message, listFolders, syncNow } from "./mail";

    interface Props {
        accounts: Account[];
        /** Whether mail is being fetched by the screen that is open. When it is not, it is fetched here. */
        fetched: boolean;
    }

    let { accounts, fetched }: Props = $props();

    /** For each account, when the newest message known of arrived. Only mail after it is new. */
    const newest = new Map<string, string>();

    async function look() {
        for (const account of accounts) {
            try {
                const inbox = (await listFolders(account.id)).find((folder) => folder.well_known === "inbox");
                if (!inbox) continue;
                const { messages } = await execute<{ messages: Message[] }>("mail.messages.list", { account_id: account.id, folder_id: inbox.id, limit: 20 });
                const latest = messages[0]?.received_at;
                const since = newest.get(account.id);
                if (latest && (!since || latest > since)) newest.set(account.id, latest);
                // The first look is what was already there: nothing in it is new.
                if (!since) continue;

                // Mail the mailbox has put under Other is left to be found, as it is in the list.
                const arrived = messages.filter((each) => each.received_at > since && !each.is_read && each.focused !== false);
                if (arrived.length > 3) notify(`${arrived.length} new messages`, account.display_name || account.username);
                else for (const each of arrived.reverse()) notify(each.from.name || each.from.address || "New message", each.subject || "(No subject)");
            } catch {
                // Looked for again in a while.
            }
        }
    }

    // What has arrived is looked for every half minute, on this computer
    // alone. Fetching is every two minutes, wherever the window is turned.
    $effect(() => {
        void accounts;
        look();
        const looking = setInterval(look, 30_000);
        const fetching = setInterval(() => !fetched && syncNow().catch(() => {}), 2 * 60_000);

        return () => {
            clearInterval(looking);
            clearInterval(fetching);
        };
    });
</script>
