import { expect, test } from "@playwright/test";
import { inbox, ran, refuse, robin, sam, withCore } from "./core";

/** Pictures for the README, taken only when asked for: `SCREENSHOTS=1 bun run test`. */
const picture = async (page: import("@playwright/test").Page, name: string) => {
    if (process.env.SCREENSHOTS) await page.screenshot({ path: `../docs/screenshots/${name}.png` });
};

test("with no account, the window says so and offers to add one", async ({ page }) => {
    await withCore(page);

    await expect(page.getByText("No account yet")).toBeVisible();
    await expect(page.getByRole("button", { name: "Sign in with your browser" })).toBeEnabled();
});

test("signing in adds the account to the list", async ({ page }) => {
    await withCore(page, { signIn: robin });

    await page.getByRole("button", { name: "Sign in with your browser" }).click();

    // With a first account, the window goes straight to its mail.
    await expect(page.getByRole("navigation", { name: "Folders" })).toBeVisible();
    await page.getByRole("button", { name: "Settings" }).click();
    const row = page.getByRole("list", { name: "Accounts" }).getByRole("listitem");
    await expect(row).toContainText("Robin Example");
    await expect(row).toContainText("robin@example.test");
    await expect(row).toContainText("Signed in");
});

test("while the browser has the sign-in, the window says to finish there", async ({ page }) => {
    await withCore(page, { signInHangs: true });

    await page.getByRole("button", { name: "Sign in with your browser" }).click();

    await expect(page.getByText("Finish signing in in your browser")).toBeVisible();
    await expect(page.getByRole("button", { name: "Sign in with your browser" })).toHaveCount(0);
});

test("a sign-in that fails shows why, in the core's own words", async ({ page }) => {
    const why = "Your organisation needs an administrator to approve Brolga before anyone can use it.";
    await withCore(page, { signIn: { fails: why } });

    await page.getByRole("button", { name: "Sign in with your browser" }).click();

    await expect(page.getByRole("alert")).toHaveText(why);
    await expect(page.getByText("No account yet")).toBeVisible();
});

test("with no app registration of Brolga's own, a client ID is asked for first", async ({ page }) => {
    await withCore(page, { needsClientId: true, signIn: robin });
    const signIn = page.getByRole("button", { name: "Sign in with your browser" });

    await expect(signIn).toBeDisabled();
    await expect(page.getByText("does not have its own app registration")).toBeVisible();

    await page.getByLabel("Client ID").fill("  11111111-2222-4333-8444-555555555555 ");
    await page.getByLabel("Tenant ID or domain").fill("contoso.example");
    await signIn.click();

    await page.getByRole("button", { name: "Settings" }).click();
    await expect(page.getByRole("list", { name: "Accounts" })).toContainText("Robin Example");
    const asked = (await ran(page)).find((r) => r.command === "accounts.sign_in");
    expect(asked?.args).toEqual({ client_id: "11111111-2222-4333-8444-555555555555", tenant: "contoso.example" });
});

test("signing in with a code shows the code and where to enter it", async ({ page }) => {
    await withCore(page, { signInHangs: true });

    await page.getByRole("button", { name: "Sign in with a code instead" }).click();

    await expect(page.getByLabel("Sign-in code")).toHaveText("ABCD1234");
    await expect(page.getByText("https://login.example.test/device")).toBeVisible();
    await picture(page, "sign-in-with-code");
});

test("a sign-in being waited for can be given up on, in the browser or with a code", async ({ page }) => {
    await withCore(page, { signInHangs: true });
    const browser = page.getByRole("button", { name: "Sign in with your browser" });

    await browser.click();
    await expect(page.getByText("Finish signing in in your browser")).toBeVisible();
    await page.getByRole("button", { name: "Cancel" }).click();
    await expect(page.getByText("Finish signing in in your browser")).toHaveCount(0);
    await expect(browser).toBeEnabled();

    await page.getByRole("button", { name: "Sign in with a code instead" }).click();
    await expect(page.getByLabel("Sign-in code")).toHaveText("ABCD1234");
    await page.getByRole("button", { name: "Cancel" }).click();
    await expect(page.getByLabel("Sign-in code")).toHaveCount(0);
    await expect(browser).toBeEnabled();
    await expect(page.getByRole("alert")).toHaveCount(0);
});

test("an account that needs it can sign in again, as itself", async ({ page }) => {
    await withCore(page, { accounts: [robin, sam], signIn: { ...sam, needs_sign_in: false } });
    await page.getByRole("button", { name: "Settings" }).click();
    const row = page.getByRole("listitem").filter({ hasText: "Sam Taylor" });

    await expect(row.getByRole("status")).toHaveText("Needs sign-in");
    await picture(page, "accounts");
    await row.getByRole("button", { name: "Sign in again" }).click();

    await expect(row).toContainText("Signed in");
    await expect(row.getByRole("button", { name: "Sign in again" })).toHaveCount(0);
    const asked = (await ran(page)).find((r) => r.command === "accounts.sign_in");
    expect(asked?.args).toEqual({ account_id: sam.id });
});

test("removing an account asks first", async ({ page }) => {
    await withCore(page, { accounts: [robin] });
    await page.getByRole("button", { name: "Settings" }).click();
    const row = page.getByRole("listitem").filter({ hasText: "Robin Example" });

    await row.getByRole("button", { name: "Remove" }).click();
    await expect(page.getByRole("list", { name: "Accounts" })).toBeVisible();
    expect((await ran(page)).some((r) => r.command === "accounts.remove")).toBe(false);

    await row.getByRole("button", { name: "Remove it?" }).click();

    await expect(page.getByText("No account yet")).toBeVisible();
});

test("the window works from the keyboard alone", async ({ page }) => {
    await withCore(page, { signIn: robin });

    const signIn = page.getByRole("button", { name: "Sign in with your browser" });
    // Past the sections of the window, and those of the settings that need no account.
    for (let presses = 0; presses < 12 && !(await signIn.evaluate((b) => b === document.activeElement)); presses++) {
        await page.keyboard.press("Tab");
    }
    await expect(signIn).toBeFocused();
    await page.keyboard.press("Enter");

    await expect(page.getByRole("navigation", { name: "Folders" })).toBeVisible();
});

test("automatic replies are scheduled from the accounts screen, in local time", async ({ page }) => {
    await withCore(page, { accounts: [robin] });
    await page.getByRole("button", { name: "Settings" }).click();
    await page.getByRole("button", { name: "Automatic replies" }).click();
    const form = page.getByRole("form", { name: "Automatic replies for robin@example.test" });

    await expect(form.getByRole("radio", { name: "Off", exact: true })).toBeChecked();
    await form.getByRole("radio", { name: "On between two times" }).check();
    // Brisbane is ten hours ahead of UTC, all year.
    await form.getByLabel("From", { exact: true }).fill("2026-10-20T08:00");
    await form.getByLabel("Until", { exact: true }).fill("2026-10-24T17:30");
    await form.getByLabel("To people in your organisation").fill("Away until Monday.");
    await form.getByLabel("Reply to people outside it").selectOption("everyone");
    await form.getByLabel("To people outside your organisation").fill("I am away until Monday.");
    await picture(page, "automatic-replies");
    await form.getByRole("button", { name: "Save" }).click();

    await expect(page.getByText("Automatic replies are scheduled.")).toBeVisible();
    expect((await ran(page)).find((r) => r.command === "mail.automatic_replies.set")?.args).toMatchObject({
        status: "scheduled",
        start: "2026-10-19T22:00:00Z",
        end: "2026-10-24T07:30:00Z",
        internal_message: "Away until Monday.",
        external_audience: "everyone",
    });

    // Opened again, the times are back in local time.
    await page.getByRole("button", { name: "Automatic replies" }).click();
    await expect(form.getByLabel("From", { exact: true })).toHaveValue("2026-10-20T08:00");
});

