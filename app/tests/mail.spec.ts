import { type Page, expect, test } from "@playwright/test";
import { bulk, inbox, ran, robin, sam, withCore } from "./core";

const picture = async (page: Page, name: string) => {
    if (process.env.SCREENSHOTS) await page.screenshot({ path: `../docs/screenshots/${name}.png` });
};

const messages = (page: Page) => page.getByRole("listbox", { name: "Messages" });
const body = (page: Page) => page.frameLocator('iframe[title="Message body"]');

test("the window opens on the inbox, newest first, and syncs", async ({ page }) => {
    await withCore(page, { accounts: [robin], mail: inbox });

    const rows = messages(page).getByRole("option");
    await expect(rows).toHaveCount(4);
    await expect(rows.first()).toContainText("Sam Taylor");
    await expect(rows.first()).toContainText("Budget review moved to Thursday");
    await expect(page.getByRole("button", { name: "Inbox" })).toContainText("2");
    await expect(page.getByText("4 messages")).toBeVisible();
    await expect.poll(async () => (await ran(page)).some((r) => r.command === "mail.sync")).toBe(true);
});

test("opening a message shows who it is from and what it says", async ({ page }) => {
    await withCore(page, { accounts: [robin], mail: inbox });

    await messages(page).getByRole("option").first().click();

    const pane = page.getByRole("region", { name: "Message" });
    await expect(pane.getByRole("heading")).toHaveText("Budget review moved to Thursday");
    await expect(pane).toContainText("Sam Taylor");
    await expect(pane).toContainText("sam.taylor@example.test");
    await expect(body(page).getByText("2 pm on Thursday")).toBeVisible();
    await picture(page, "mail");
});

test("the message is shown where it cannot run scripts or reach the app", async ({ page }) => {
    await withCore(page, { accounts: [robin], mail: inbox });

    await messages(page).getByRole("option").first().click();

    const frame = page.locator('iframe[title="Message body"]');
    // No allow-scripts, no allow-forms, no allow-popups.
    await expect(frame).toHaveAttribute("sandbox", "allow-same-origin");
});

test("remote pictures stay out until asked for, and then come from the core", async ({ page }) => {
    await withCore(page, { accounts: [robin], mail: inbox });
    await messages(page).getByRole("option").nth(1).click();
    const pane = page.getByRole("region", { name: "Message" });

    await expect(pane.getByRole("status")).toContainText("3 remote pictures were not loaded");
    await expect(body(page).locator("img")).toHaveCount(0);
    await picture(page, "remote-pictures-blocked");

    await pane.getByRole("button", { name: "Show pictures" }).click();

    await expect(body(page).locator("img")).toHaveCount(1);
    // What is shown is the picture itself, not an address for the view to fetch.
    await expect(body(page).locator("img")).toHaveAttribute("src", /^data:image\/png/);
    const asked = (await ran(page)).filter((r) => r.command === "mail.messages.get").at(-1);
    expect(asked?.args.allow_remote_content).toBe(true);
});

test("a link opens in the browser, never in the app", async ({ page }) => {
    await withCore(page, { accounts: [robin], mail: inbox });
    await messages(page).getByRole("option").first().click();

    await body(page).getByRole("link", { name: "here" }).click();

    await expect.poll(async () => (await ran(page)).find((r) => r.command === "app.open_link")?.args).toEqual({
        url: "https://example.test/agenda",
    });
    // The app is still the app, and the message is still there to read.
    await expect(page.getByRole("navigation", { name: "Folders" })).toBeVisible();
    await expect(body(page).getByText("2 pm on Thursday")).toBeVisible();
    await expect(body(page).getByRole("link", { name: "here" })).toBeVisible();
    expect(page.url()).toBe("http://localhost:4173/");
});

test("a link that says one address and goes to another is called out", async ({ page }) => {
    await withCore(page, { accounts: [robin], mail: inbox });
    await messages(page).getByRole("option").nth(3).click();

    await body(page).getByRole("link").hover();

    const pane = page.getByRole("region", { name: "Message" });
    await expect(pane).toContainText("This link goes somewhere other than it says: https://account-security.example/login");
    await picture(page, "deceptive-link");
});

test("search narrows the list, and clearing it brings the folder back", async ({ page }) => {
    await withCore(page, { accounts: [robin], mail: inbox });
    const search = page.getByRole("searchbox", { name: "Search mail" });

    await search.fill("invoice");
    await search.press("Enter");

    await expect(messages(page).getByRole("option")).toHaveCount(1);
    await expect(page.getByText("1 result for “invoice”")).toBeVisible();

    await search.fill("");
    await expect(messages(page).getByRole("option")).toHaveCount(4);
});

