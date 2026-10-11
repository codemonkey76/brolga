import { expect, test } from "@playwright/test";
import { ran, robin, sam, withCore } from "./core";

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
    await page.getByRole("button", { name: "Accounts" }).click();
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

    await page.getByRole("button", { name: "Accounts" }).click();
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

test("an account that needs it can sign in again, as itself", async ({ page }) => {
    await withCore(page, { accounts: [robin, sam], signIn: { ...sam, needs_sign_in: false } });
    await page.getByRole("button", { name: "Accounts" }).click();
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
    await page.getByRole("button", { name: "Accounts" }).click();
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
    for (let presses = 0; presses < 6 && !(await signIn.evaluate((b) => b === document.activeElement)); presses++) {
        await page.keyboard.press("Tab");
    }
    await expect(signIn).toBeFocused();
    await page.keyboard.press("Enter");

    await expect(page.getByRole("navigation", { name: "Folders" })).toBeVisible();
});

test("automatic replies are scheduled from the accounts screen, in local time", async ({ page }) => {
    await withCore(page, { accounts: [robin] });
    await page.getByRole("button", { name: "Accounts" }).click();
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
    await page.getByRole("button", { name: "Accounts" }).click();
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