test("a sign-in from before automatic replies says to sign in again", async ({ page }) => {
    await withCore(page, { accounts: [robin], repliesRefused: "This sign-in was not given permission for mailbox settings. Sign in to the account again, from the accounts screen, to grant it." });
    await page.getByRole("button", { name: "Settings" }).click();
    await page.getByRole("button", { name: "Automatic replies" }).click();

    await expect(page.getByRole("alert")).toContainText("Sign in to the account again");
    await expect(page.getByRole("button", { name: "Save" })).toHaveCount(0);
});

test("what agents have asked for is shown, with what was turned away", async ({ page }) => {
    await withCore(page, {
        accounts: [robin],
        agentLog: [
            {
                at: "2026-10-09T05:02:00Z",
                client: "claude-code",
                command: "mail.drafts.send",
                args: { draft_id: "draft-3" },
                outcome: "refused",
                detail: "`mail.drafts.send` reaches other people or cannot be undone, so it needs the user's say-so.",
            },
            { at: "2026-10-09T05:01:00Z", client: "claude-code", command: "mail.drafts.create", args: { subject: "[14 characters]" }, outcome: "ok", detail: "" },
            { at: "2026-10-09T05:00:00Z", client: "claude-code", command: "mail.search", args: { query: "[7 characters]" }, outcome: "ok", detail: "" },
        ],
    });
    await page.getByRole("button", { name: "Agents" }).click();

    const rows = page.getByRole("row");
    await expect(rows).toHaveCount(4);
    await expect(rows.nth(1)).toContainText("mail.drafts.send");
    await expect(rows.nth(1)).toContainText("Not allowed");
    await expect(rows.nth(1)).toContainText("needs the user's say-so");
    await expect(rows.nth(2)).toContainText("Ran");
    await expect(rows.nth(3)).toContainText("[7 characters]");
    await picture(page, "agents");
});

test("with nothing asked yet, the agents screen says so", async ({ page }) => {
    await withCore(page, { accounts: [robin] });
    await page.getByRole("button", { name: "Agents" }).click();

    await expect(page.getByText("No agent has asked for anything yet.")).toBeVisible();
});

test("an agent that wants to send is put to the user, who can say no", async ({ page }) => {
    await withCore(page, {
        accounts: [robin],
        asking: [
            { id: 1, client: "claude-code", command: "mail.drafts.send", args: { draft_id: "draft-7" } },
            { id: 2, client: "claude-code", command: "mail.messages.delete_permanently", args: { message_ids: ["m1"] } },
        ],
    });

    const dialog = page.getByRole("alertdialog");
    await expect(dialog).toContainText("claude-code wants to send a message");
    await expect(dialog).toContainText("1 more waiting.");
    // Saying no is where the keyboard starts.
    await expect(dialog.getByRole("button", { name: "Don't allow" })).toBeFocused();
    await picture(page, "agent-asks");

    await dialog.getByRole("button", { name: "Don't allow" }).click();
    await expect(dialog).toContainText("wants to delete messages for good");
    await dialog.getByRole("button", { name: "Allow this once" }).click();
    await expect(dialog).toHaveCount(0);

    const answers = (await ran(page)).filter((r) => r.command === "app.confirmations.answer").map((r) => r.args);
    expect(answers).toEqual([
        { id: 1, allow: false },
        { id: 2, allow: true },
    ]);
});

test("a shared mailbox is added by its address, read in Mail, and removed", async ({ page }) => {
    await withCore(page, { accounts: [robin], mail: inbox });
    await page.getByRole("button", { name: "Settings" }).click();
    await page.getByRole("button", { name: "Shared mailboxes" }).click();
    const section = page.getByRole("region", { name: "Shared mailboxes of robin@example.test" });

    // One there is no access to is not added, and says why.
    await section.getByRole("textbox", { name: "Address of a shared mailbox" }).fill("payroll@example.test");
    await section.getByRole("button", { name: "Add" }).click();
    await expect(section.getByRole("alert")).toContainText("not been given access");

    await section.getByRole("textbox", { name: "Address of a shared mailbox" }).fill("support@example.test");
    await section.getByRole("button", { name: "Add" }).click();
    await expect(section.getByRole("listitem")).toContainText("Support · support@example.test");
    await expect(section.getByRole("alert")).toHaveCount(0);

    // In Mail it is chosen beside the account, and what is asked for is asked of it.
    await page.getByRole("button", { name: "Mail", exact: true }).click();
    const picker = page.getByRole("combobox", { name: "Account" });
    await expect(picker.getByRole("option")).toHaveText(["robin@example.test", "↳ support@example.test (shared)"]);
    await picker.selectOption({ label: "↳ support@example.test (shared)" });
    await expect.poll(async () => (await ran(page)).filter((r) => r.command === "mail.folders.list").at(-1)?.args).toMatchObject({ account_id: "tenant-1.robin~support@example.test" });

    // Its folders are beside the account's own, whichever of the two is shown, and opening one of them goes to that mailbox.
    const folders = page.getByRole("navigation", { name: "Folders" });
    await expect(folders.getByRole("list", { name: "robin@example.test" }).getByRole("button", { name: "Inbox" })).toBeVisible();
    await picker.selectOption({ label: "robin@example.test" });
    const theirs = folders.getByRole("list", { name: "support@example.test" });
    await expect(theirs.getByRole("button", { name: "Inbox" })).toBeVisible();
    await picture(page, "shared-mailbox-folders");
    await theirs.getByRole("button", { name: "Archive" }).click();
    await expect(picker).toHaveValue("tenant-1.robin~support@example.test");
    await expect.poll(async () => (await ran(page)).filter((r) => r.command === "mail.messages.list").at(-1)?.args).toMatchObject({ account_id: "tenant-1.robin~support@example.test", folder_id: "shared-archive" });

    await page.getByRole("button", { name: "Settings" }).click();
    await page.getByRole("button", { name: "Shared mailboxes" }).click();
    await page.getByRole("button", { name: "Remove support@example.test" }).click();
    await page.getByRole("button", { name: "Mail", exact: true }).click();
    await expect(page.getByRole("combobox", { name: "Account" })).toHaveCount(0);
});

test("the mailbox shown has its folders first, and another's are under them, closed up as they were left", async ({ page }) => {
    const shared = [{ id: "tenant-1.robin~support@example.test", account_id: "tenant-1.robin", address: "support@example.test", name: "Support" }];
    await withCore(page, { accounts: [robin], mail: inbox, shared });

    // A folder of the account is closed up while the account is the one shown.
    const folders = page.getByRole("navigation", { name: "Folders" });
    const shown = folders.getByRole("list", { name: "Folders" });
    await shown.getByRole("button", { name: "Collapse Projects" }).click();
    await expect(shown.getByRole("button", { name: "Harbour upgrade", exact: true })).toHaveCount(0);

    // With the shared mailbox shown, its own folders come first and the account's are under them.
    await page.getByRole("combobox", { name: "Account" }).selectOption({ label: "↳ support@example.test (shared)" });
    const mine = folders.getByRole("list", { name: "robin@example.test" });
    await expect(mine.getByRole("button", { name: "Inbox" })).toBeVisible();
    const top = await shown.boundingBox();
    const under = await mine.boundingBox();
    expect(under!.y).toBeGreaterThan(top!.y + top!.height - 1);

    // What was closed up is still closed, and is opened where it is.
    await expect(mine.getByRole("button", { name: "Harbour upgrade", exact: true })).toHaveCount(0);
    await mine.getByRole("button", { name: "Expand Projects" }).click();
    await expect(mine.getByRole("button", { name: "Harbour upgrade", exact: true })).toBeVisible();

    // The whole of the other mailbox closes up under its name.
    const name = folders.getByRole("button", { name: "robin@example.test" });
    await expect(name).toHaveAttribute("aria-expanded", "true");
    await name.click();
    await expect(name).toHaveAttribute("aria-expanded", "false");
    await expect(mine).toHaveCount(0);
});