test("a search with no matches says so", async ({ page }) => {
    await withCore(page, { accounts: [robin], mail: inbox });
    const search = page.getByRole("searchbox", { name: "Search mail" });

    await search.fill("no such thing");
    await search.press("Enter");

    await expect(page.getByText("Nothing matches.")).toBeVisible();
});

test("a folder of thousands keeps only what is on screen in the page", async ({ page }) => {
    await withCore(page, { accounts: [robin], mail: [...inbox, ...bulk(5000)] });

    await page.getByRole("button", { name: "Archive" }).click();
    await expect(page.getByText("5,000 messages")).toBeVisible();
    const rows = messages(page).getByRole("option");
    await expect(rows.first()).toContainText("Archived message 1");
    expect(await rows.count()).toBeLessThan(60);

    // Jump to the end: the last message is there, fetched when it was needed.
    await messages(page).evaluate((list) => (list.scrollTop = list.scrollHeight));

    await expect(messages(page).getByText("Archived message 5000", { exact: true })).toBeVisible();
    expect(await rows.count()).toBeLessThan(60);
    const pages = (await ran(page)).filter((r) => r.command === "mail.messages.list" && r.args.folder_id === "archive");
    expect(pages.length).toBeLessThan(6);
});

test("the list can be read with the arrow keys", async ({ page }) => {
    await withCore(page, { accounts: [robin], mail: inbox });
    await expect(messages(page).getByRole("option")).toHaveCount(4);

    await messages(page).focus();
    await page.keyboard.press("ArrowDown");
    await page.keyboard.press("ArrowDown");

    await expect(messages(page).getByRole("option", { selected: true })).toContainText("Your October newsletter");
    await expect(page.getByRole("region", { name: "Message" }).getByRole("heading")).toHaveText("Your October newsletter");
});

test("F9 syncs, and Ctrl+E goes to search", async ({ page }) => {
    await withCore(page, { accounts: [robin], mail: inbox });
    await expect(messages(page).getByRole("option")).toHaveCount(4);
    const syncs = async () => (await ran(page)).filter((r) => r.command === "mail.sync").length;
    const before = await syncs();

    await page.keyboard.press("F9");
    await expect.poll(syncs).toBe(before + 1);

    await page.keyboard.press("Control+e");
    await expect(page.getByRole("searchbox", { name: "Search mail" })).toBeFocused();
});

test("new mail appears at the top without losing the place in the list", async ({ page }) => {
    const fresh = { ...inbox[0], id: "fresh", subject: "Just arrived", preview: "Hot off the press.", received_at: "2026-10-09T09:00:00Z" };
    await withCore(page, { accounts: [robin], mail: inbox, arrives: [fresh] });
    await messages(page).getByRole("option").nth(2).click();
    await expect(page.getByRole("region", { name: "Message" }).getByRole("heading")).toHaveText("Invoice 2041 attached");

    await page.keyboard.press("F9");

    await expect(messages(page).getByRole("option")).toHaveCount(5);
    await expect(messages(page).getByRole("option").first()).toContainText("Just arrived");
    await expect(page.getByRole("button", { name: "Inbox" })).toContainText("3");
    // What was being read is still being read.
    await expect(messages(page).getByRole("option", { selected: true })).toContainText("Invoice 2041 attached");
    await expect(page.getByRole("region", { name: "Message" }).getByRole("heading")).toHaveText("Invoice 2041 attached");
});

test("a sync that fails says why and leaves the mail on screen", async ({ page }) => {
    const why = "robin@example.test needs to sign in again. Your sign-in has expired or was revoked.";
    await withCore(page, { accounts: [robin], mail: inbox, syncFails: why });

    await expect(page.getByRole("alert")).toHaveText(why);
    await expect(messages(page).getByRole("option")).toHaveCount(4);
});

test("with two accounts, each has its own folders", async ({ page }) => {
    await withCore(page, { accounts: [robin, { ...sam, needs_sign_in: false }], mail: inbox });

    await page.getByRole("combobox", { name: "Account" }).selectOption(sam.id);

    await expect.poll(async () => (await ran(page)).filter((r) => r.command === "mail.folders.list").at(-1)?.args).toEqual({
        account_id: sam.id,
    });
});