test("a shared mailbox's folder stays open when the account's folders are answered late", async ({ page }) => {
    const shared = [{ id: "tenant-1.robin~support@example.test", account_id: "tenant-1.robin", address: "support@example.test", name: "Support" }];
    await withCore(page, { accounts: [robin], mail: inbox, shared, syncNeverEnds: true });

    const folders = page.getByRole("navigation", { name: "Folders" });
    const theirs = folders.getByRole("list", { name: "support@example.test" });
    await expect(theirs.getByRole("button", { name: "Archive" })).toBeVisible();

    // While it syncs, the account's folders are asked for again and again. One such answer is still on its way...
    await page.evaluate(() => (window as unknown as { holdFolders: (account: string) => void }).holdFolders("tenant-1.robin"));
    await expect.poll(() => page.evaluate(() => (window as unknown as { foldersHeld: () => number }).foldersHeld()), { timeout: 10_000 }).toBeGreaterThan(0);

    // ...when a folder of the shared mailbox is opened, and it comes after.
    await theirs.getByRole("button", { name: "Archive" }).click();
    const picker = page.getByRole("combobox", { name: "Account" });
    await expect(picker).toHaveValue("tenant-1.robin~support@example.test");
    const open = folders.getByRole("list", { name: "Folders" }).getByRole("button", { name: "Archive", exact: true });
    await expect(open).toHaveAttribute("aria-current", "true");
    await page.evaluate(() => (window as unknown as { letFoldersGo: () => void }).letFoldersGo());

    // The folder opened is still the one shown, and the account's folders are beside it, not in its place.
    await expect(folders.getByRole("list", { name: "robin@example.test" }).getByRole("button", { name: "Inbox" })).toBeVisible();
    await expect(open).toHaveAttribute("aria-current", "true");
    expect((await ran(page)).filter((r) => r.command === "mail.messages.list").at(-1)?.args).toMatchObject({ account_id: "tenant-1.robin~support@example.test", folder_id: "shared-archive" });
});

test("a message can be sent under a shared mailbox's name, chosen where it is written", async ({ page }) => {
    const shared = [{ id: "tenant-1.robin~support@example.test", account_id: "tenant-1.robin", address: "support@example.test", name: "Support" }];
    await withCore(page, { accounts: [robin], mail: inbox, shared });
    await page.getByRole("button", { name: "New message" }).click();
    const from = page.getByRole("combobox", { name: "From" });

    await expect(from.getByRole("option")).toHaveText(["robin@example.test", "support@example.test"]);
    await expect(from).toHaveValue("robin@example.test");
    await from.selectOption("support@example.test");
    await expect.poll(async () => (await ran(page)).filter((r) => r.command === "mail.drafts.edit").at(-1)?.args).toMatchObject({ from: "support@example.test" });
    // Put back to one's own, it is said as no choice at all.
    await from.selectOption("robin@example.test");
    await expect.poll(async () => (await ran(page)).filter((r) => r.command === "mail.drafts.edit").at(-1)?.args).toMatchObject({ from: "" });
});

test("with no shared mailbox there is no choice of who a message is from", async ({ page }) => {
    await withCore(page, { accounts: [robin], mail: inbox });
    await page.getByRole("button", { name: "New message" }).click();
    await expect(page.getByRole("textbox", { name: "Subject" })).toBeVisible();
    await expect(page.getByRole("combobox", { name: "From" })).toHaveCount(0);
});

test("rules are listed, turned off, made, changed and deleted", async ({ page }) => {
    const rules = [
        {
            id: "rule-1",
            name: "Newsletters",
            enabled: true,
            sequence: 1,
            read_only: false,
            conditions: { senderContains: ["newsletter"] },
            exceptions: {},
            actions: { markAsRead: true },
            summary: "When the sender contains “newsletter”: mark it read.",
        },
        {
            id: "rule-2",
            name: "Voicemail",
            enabled: true,
            sequence: 2,
            read_only: false,
            conditions: { isVoicemail: true },
            exceptions: {},
            actions: { copyToFolder: "archive" },
            summary: "When it is a voicemail: copy it to Archive.",
        },
        {
            id: "rule-3",
            name: "Set by the organisation",
            enabled: true,
            sequence: 3,
            read_only: true,
            conditions: {},
            exceptions: {},
            actions: { delete: true },
            summary: "For every message: delete it.",
        },
    ];
    await withCore(page, { accounts: [robin], mail: inbox, rules });
    await page.getByRole("button", { name: "Settings" }).click();
    await page.getByRole("button", { name: "Rules" }).click();
    const section = page.getByRole("region", { name: "Rules for robin@example.test" });
    const rows = section.getByRole("listitem");

    await expect(rows).toHaveCount(3);
    await expect(rows.nth(0)).toContainText("When the sender contains “newsletter”: mark it read.");
    // One with a part the form cannot hold is not offered for changing, so saving cannot lose that part.
    await expect(section.getByRole("button", { name: "Change Voicemail" })).toHaveCount(0);
    await expect(section.getByRole("button", { name: "Delete Voicemail" })).toBeVisible();
    // One the server holds read-only is only shown.
    await expect(rows.nth(2)).toContainText("Read-only");
    await expect(rows.nth(2).getByRole("button")).toHaveCount(0);
    await expect(rows.nth(2).getByRole("checkbox")).toBeDisabled();

    await section.getByRole("checkbox", { name: "Newsletters is on" }).uncheck();
    await expect.poll(async () => (await ran(page)).find((r) => r.command === "mail.rules.update")?.args).toMatchObject({ rule_id: "rule-1", enabled: false });

    await section.getByRole("button", { name: "New rule" }).click();
    const form = section.getByRole("form", { name: "New rule" });
    await form.getByLabel("Name").fill("Receipts");
    await form.getByLabel("The subject contains").fill("receipt, tax invoice");
    await form.getByLabel("It has attachments").check();
    // A rule that does nothing is not sent.
    await form.getByRole("button", { name: "Save rule" }).click();
    await expect(section.getByRole("alert")).toContainText("at least one thing");
    await form.getByLabel("Move it to").selectOption({ label: "Archive" });
    await form.getByLabel("Categorise it").selectOption("Clients");
    await form.getByLabel("Forward it to").fill("books@example.test");
    await picture(page, "rules");
    await form.getByRole("button", { name: "Save rule" }).click();

    await expect(rows).toHaveCount(4);
    const made = (await ran(page)).find((r) => r.command === "mail.rules.create")?.args;
    expect(made).toMatchObject({
        name: "Receipts",
        conditions: { subjectContains: ["receipt", "tax invoice"], hasAttachments: true },
        actions: { moveToFolder: "archive", assignCategories: ["Clients"], forwardTo: ["books@example.test"] },
    });

    // Changing one starts from what it holds, and sends each part whole.
    await section.getByRole("button", { name: "Change Receipts" }).click();
    const changing = section.getByRole("form", { name: "Change rule" });
    await expect(changing.getByLabel("The subject contains")).toHaveValue("receipt, tax invoice");
    await expect(changing.getByLabel("Move it to")).toHaveValue("archive");
    await changing.getByLabel("Forward it to").fill("");
    await changing.getByLabel("Mark it read").check();
    await changing.getByRole("button", { name: "Save rule" }).click();
    await expect.poll(async () => (await ran(page)).filter((r) => r.command === "mail.rules.update").at(-1)?.args).toMatchObject({
        rule_id: "rule-4",
        name: "Receipts",
        actions: { moveToFolder: "archive", assignCategories: ["Clients"], markAsRead: true },
    });
    expect(((await ran(page)).filter((r) => r.command === "mail.rules.update").at(-1)?.args as { actions: object }).actions).not.toHaveProperty("forwardTo");

    // Deleting asks first.
    await section.getByRole("button", { name: "Delete Voicemail" }).click();
    expect((await ran(page)).some((r) => r.command === "mail.rules.delete")).toBe(false);
    await section.getByRole("button", { name: "Delete it?" }).click();
    await expect(rows).toHaveCount(3);
});

test("mail instructions are written, ordered, turned off and deleted, and one computer is chosen to carry them out", async ({ page }) => {
    const instructions = [{ id: "i-1", name: "Newsletters", text: "Newsletters are marked read.", enabled: true, actions: ["set_read"] }];
    const shared = [{ id: `${robin.id}~support@example.test`, account_id: robin.id, address: "support@example.test", name: "Support" }];
    await withCore(page, { accounts: [robin], mail: inbox, shared, instructions, instructionsProcessor: { device_id: "device-laptop", device_name: "laptop", since: "2026-10-15T08:00:00Z" } });
    await page.getByRole("button", { name: "Settings" }).click();
    await page.getByRole("button", { name: "Instructions" }).click();
    const section = page.getByRole("region", { name: "Mail instructions for robin@example.test" });
    const rows = section.getByRole("listitem");

    // A shared mailbox keeps instructions of its own, so it is offered too.
    await expect(page.getByRole("combobox", { name: "Account" }).getByRole("option")).toHaveText(["robin@example.test", "support@example.test"]);
    await expect(rows).toHaveCount(1);
    await expect(section.getByLabel("Processing device")).toContainText("laptop carries out these instructions");

    await section.getByRole("button", { name: "New instruction" }).click();
    const form = section.getByRole("form", { name: "New instruction" });
    await form.getByLabel("Name", { exact: true }).fill("Telstra invoices");
    await form.getByLabel("The instruction, in plain words").fill("Invoices from Telstra go in Accounts/Telstra and are categorised Bills.");
    // An instruction that may do nothing is not sent.
    await form.getByRole("button", { name: "Save instruction" }).click();
    await expect(section.getByRole("alert")).toContainText("at least one thing");
    await form.getByLabel("Move it to a folder the instruction names").check();
    await form.getByLabel("Give it categories").check();
    await picture(page, "instructions");
    await form.getByRole("button", { name: "Save instruction" }).click();
    await expect(rows).toHaveCount(2);
    expect((await ran(page)).find((r) => r.command === "mail.instructions.set")?.args).toMatchObject({
        name: "Telstra invoices",
        actions: ["move", "set_categories"],
        enabled: true,
        version: 1,
    });

    await section.getByRole("checkbox", { name: "Newsletters is on" }).uncheck();
    await expect.poll(async () => (await ran(page)).filter((r) => r.command === "mail.instructions.set").at(-1)?.args).toMatchObject({ instruction_id: "i-1", enabled: false, version: 2 });

    await section.getByRole("button", { name: "Move Newsletters down" }).click();
    await expect(rows.nth(0)).toContainText("Telstra invoices");
    expect((await ran(page)).find((r) => r.command === "mail.instructions.reorder")?.args).toMatchObject({ instruction_id: "i-1", position: 2 });

    // Changed on another computer meanwhile, a change is refused and what was written stays in the form.
    await refuse(page, "mail.instructions.set", "The instructions were changed on laptop since they were read, so this change was not made.");
    await section.getByRole("button", { name: "Change Telstra invoices" }).click();
    const changing = section.getByRole("form", { name: "Change instruction" });
    await changing.getByLabel("Name", { exact: true }).fill("Telstra");
    await changing.getByRole("button", { name: "Save instruction" }).click();
    await expect(section.getByRole("alert")).toContainText("changed on laptop");
    await expect(changing.getByLabel("Name", { exact: true })).toHaveValue("Telstra");
    await changing.getByRole("button", { name: "Cancel" }).click();
    await refuse(page, "mail.instructions.set", null);

    // Deleting asks first.
    await section.getByRole("button", { name: "Delete Newsletters" }).click();
    expect((await ran(page)).some((r) => r.command === "mail.instructions.delete")).toBe(false);
    await section.getByRole("button", { name: "Delete it?" }).click();
    await expect(rows).toHaveCount(1);

    // Taking the job from another computer asks first, and says which.
    const device = section.getByLabel("Processing device");
    await device.getByRole("button", { name: "Carry them out on this computer instead" }).click();
    await expect(device).toContainText("Move the job from laptop to this computer (titan)?");
    expect((await ran(page)).some((r) => r.command === "mail.instructions.processor.take")).toBe(false);
    await device.getByRole("button", { name: "Yes, on this computer" }).click();
    await expect(device).toContainText("This computer (titan) carries out these instructions.");

    await device.getByRole("button", { name: "Rename" }).click();
    await device.getByLabel("This computer's name").fill("Workshop PC");
    await device.getByRole("button", { name: "Save" }).click();
    await expect(device).toContainText("This computer (Workshop PC) carries out these instructions.");
});

test("the model is chosen, agreed to and tried, and what the instructions did can be undone", async ({ page }) => {
    const instructions = [{ id: "i-1", name: "Telstra", text: "Invoices from Telstra go in Accounts/Telstra.", enabled: true, actions: ["move"] }];
    const instructionsLog = [
        { at: "2026-10-16T09:00:00Z", command: "mail.messages.move", outcome: "ok", detail: "A Telstra invoice.", subject: "Your Telstra invoice", instruction: "Telstra", undo_id: "undo-1" },
        { at: "2026-10-16T09:00:00Z", command: "mail.instructions.move", outcome: "refused", detail: "the instruction does not allow it", subject: "Free phones!!!", instruction: null, undo_id: null },
    ];
    await withCore(page, { accounts: [robin], mail: inbox, instructions, instructionsLog, instructionsProcessor: { device_id: "device-here", device_name: "titan", since: "2026-10-15T08:00:00Z" } });
    await page.getByRole("button", { name: "Settings" }).click();
    await page.getByRole("button", { name: "Instructions" }).click();
    const section = page.getByRole("region", { name: "Mail instructions for robin@example.test" });
    const model = section.getByLabel("Model", { exact: true });

    await expect(model).toContainText("No model has been chosen yet.");
    await model.getByRole("button", { name: "Choose a model" }).click();
    const form = section.getByRole("form", { name: "Choose the model" });
    await expect(form.getByLabel("Model", { exact: true })).toHaveValue("claude-haiku-5-5");
    await form.getByRole("textbox", { name: "API key" }).fill("sk-ant-mine");
    // Agreeing is the person's, and says what goes.
    await expect(form.getByText("Never what is in the attachments.")).toBeVisible();
    await form.getByRole("button", { name: "Save" }).click();
    await expect(model).toContainText("has not been agreed to, so nothing is sent");
    await expect(section.getByLabel("Processing", { exact: true })).toContainText("Nothing is being done");
    expect((await ran(page)).find((r) => r.command === "mail.instructions.model.set_key")?.args).toEqual({ key: "sk-ant-mine" });

    await model.getByRole("button", { name: "Change the model" }).click();
    await form.getByLabel("Send this mailbox's mail to Anthropic to be read").check();
    await picture(page, "instructions-model");
    await form.getByRole("button", { name: "Save" }).click();
    await expect(model).toContainText("claude-haiku-5-5 at Anthropic reads the mail.");
    expect((await ran(page)).filter((r) => r.command === "mail.instructions.model.set").at(-1)?.args).toMatchObject({ provider: "anthropic", cloud_consent: true });
    await expect(section.getByLabel("Processing", { exact: true })).toContainText("7 sent to the model today, of at most 500");

    await model.getByRole("button", { name: "Try it" }).click();
    await expect(model).toContainText("answered properly in 1.2 seconds");

    // A local model is picked from those the server has.
    await model.getByRole("button", { name: "Change the model" }).click();
    await form.getByLabel("A model on an Ollama server").check();
    await expect(form.getByLabel("Model", { exact: true })).toHaveValue("llama3.2");
    await form.getByLabel("Model", { exact: true }).selectOption("qwen3");
    await form.getByRole("button", { name: "Save" }).click();
    await expect(model).toContainText("qwen3 on Ollama reads the mail.");

    await section.getByText("What the instructions have done").click();
    const log = section.getByRole("list", { name: "What the instructions have done" });
    await expect(log.getByRole("listitem").nth(0)).toContainText("Moved: Your Telstra invoice");
    await expect(log.getByRole("listitem").nth(1)).toContainText("Not done");
    await log.getByRole("button", { name: "Undo" }).click();
    await expect(log.getByRole("button", { name: "Undone" })).toBeDisabled();
    expect((await ran(page)).find((r) => r.command === "mail.undo")?.args).toMatchObject({ undo_id: "undo-1" });
});

test("mail instructions that cannot be read say why", async ({ page }) => {
    await withCore(page, { accounts: [robin], offline: true });
    await page.getByRole("button", { name: "Settings" }).click();
    await page.getByRole("button", { name: "Instructions" }).click();

    const section = page.getByRole("region", { name: "Mail instructions for robin@example.test" });
    await expect(section.getByRole("alert")).toContainText("could not be reached");
    await expect(section.getByRole("button", { name: "New instruction" })).toHaveCount(0);
});

test("rules that cannot be read say why", async ({ page }) => {
    await withCore(page, { accounts: [robin], offline: true });
    await page.getByRole("button", { name: "Settings" }).click();
    await page.getByRole("button", { name: "Rules" }).click();

    const section = page.getByRole("region", { name: "Rules for robin@example.test" });
    await expect(section.getByRole("alert")).toContainText("could not be reached");
    await expect(section.getByRole("button", { name: "New rule" })).toHaveCount(0);
});

test("categories are added, recoloured and deleted", async ({ page }) => {
    await withCore(page, { accounts: [robin], mail: inbox });
    await page.getByRole("button", { name: "Settings" }).click();
    await page.getByRole("button", { name: "Categories" }).click();
    const section = page.getByRole("region", { name: "Categories for robin@example.test" });
    const rows = section.getByRole("listitem");
    await expect(rows).toHaveCount(3);

    await section.getByRole("textbox", { name: "Name of a new category" }).fill("Training");
    await section.getByRole("combobox", { name: "Colour of the new category" }).selectOption({ label: "Green" });
    await section.getByRole("button", { name: "Add" }).click();
    await expect(rows).toHaveCount(4);
    expect((await ran(page)).find((r) => r.command === "mail.categories.create")?.args).toMatchObject({ name: "Training", color: "preset4" });

    // A second of the same name is refused, in the core's words.
    await section.getByRole("textbox", { name: "Name of a new category" }).fill("training");
    await section.getByRole("button", { name: "Add" }).click();
    await expect(section.getByRole("alert")).toContainText("already a category");

    await section.getByRole("combobox", { name: "Colour of Training" }).selectOption({ label: "Dark blue" });
    await expect.poll(async () => (await ran(page)).find((r) => r.command === "mail.categories.set_color")?.args).toMatchObject({ name: "Training", color: "preset22" });
    await expect(section.getByRole("alert")).toHaveCount(0);
    await picture(page, "category-list");

    await section.getByRole("button", { name: "Delete Plain" }).click();
    await section.getByRole("button", { name: "Delete it?" }).click();
    await expect(rows).toHaveCount(3);
    await expect(section).not.toContainText("Plain");
});

test("a person who shared only their calendar is added the same way, and is in Calendar and not in Mail", async ({ page }) => {
    await withCore(page, { accounts: [robin], mail: inbox });
    await page.getByRole("button", { name: "Settings" }).click();
    await page.getByRole("button", { name: "Shared mailboxes" }).click();
    const section = page.getByRole("region", { name: "Shared mailboxes of robin@example.test" });
    await section.getByRole("textbox", { name: "Address of a shared mailbox" }).fill("alex@example.test");
    await section.getByRole("button", { name: "Add" }).click();
    await expect(section.getByRole("listitem")).toContainText("Alex Chen · alex@example.test (calendar only)");

    await page.getByRole("button", { name: "Mail", exact: true }).click();
    await expect(page.getByRole("navigation", { name: "Folders" })).toBeVisible();
    await expect(page.getByRole("combobox", { name: "Account" })).toHaveCount(0);

    await page.getByRole("button", { name: "Calendar", exact: true }).click();
    await expect(page.getByRole("combobox", { name: "Account" }).getByRole("option")).toHaveText(["robin@example.test", "↳ alex@example.test (shared)"]);
});

test("the calendar is shared with someone, what they see is changed, and the sharing is stopped", async ({ page }) => {
    await withCore(page, { accounts: [robin] });
    await page.getByRole("button", { name: "Settings" }).click();
    await page.getByRole("button", { name: "Calendar sharing" }).click();
    const section = page.getByRole("region", { name: "Calendar sharing for robin@example.test" });
    const rows = section.getByRole("listitem");

    // The organisation as a whole is there from the start, and cannot be removed.
    await expect(rows).toHaveCount(1);
    await expect(rows.first()).toContainText("Everyone in your organisation");
    await expect(rows.first().getByRole("button")).toHaveCount(0);
    await expect(section.getByRole("combobox", { name: "What Everyone in your organisation can see" })).toHaveValue("freeBusyRead");

    await section.getByRole("textbox", { name: "Address of someone to share with" }).fill("sam@example.test");
    await section.getByRole("combobox", { name: "What they can see" }).selectOption({ label: "Titles and places" });
    await section.getByRole("button", { name: "Share" }).click();
    await expect(rows).toHaveCount(2);
    expect((await ran(page)).find((r) => r.command === "calendar.sharing.set")?.args).toMatchObject({ address: "sam@example.test", role: "limitedRead" });

    await section.getByRole("combobox", { name: "What sam@example.test can see" }).selectOption({ label: "Can change it" });
    await expect.poll(async () => (await ran(page)).filter((r) => r.command === "calendar.sharing.set").at(-1)?.args).toMatchObject({ share_id: "share-1", role: "write" });
    await picture(page, "calendar-sharing");

    // Someone outside is offered less, and asking for more says why not.
    await section.getByRole("textbox", { name: "Address of someone to share with" }).fill("pat@elsewhere.test");
    await section.getByRole("combobox", { name: "What they can see" }).selectOption({ label: "Can change it" });
    await section.getByRole("button", { name: "Share" }).click();
    await expect(section.getByRole("alert")).toContainText("will not let them");
    await expect(rows).toHaveCount(2);

    await section.getByRole("button", { name: "Stop sharing with sam@example.test" }).click();
    await expect(rows).toHaveCount(1);
});

test("a PST file is read into an archive, with how far it has got, and then is read in Mail", async ({ page }) => {
    await withCore(page, { accounts: [robin], mail: inbox, pickFiles: ["/home/robin/Old mail.pst"] });
    await page.getByRole("button", { name: "Settings" }).click();
    await page.getByRole("navigation", { name: "Settings" }).getByRole("button", { name: "Archives" }).click();
    const section = page.getByRole("region", { name: "Archives" });
    await expect(section.getByRole("listitem")).toHaveCount(0);

    await section.getByRole("button", { name: "Import a PST file…" }).click();
    const row = section.getByRole("listitem");
    await expect(row).toContainText("Old mail");
    // While it is read, how far it has got is said and it can be stopped.
    await expect(row.getByRole("status")).toContainText(/Reading: [\d,]+ of 1,500 messages/);
    await expect(row.getByRole("progressbar", { name: "Reading Old mail" })).toBeVisible();
    await expect(row.getByRole("button", { name: "Stop reading Old mail" })).toBeVisible();
    await picture(page, "archive-importing");
    // Then it is done, and says how much is in it.
    await expect(row).toContainText("1,500 messages", { timeout: 8000 });
    await expect(row.getByRole("progressbar")).toHaveCount(0);
    expect((await ran(page)).find((r) => r.command === "archives.import")?.args).toMatchObject({ path: "/home/robin/Old mail.pst" });

    // It is in Mail beside the account, and what is asked for is asked of it.
    await page.getByRole("button", { name: "Mail", exact: true }).click();
    const picker = page.getByRole("combobox", { name: "Account" });
    await expect(picker.getByRole("option")).toHaveText(["robin@example.test", "Old mail (archive)"]);
    await picker.selectOption({ label: "Old mail (archive)" });
    await expect.poll(async () => (await ran(page)).filter((r) => r.command === "mail.folders.list").at(-1)?.args).toMatchObject({ account_id: "archive:1" });
    // Nothing can be written from an archive, and it says where to instead.
    await page.getByRole("button", { name: "New message" }).click();
    await expect(page.getByRole("alert")).toContainText("nothing can be sent from it");
    expect((await ran(page)).some((r) => r.command === "mail.drafts.create" && r.args?.account_id === "archive:1")).toBe(false);

    // Removing asks first.
    await page.getByRole("button", { name: "Settings" }).click();
    await section.getByRole("button", { name: "Remove Old mail" }).click();
    expect((await ran(page)).some((r) => r.command === "archives.remove")).toBe(false);
    await section.getByRole("button", { name: "Remove it?" }).click();
    await expect(section.getByRole("listitem")).toHaveCount(0);
});

test("Brolga is kept running and started at sign-in only when chosen, and says when there is no tray", async ({ page }) => {
    await withCore(page, { accounts: [robin], noTray: true });
    await page.getByRole("button", { name: "Settings" }).click();
    await page.getByRole("navigation", { name: "Settings" }).getByRole("button", { name: "Start-up and updates" }).click();
    const here = page.getByRole("region", { name: "This computer" });
    const applied = () => page.evaluate(() => (window as unknown as { applied: unknown[] }).applied.at(-1));

    await expect(here.getByRole("checkbox", { name: /Keep running/ })).not.toBeChecked();
    await expect(here.getByRole("checkbox", { name: /Start when I sign in/ })).not.toBeChecked();

    await here.getByRole("checkbox", { name: /Start when I sign in/ }).check();
    await expect.poll(applied).toMatchObject({ keep_running: false, start_at_login: true });
    await here.getByRole("checkbox", { name: /Keep running/ }).check();
    await expect.poll(applied).toMatchObject({ keep_running: true, start_at_login: true });
    await expect(here.getByRole("status")).toContainText("no system tray");
    await picture(page, "this-computer");
});

test("an import can be stopped, and one that failed says why", async ({ page }) => {
    const failed = { id: "archive:9", name: "Broken", source: "Broken.pst", state: "failed", messages: 0, messages_done: null, messages_total: null, skipped: 0, error: "this is not a PST or OST file, or it is damaged" };
    await withCore(page, { accounts: [robin], archives: [failed], pickFiles: ["/home/robin/Huge.pst"] });
    await page.getByRole("button", { name: "Settings" }).click();
    await page.getByRole("navigation", { name: "Settings" }).getByRole("button", { name: "Archives" }).click();
    const section = page.getByRole("region", { name: "Archives" });
    await expect(section.getByRole("listitem").filter({ hasText: "Broken" })).toContainText("Failed: this is not a PST or OST file");

    await section.getByRole("button", { name: "Import a PST file…" }).click();
    await section.getByRole("button", { name: "Stop reading Huge" }).click();
    await expect(section.getByRole("listitem").filter({ hasText: "Huge" })).toContainText(/Stopped: [\d,]+ messages? were read/);
    expect((await ran(page)).find((r) => r.command === "archives.cancel")?.args).toMatchObject({ archive_id: "archive:2" });
});

test("looking for new versions is on until it is turned off, and then nothing is looked for unasked", async ({ page }) => {
    await withCore(page, { accounts: [robin], newer: { version: "0.1.6", installs: true, page: "https://example.test/releases" } });
    await page.getByRole("button", { name: "Settings" }).click();
    await page.getByRole("navigation", { name: "Settings" }).getByRole("button", { name: "Start-up and updates" }).click();
    const box = page.getByRole("region", { name: "This computer" }).getByRole("checkbox", { name: /Look for new versions/ });
    await expect(box).toBeChecked();

    await box.uncheck();
    await expect(box).not.toBeChecked();
    expect((await ran(page)).filter((r) => r.command === "app.desktop.set").at(-1)?.args).toEqual({ check_for_updates: false });
});

const contact = (id: string, name: string, more: Record<string, unknown> = {}) => ({
    id,
    folder_id: "",
    name,
    given_name: name.split(" ")[0],
    surname: name.split(" ")[1] ?? "",
    nickname: "",
    company: "",
    job_title: "",
    department: "",
    office: "",
    emails: [],
    business_phones: [],
    home_phones: [],
    mobile: "",
    notes: "",
    birthday: null,
    ...more,
});

test("with two accounts, whose contacts are shown is chosen, and each is asked for its own", async ({ page }) => {
    const second = { ...sam, needs_sign_in: false };
    await withCore(page, { accounts: [robin, second], contacts: [contact("c1", "Sam Taylor")] });
    await page.getByRole("button", { name: "People" }).click();
    const whose = page.getByRole("complementary", { name: "Folders of contacts" }).getByLabel("Account");
    const list = page.getByRole("list", { name: "Contacts" });

    await expect(whose).toHaveValue(robin.id);
    await list.getByRole("button", { name: /Sam Taylor/ }).click();
    await whose.selectOption({ label: second.username });

    await expect.poll(async () => (await ran(page)).filter((r) => r.command === "contacts.list").at(-1)?.args.account_id).toBe(second.id);
    // Nobody of the other account's is left open.
    await expect(page.getByRole("region", { name: "Contact" })).toContainText("Choose a contact");
});

test("with one account there is no choice of whose contacts", async ({ page }) => {
    await withCore(page, { accounts: [robin] });
    await page.getByRole("button", { name: "People" }).click();

    await expect(page.getByRole("button", { name: "New contact" })).toBeVisible();
    await expect(page.getByRole("complementary", { name: "Folders of contacts" }).getByLabel("Account")).toHaveCount(0);
});

test("Escape closes a contact form with nothing written in it, and asks before one with something is thrown away", async ({ page }) => {
    await withCore(page, { accounts: [robin], contacts: [contact("c1", "Sam Taylor")] });
    await page.getByRole("button", { name: "People" }).click();
    const form = page.getByRole("form", { name: "New contact" });
    const asked = page.getByRole("alertdialog", { name: "Discard the contact" });

    await page.getByRole("button", { name: "New contact" }).click();
    await page.keyboard.press("Escape");
    await expect(form).toHaveCount(0);

    await page.getByRole("button", { name: "New contact" }).click();
    await form.getByLabel("First name").fill("Kim");
    await page.keyboard.press("Escape");
    await expect(asked).toBeVisible();
    await asked.getByRole("button", { name: "Keep" }).click();
    await expect(form.getByLabel("First name")).toHaveValue("Kim");

    // Escape in the search box is for the search.
    await page.getByRole("searchbox", { name: "Search people" }).press("Escape");
    await expect(asked).toHaveCount(0);
    await expect(form.getByLabel("First name")).toHaveValue("Kim");

    await form.getByLabel("Last name").press("Escape");
    await asked.getByRole("button", { name: "Discard for good?" }).click();
    await expect(form).toHaveCount(0);
});

test("contacts that could not be read say so only until they are read", async ({ page }) => {
    await withCore(page, { accounts: [robin], contacts: [contact("c1", "Sam Taylor")] });
    await page.getByRole("button", { name: "People" }).click();
    const list = page.getByRole("region", { name: "People" });
    await expect(list.getByRole("button", { name: /Sam Taylor/ })).toBeVisible();

    await refuse(page, "contacts.list", "Your contacts could not be read just now.");
    await page.getByRole("searchbox", { name: "Search people" }).fill("s");
    await expect(list.getByRole("alert")).toHaveText("Your contacts could not be read just now.");

    await refuse(page, "contacts.list", null);
    await page.getByRole("searchbox", { name: "Search people" }).fill("sa");
    await expect(list.getByRole("button", { name: /Sam Taylor/ })).toBeVisible();
    await expect(list.getByRole("alert")).toHaveCount(0);
});

test("a contact with the same number or address twice shows both", async ({ page }) => {
    const twice = contact("c1", "Sam Taylor", { emails: [{ address: "sam@example.test", name: "" }, { address: "sam@example.test", name: "" }], business_phones: ["07 5550 0001", "07 5550 0001"] });
    await withCore(page, { accounts: [robin], contacts: [twice, contact("c2", "Jo Nguyen", { business_phones: ["07 5550 0001"] })] });
    await page.getByRole("button", { name: "People" }).click();
    const card = page.getByRole("region", { name: "Contact" });

    await page.getByRole("list", { name: "Contacts" }).getByRole("button", { name: /Jo Nguyen/ }).click();
    await expect(card.getByText("07 5550 0001")).toHaveCount(1);
    await page.getByRole("list", { name: "Contacts" }).getByRole("button", { name: /Sam Taylor/ }).click();
    await expect(card.getByText("07 5550 0001")).toHaveCount(2);
    await expect(card.getByText("sam@example.test")).toHaveCount(2);
});

test("a choice that was refused goes back to what it was: sharing, a category's colour, a rule and this computer's", async ({ page }) => {
    const rules = [{ id: "rule-1", name: "Newsletters", enabled: true, sequence: 1, read_only: false, conditions: { senderContains: ["newsletter"] }, exceptions: {}, actions: { markAsRead: true } }];
    await withCore(page, { accounts: [robin], rules });
    await page.getByRole("button", { name: "Settings" }).click();
    const why = "Microsoft 365 could not be reached.";

    await page.getByRole("button", { name: "Calendar sharing" }).click();
    const sharing = page.getByRole("region", { name: "Calendar sharing for robin@example.test" });
    const seen = sharing.getByRole("combobox", { name: "What Everyone in your organisation can see" });
    await expect(seen).toHaveValue("freeBusyRead");
    await refuse(page, "calendar.sharing.set");
    await seen.selectOption("read");
    await expect(sharing.getByRole("alert")).toHaveText(why);
    await expect(seen).toHaveValue("freeBusyRead");
    await page.getByRole("button", { name: "Calendar sharing" }).click();

    await page.getByRole("button", { name: "Categories" }).click();
    const categories = page.getByRole("region", { name: "Categories for robin@example.test" });
    const colour = categories.getByRole("combobox", { name: "Colour of Clients" });
    await expect(colour).toHaveValue("preset7");
    await refuse(page, "mail.categories.set_color");
    await colour.selectOption({ label: "Green" });
    await expect(categories.getByRole("alert")).toHaveText(why);
    await expect(colour).toHaveValue("preset7");
    await page.getByRole("button", { name: "Categories" }).click();

    await page.getByRole("button", { name: "Rules" }).click();
    const listed = page.getByRole("region", { name: "Rules for robin@example.test" });
    const on = listed.getByRole("checkbox", { name: "Newsletters is on" });
    await expect(on).toBeChecked();
    await refuse(page, "mail.rules.update");
    await on.click();
    await expect(listed.getByRole("alert")).toHaveText(why);
    await expect(on).toBeChecked();

    await page.getByRole("navigation", { name: "Settings" }).getByRole("button", { name: "Start-up and updates" }).click();
    const here = page.getByRole("region", { name: "This computer" });
    await refuse(page, "app.desktop.set");
    await here.getByRole("checkbox", { name: /Keep running/ }).click();
    await expect(here.getByRole("alert")).toHaveText(why);
    await expect(here.getByRole("checkbox", { name: /Keep running/ })).not.toBeChecked();
    await here.getByRole("checkbox", { name: /Look for new versions/ }).click();
    await expect(here.getByRole("checkbox", { name: /Look for new versions/ })).toBeChecked();
});

test("a shared mailbox that could not be removed stays in the list, and it says why", async ({ page }) => {
    const shared = [{ id: "tenant-1.robin~support@example.test", account_id: "tenant-1.robin", address: "support@example.test", name: "Support" }];
    await withCore(page, { accounts: [robin], shared });
    await page.getByRole("button", { name: "Settings" }).click();
    await page.getByRole("button", { name: "Shared mailboxes" }).click();
    const section = page.getByRole("region", { name: "Shared mailboxes of robin@example.test" });

    await refuse(page, "accounts.shared.remove", "The mailbox could not be removed just now.");
    await section.getByRole("button", { name: "Remove support@example.test" }).click();
    await expect(section.getByRole("alert")).toHaveText("The mailbox could not be removed just now.");
    await expect(section.getByRole("listitem")).toContainText("support@example.test");

    await refuse(page, "accounts.shared.remove", null);
    await section.getByRole("button", { name: "Remove support@example.test" }).click();
    await expect(section.getByRole("listitem")).toHaveCount(0);
    await expect(section.getByRole("alert")).toHaveCount(0);
});

test("on a dark window the dot of a colour has a ring, so that a dark one is seen", async ({ page }) => {
    await page.emulateMedia({ colorScheme: "dark" });
    await withCore(page, { accounts: [robin] });
    await page.getByRole("button", { name: "Settings" }).click();
    await page.getByRole("button", { name: "Categories" }).click();
    const dot = page.getByRole("region", { name: "Categories for robin@example.test" }).locator(".dot").first();

    await expect(dot).not.toHaveCSS("box-shadow", "none");
    await page.emulateMedia({ colorScheme: "light" });
    await expect(dot).toHaveCSS("box-shadow", "none");
});

test("automatic replies scheduled by another zone's clock show when that is here, and are kept as they are", async ({ page }) => {
    // Set elsewhere, in London's time: an hour ahead of UTC until the last Sunday of October.
    const replies = {
        status: "scheduled",
        internal_message: "Away.",
        start: { date_time: "2026-10-20T08:00:00", time_zone: "Europe/London", at: "2026-10-20T07:00:00Z" },
        end: { date_time: "2026-10-26T17:30:00", time_zone: "Europe/London", at: "2026-10-26T17:30:00Z" },
    };
    await withCore(page, { accounts: [robin], replies });
    await page.getByRole("button", { name: "Settings" }).click();
    await page.getByRole("button", { name: "Automatic replies" }).click();
    const form = page.getByRole("form", { name: "Automatic replies for robin@example.test" });

    await expect(form.getByLabel("From", { exact: true })).toHaveValue("2026-10-20T17:00");
    await expect(form.getByLabel("Until", { exact: true })).toHaveValue("2026-10-27T03:30");
    // Something else is changed, and the times go back as the moments they were.
    await form.getByLabel("To people in your organisation").fill("Away until Monday.");
    await form.getByRole("button", { name: "Save" }).click();
    await expect.poll(async () => (await ran(page)).find((r) => r.command === "mail.automatic_replies.set")?.args).toMatchObject({ start: "2026-10-20T07:00:00Z", end: "2026-10-26T17:30:00Z" });
});

test("automatic replies set in a zone under Windows' name for it show their times, and one under no known name says so", async ({ page }) => {
    // Set from Outlook in Sydney's time, which Microsoft 365 names as Windows does. The core gives the moments.
    const replies = {
        status: "scheduled",
        internal_message: "Away.",
        start: { date_time: "2026-10-20T08:00:00", time_zone: "AUS Eastern Standard Time", at: "2026-10-19T21:00:00Z" },
        end: { date_time: "2026-10-24T17:30:00", time_zone: "AUS Eastern Standard Time", at: "2026-10-24T06:30:00Z" },
    };
    await withCore(page, { accounts: [robin], replies });
    await page.getByRole("button", { name: "Settings" }).click();
    await page.getByRole("button", { name: "Automatic replies" }).click();
    const form = page.getByRole("form", { name: "Automatic replies for robin@example.test" });

    // In Brisbane, an hour behind Sydney in October.
    await expect(form.getByLabel("From", { exact: true })).toHaveValue("2026-10-20T07:00");
    await expect(form.getByLabel("Until", { exact: true })).toHaveValue("2026-10-24T16:30");
    await expect(form.getByText("a time zone Brolga does not know")).toHaveCount(0);
    // Something else is changed, and the times go back as the moments they were, with nothing typed again.
    await form.getByLabel("To people in your organisation").fill("Away until Monday.");
    await form.getByRole("button", { name: "Save" }).click();
    await expect.poll(async () => (await ran(page)).find((r) => r.command === "mail.automatic_replies.set")?.args).toMatchObject({ start: "2026-10-19T21:00:00Z", end: "2026-10-24T06:30:00Z" });
});

test("automatic replies set in a zone under no name that is known leave the times to be set, and say why", async ({ page }) => {
    const replies = {
        status: "scheduled",
        internal_message: "Away.",
        start: { date_time: "2026-10-20T08:00:00", time_zone: "Middle of Nowhere Standard Time", at: null },
        end: { date_time: "2026-10-24T17:30:00", time_zone: "Middle of Nowhere Standard Time", at: null },
    };
    await withCore(page, { accounts: [robin], replies });
    await page.getByRole("button", { name: "Settings" }).click();
    await page.getByRole("button", { name: "Automatic replies" }).click();
    const form = page.getByRole("form", { name: "Automatic replies for robin@example.test" });

    await expect(form.getByLabel("From", { exact: true })).toHaveValue("");
    await expect(form.getByText("a time zone Brolga does not know (“Middle of Nowhere Standard Time”)")).toBeVisible();
});

test("settings are in sections, one shown at a time, and a mailbox's are shown for the account chosen", async ({ page }) => {
    await withCore(page, { accounts: [robin, { ...sam, needs_sign_in: false }], mail: inbox });
    await page.getByRole("button", { name: "Settings" }).click();
    const sections = page.getByRole("navigation", { name: "Settings" });
    const pane = page.getByRole("region", { name: "Accounts", exact: true });

    // Accounts to begin with: who is signed in, and adding another.
    await expect(sections.getByRole("button", { name: "Accounts", exact: true })).toHaveAttribute("aria-current", "page");
    await expect(pane.getByRole("list", { name: "Accounts" }).getByRole("listitem")).toHaveCount(2);
    await expect(pane.getByRole("button", { name: "Sign in with your browser" })).toBeVisible();
    // Nothing of a mailbox is asked which mailbox here.
    await expect(page.getByRole("combobox", { name: "Account" })).toHaveCount(0);

    // A mailbox's settings are one account's at a time, and the other's when it is chosen.
    await sections.getByRole("button", { name: "Rules" }).click();
    await expect(page.getByRole("heading", { level: 2, name: "Rules" })).toBeVisible();
    await expect(page.getByRole("region", { name: "Rules for robin@example.test" })).toBeVisible();
    await expect(page.getByRole("list", { name: "Accounts" })).toHaveCount(0);
    await page.getByRole("combobox", { name: "Account" }).selectOption("sam.taylor@northwind.example");
    await expect(page.getByRole("region", { name: "Rules for sam.taylor@northwind.example" })).toBeVisible();
    await expect(page.getByRole("region", { name: "Rules for robin@example.test" })).toHaveCount(0);
    // The account stays chosen from one section to the next.
    await sections.getByRole("button", { name: "Categories" }).click();
    await expect(page.getByRole("region", { name: "Categories for sam.taylor@northwind.example" })).toBeVisible();
    await sections.getByRole("button", { name: "Signature" }).click();
    await expect(page.getByRole("form", { name: "Signature for sam.taylor@northwind.example" })).toBeVisible();

    // The section is the one left, when the settings are come back to and when the window is opened again.
    await page.getByRole("button", { name: "Mail", exact: true }).click();
    await page.getByRole("button", { name: "Settings" }).click();
    await expect(sections.getByRole("button", { name: "Signature" })).toHaveAttribute("aria-current", "page");
    await page.reload();
    await page.getByRole("button", { name: "Settings" }).click();
    await expect(page.getByRole("navigation", { name: "Settings" }).getByRole("button", { name: "Signature" })).toHaveAttribute("aria-current", "page");
});

test("what is chosen in the settings for the week and for writing is what the calendar and a new message go by", async ({ page }) => {
    await withCore(page, { accounts: [robin], mail: inbox });
    await page.getByRole("button", { name: "Settings" }).click();
    const sections = page.getByRole("navigation", { name: "Settings" });

    await sections.getByRole("button", { name: "Week" }).click();
    await page.getByRole("region", { name: "Week" }).getByRole("combobox", { name: "Week starts on" }).selectOption("Sunday");
    await sections.getByRole("button", { name: "Writing" }).click();
    await page.getByRole("checkbox", { name: /Write new messages in plain text/ }).check();

    await page.getByRole("button", { name: "Calendar", exact: true }).click();
    await expect(page.getByRole("combobox", { name: "Week starts on" })).toHaveValue("0");
    await page.getByRole("button", { name: "Mail", exact: true }).click();
    await page.getByRole("button", { name: "New message" }).click();
    await expect(page.getByRole("form", { name: "New message" }).getByRole("checkbox", { name: "Plain text" })).toBeChecked();
});

test("with no account there is nothing of a mailbox to set, and what is this computer's can still be", async ({ page }) => {
    await withCore(page, {});
    const sections = page.getByRole("navigation", { name: "Settings" });

    await expect(sections.getByRole("button", { name: "Accounts", exact: true })).toHaveAttribute("aria-current", "page");
    for (const of of ["Shared mailboxes", "Signature", "Automatic replies", "Rules", "Categories", "Calendar sharing"]) await expect(sections.getByRole("button", { name: of })).toBeDisabled();
    await sections.getByRole("button", { name: "Start-up and updates" }).click();
    await expect(page.getByRole("region", { name: "This computer" })).toBeVisible();
});
