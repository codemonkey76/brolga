import { type Page, expect, test } from "@playwright/test";
import { bulk, inbox, lunch, ran, robin, sam, sent, withCore } from "./core";

const picture = async (page: Page, name: string) => {
    if (process.env.SCREENSHOTS) await page.screenshot({ path: `../docs/screenshots/${name}.png` });
};

const messages = (page: Page) => page.getByRole("listbox", { name: "Messages" });
const tree = (page: Page) => page.getByRole("list", { name: "Folders" });
const folder = (page: Page, name: string) => tree(page).getByRole("button", { name: new RegExp(`^${name}`) });
const body = (page: Page) => page.frameLocator('iframe[title="Message body"]');

test("the window opens on the inbox, newest first, and syncs", async ({ page }) => {
    await withCore(page, { accounts: [robin], mail: inbox });

    const rows = messages(page).getByRole("option");
    await expect(rows).toHaveCount(4);
    await expect(rows.first()).toContainText("Sam Taylor");
    await expect(rows.first()).toContainText("Budget review moved to Thursday");
    await expect(folder(page, "Inbox")).toContainText("2");
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

    await folder(page, "Archive").click();
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
    await expect(folder(page, "Inbox")).toContainText("3");
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

test("folders inside folders can be closed up, and stay closed", async ({ page }) => {
    await withCore(page, { accounts: [robin], mail: inbox });

    await expect(folder(page, "Surveys")).toBeVisible();
    await page.getByRole("button", { name: "Collapse Projects" }).click();

    await expect(folder(page, "Harbour upgrade")).toHaveCount(0);
    await expect(folder(page, "Surveys")).toHaveCount(0);
    await expect(folder(page, "Projects")).toBeVisible();

    await page.reload();
    await expect(page.getByRole("button", { name: "Expand Projects" })).toBeVisible();
    await expect(folder(page, "Website")).toHaveCount(0);

    await page.getByRole("button", { name: "Expand Projects" }).click();
    await expect(folder(page, "Website")).toBeVisible();
});

test("a pinned folder is kept at the top, and can be unpinned", async ({ page }) => {
    await withCore(page, { accounts: [robin], mail: [...inbox, ...bulk(3)], pinned: ["archive"] });
    const favourites = page.getByRole("list", { name: "Favourites" });

    await expect(favourites.getByRole("button", { name: /^Archive/ })).toBeVisible();
    await picture(page, "folders");

    await favourites.getByRole("button", { name: /^Archive/ }).click();
    await expect(page.getByText("3 messages")).toBeVisible();

    await page.getByRole("button", { name: "Pin Surveys" }).click();
    await expect(favourites.getByRole("button", { name: /^Surveys/ })).toBeVisible();

    await favourites.getByRole("button", { name: "Unpin Archive" }).click();
    await expect(favourites.getByRole("listitem")).toHaveCount(1);
    expect((await ran(page)).filter((r) => r.command === "mail.folders.set_favourite")).toHaveLength(2);
});

test("as conversations, a thread is one line and opens with all its messages", async ({ page }) => {
    await withCore(page, { accounts: [robin], mail: [...inbox, ...lunch] });
    const rows = messages(page).getByRole("option");
    await expect(rows).toHaveCount(6);

    await page.getByRole("checkbox", { name: "Conversations" }).check();

    await expect(rows).toHaveCount(5);
    await expect(page.getByText("5 conversations")).toBeVisible();
    await expect(rows.first()).toContainText("Jo Nguyen");
    await expect(rows.first().getByLabel("2 messages")).toBeVisible();
    await expect(page.getByRole("combobox", { name: "Sort" })).toBeDisabled();

    await rows.first().click();
    const thread = page.getByRole("list", { name: "Conversation" }).getByRole("button");
    // The answer from Sent Items is part of what was said, though not of the inbox.
    await expect(thread).toHaveCount(3);
    await expect(thread.nth(1)).toContainText("Robin Example");
    await expect(thread.nth(2)).toHaveAttribute("aria-current", "true");
    await expect(body(page).getByText("Booked for four of us")).toBeVisible();
    await picture(page, "conversation");

    await thread.nth(1).click();
    await expect(body(page).getByText("Half past twelve?")).toBeVisible();
    await expect(thread.nth(1)).toHaveAttribute("aria-current", "true");
    await expect(rows.first()).toHaveAttribute("aria-selected", "true");

    await page.reload();
    await expect(page.getByRole("checkbox", { name: "Conversations" })).toBeChecked();
    await expect(rows).toHaveCount(5);
});

test("a message on its own has no conversation list", async ({ page }) => {
    await withCore(page, { accounts: [robin], mail: inbox });
    await page.getByRole("checkbox", { name: "Conversations" }).check();

    await messages(page).getByRole("option").first().click();

    await expect(body(page).getByText("2 pm on Thursday")).toBeVisible();
    await expect(page.getByRole("list", { name: "Conversation" })).toHaveCount(0);
});

test("the list can be sorted by sender, by subject and oldest first", async ({ page }) => {
    await withCore(page, { accounts: [robin], mail: inbox });
    const first = messages(page).getByRole("option").first();
    const sort = page.getByRole("combobox", { name: "Sort" });

    await sort.selectOption("sender");
    await expect(first).toContainText("Alex Chen");

    await sort.selectOption("subject");
    await expect(first).toContainText("Action needed");

    await sort.selectOption("oldest");
    await expect(first).toContainText("IT Support");

    await sort.selectOption("newest");
    await expect(first).toContainText("Sam Taylor");
});

test("attachments are listed on the message and saved when asked", async ({ page }) => {
    await withCore(page, { accounts: [robin], mail: inbox });

    await messages(page).getByRole("option").filter({ hasText: "Invoice 2041" }).click();
    const attached = page.getByRole("list", { name: "Attachments" });

    // The picture within the body is not offered as a file.
    await expect(attached.getByRole("listitem")).toHaveCount(2);
    await expect(attached.getByRole("button", { name: /^📎 Invoice 2041\.pdf/ })).toContainText("180 KB");
    // A link to a file kept elsewhere has nothing to save.
    await expect(attached.getByRole("button", { name: /Statement/ })).toHaveCount(0);
    await expect(attached).toContainText("Statement.xlsx");

    await attached.getByRole("button", { name: /^📎 Invoice 2041\.pdf/ }).click();
    await expect(page.getByText("Saved to /home/robin/Downloads/Invoice 2041.pdf")).toBeVisible();
    await picture(page, "attachments");

    await messages(page).getByRole("option").first().click();
    await expect(body(page).getByText("2 pm on Thursday")).toBeVisible();
    await expect(attached).toHaveCount(0);
    await expect(page.getByText("Saved to")).toHaveCount(0);
});

test("a sender can be trusted for pictures, and untrusted again", async ({ page }) => {
    await withCore(page, { accounts: [robin], mail: inbox });
    const pane = page.getByRole("region", { name: "Message" });

    await messages(page).getByRole("option").nth(1).click();
    await expect(body(page).locator("img")).toHaveCount(0);
    await pane.getByRole("button", { name: "Always show from northwind.news@example.test" }).click();

    await expect(body(page).locator("img")).toHaveCount(1);
    await expect(pane.getByText("are always shown")).toBeVisible();

    // It holds the next time the message is opened, with nothing to click.
    await messages(page).getByRole("option").first().click();
    await expect(body(page).getByText("2 pm on Thursday")).toBeVisible();
    await messages(page).getByRole("option").nth(1).click();
    await expect(body(page).locator("img")).toHaveCount(1);

    await pane.getByRole("button", { name: "Stop" }).click();
    await expect(body(page).locator("img")).toHaveCount(0);
    await expect(pane.getByRole("button", { name: "Show pictures" })).toBeVisible();
});

test("a message can be read as plain text, with no frame at all", async ({ page }) => {
    await withCore(page, { accounts: [robin], mail: inbox });
    await messages(page).getByRole("option").first().click();
    await expect(body(page).getByText("2 pm on Thursday")).toBeVisible();

    await page.getByRole("checkbox", { name: "Plain text" }).check();

    await expect(page.locator("iframe")).toHaveCount(0);
    await expect(page.getByLabel("Message body")).toContainText("the room is booked for 2 pm on Thursday");
    expect((await ran(page)).filter((r) => r.command === "mail.messages.get").at(-1)?.args.format).toBe("text");
});

test("the reading pane can go under the list", async ({ page }) => {
    await withCore(page, { accounts: [robin], mail: inbox });
    await messages(page).getByRole("option").first().click();
    const list = await messages(page).boundingBox();
    const beside = await page.getByRole("region", { name: "Message" }).boundingBox();
    expect(beside!.x).toBeGreaterThan(list!.x + list!.width - 1);

    await page.getByRole("combobox", { name: "Reading pane" }).selectOption("bottom");

    const under = await page.getByRole("region", { name: "Message" }).boundingBox();
    const listNow = await messages(page).boundingBox();
    expect(under!.y).toBeGreaterThan(listNow!.y + listNow!.height - 1);
    await expect(body(page).getByText("2 pm on Thursday")).toBeVisible();
    await picture(page, "reading-pane-bottom");
});

test("the window is dark when the desktop is, the message included", async ({ page }) => {
    await page.emulateMedia({ colorScheme: "dark" });
    await withCore(page, { accounts: [robin], mail: inbox });
    await messages(page).getByRole("option").nth(1).click();
    await expect(body(page).getByText("five ways to plan")).toBeVisible();

    // How light a colour is, from 0 to 255, whatever form the browser gives it in.
    const lightness = (colour: string) => {
        const [r, g, b] = (colour.match(/[\d.]+/g) ?? []).map(Number);
        return colour.startsWith("color(") ? ((r + g + b) / 3) * 255 : (r + g + b) / 3;
    };
    const ground = await page.evaluate(() => getComputedStyle(document.body).backgroundColor);
    const words = await page.evaluate(() => getComputedStyle(document.body).color);
    const inMessage = await body(page).locator("body").evaluate((el) => getComputedStyle(el).color);

    expect(lightness(ground)).toBeLessThan(80);
    expect(lightness(words)).toBeGreaterThan(170);
    expect(lightness(inMessage)).toBeGreaterThan(170);
    await picture(page, "dark");
});

test("a list by date is marked off into today, yesterday and earlier", async ({ page }) => {
    // A Friday afternoon in Brisbane.
    await page.clock.setFixedTime(new Date("2026-10-09T06:00:00Z"));
    await withCore(page, { accounts: [robin], mail: inbox });
    const rows = messages(page).getByRole("option");

    await expect(page.locator(".stretch")).toHaveText("Today");
    await expect(rows.nth(0).locator(".starts")).toHaveCount(0);
    await expect(rows.nth(1).locator(".starts")).toHaveCount(0);
    await expect(rows.nth(2).locator(".starts")).toHaveText("Yesterday");
    await expect(rows.nth(3).locator(".starts")).toHaveText("Earlier this week");
    await picture(page, "mail-by-date");

    // By sender there are no stretches of time to mark.
    await page.getByRole("combobox", { name: "Sort" }).selectOption("sender");
    await expect(page.locator(".stretch")).toHaveCount(0);
    await expect(page.locator(".starts")).toHaveCount(0);
});

test("an attachment can be saved to a place chosen in the save dialog", async ({ page }) => {
    await withCore(page, { accounts: [robin], mail: inbox, savePath: "/home/robin/Documents/September.pdf" });
    await messages(page).getByRole("option").filter({ hasText: "Invoice 2041" }).click();

    await page.getByRole("button", { name: "Save Invoice 2041.pdf as…" }).click();

    await expect(page.getByText("Saved to /home/robin/Documents/September.pdf")).toBeVisible();
    expect((await ran(page)).find((r) => r.command === "mail.attachments.save")?.args.path).toBe("/home/robin/Documents/September.pdf");
});

test("dismissing the save dialog saves nothing", async ({ page }) => {
    await withCore(page, { accounts: [robin], mail: inbox });
    await messages(page).getByRole("option").filter({ hasText: "Invoice 2041" }).click();

    await page.getByRole("button", { name: "Save Invoice 2041.pdf as…" }).click();

    await expect(page.getByText("Saved to")).toHaveCount(0);
    expect((await ran(page)).some((r) => r.command === "mail.attachments.save")).toBe(false);
});

test("opening a message marks it read, quietly", async ({ page }) => {
    await withCore(page, { accounts: [robin], mail: inbox });
    await expect(folder(page, "Inbox")).toContainText("2");

    await messages(page).getByRole("option").first().click();

    await expect(folder(page, "Inbox")).toContainText("1");
    await expect(messages(page).getByRole("option").first()).not.toHaveClass(/unread/);
    await expect(page.getByRole("button", { name: "Undo" })).toHaveCount(0);
    expect((await ran(page)).find((r) => r.command === "mail.messages.set_read")?.args).toMatchObject({ message_ids: ["m1"], is_read: true });
});

test("a message can be marked unread and flagged, and each can be undone", async ({ page }) => {
    await withCore(page, { accounts: [robin], mail: inbox });
    const first = messages(page).getByRole("option").first();
    const actions = page.getByRole("toolbar", { name: "Message actions" });
    await first.click();
    await expect(folder(page, "Inbox")).toContainText("1");

    await actions.getByRole("button", { name: "Mark as unread" }).click();
    await expect(first).toHaveClass(/unread/);
    await expect(folder(page, "Inbox")).toContainText("2");
    await expect(page.getByText("Marked as unread")).toBeVisible();

    await page.getByRole("button", { name: "Undo" }).click();
    await expect(first).not.toHaveClass(/unread/);
    await expect(actions.getByRole("button", { name: "Mark as unread" })).toBeVisible();
    await expect(page.getByRole("button", { name: "Undo" })).toHaveCount(0);

    await actions.getByRole("button", { name: "Flag", exact: true }).click();
    await expect(first.getByLabel("Flagged")).toBeVisible();
    await expect(actions).toContainText("Flagged");
    await picture(page, "flagged-with-undo");

    await page.keyboard.press("Control+z");
    await expect(first.getByLabel("Flagged")).toHaveCount(0);
    await expect(actions.getByRole("button", { name: "Flag", exact: true })).toBeVisible();
});

test("the offer to undo goes after ten seconds", async ({ page }) => {
    await page.clock.install();
    await withCore(page, { accounts: [robin], mail: inbox });
    await messages(page).getByRole("option").first().click();
    await page.getByRole("button", { name: "Mark as unread" }).click();
    await expect(page.getByRole("button", { name: "Undo" })).toBeVisible();

    await page.clock.fastForward(9_000);
    await expect(page.getByRole("button", { name: "Undo" })).toBeVisible();
    await page.clock.fastForward(2_000);
    await expect(page.getByRole("button", { name: "Undo" })).toHaveCount(0);
});

test("read, unread and flag have keys", async ({ page }) => {
    await withCore(page, { accounts: [robin], mail: inbox });
    const first = messages(page).getByRole("option").first();
    await first.click();
    await expect(first).not.toHaveClass(/unread/);

    await page.keyboard.press("Control+u");
    await expect(first).toHaveClass(/unread/);
    await page.keyboard.press("Control+q");
    await expect(first).not.toHaveClass(/unread/);
    await page.keyboard.press("Insert");
    await expect(first.getByLabel("Flagged")).toBeVisible();
    await page.keyboard.press("Insert");
    await expect(first.getByLabel("Flagged")).toHaveCount(0);
});

test("a change Microsoft 365 refuses is told of, once", async ({ page }) => {
    await withCore(page, {
        accounts: [robin],
        mail: inbox,
        refused: [{ message_id: "m3", subject: "Invoice 2041 attached", change: "flag", error: "This message cannot be changed." }],
    });

    const told = page.getByRole("alert");
    await expect(told).toContainText("would not change the flag of “Invoice 2041 attached”");
    await expect(told).toContainText("it has been put back");
    expect((await ran(page)).some((r) => r.command === "app.outbox.dismiss_refused")).toBe(true);
});

test("deleting moves the message away, opens the next, and can be undone", async ({ page }) => {
    await withCore(page, { accounts: [robin], mail: inbox });
    const rows = messages(page).getByRole("option");
    const pane = page.getByRole("region", { name: "Message" });
    await rows.first().click();
    await expect(pane.getByRole("heading")).toHaveText("Budget review moved to Thursday");

    await pane.getByRole("button", { name: "Delete", exact: true }).click();

    await expect(rows).toHaveCount(3);
    await expect(pane.getByRole("heading")).toHaveText("Your October newsletter");
    await expect(rows.first()).toHaveAttribute("aria-selected", "true");
    await expect(page.getByText("Moved to Deleted Items")).toBeVisible();
    expect((await ran(page)).find((r) => r.command === "mail.messages.move")?.args).toMatchObject({ message_ids: ["m1"], folder_id: "deleteditems" });

    await page.getByRole("button", { name: "Undo" }).click();
    await expect(rows).toHaveCount(4);
    await expect(rows.first()).toContainText("Budget review moved to Thursday");

    // In Deleted Items there is nothing further to delete to.
    await rows.first().click();
    await page.keyboard.press("Delete");
    await expect(rows).toHaveCount(3);
    await folder(page, "Deleted Items").click();
    await expect(rows).toHaveCount(1);
    await rows.first().click();
    await expect(pane.getByRole("heading")).toHaveText("Budget review moved to Thursday");
    await expect(pane.getByRole("button", { name: "Delete", exact: true })).toHaveCount(0);
});

test("a message can be archived by key and moved to any folder", async ({ page }) => {
    await withCore(page, { accounts: [robin], mail: inbox });
    const rows = messages(page).getByRole("option");
    await rows.first().click();
    await expect(body(page).getByText("2 pm on Thursday")).toBeVisible();

    await page.keyboard.press("Backspace");
    await expect(rows).toHaveCount(3);
    await expect(page.getByText("Archived")).toBeVisible();

    await page.getByRole("combobox", { name: "Move to folder" }).selectOption({ label: "\u00a0\u00a0\u00a0\u00a0Surveys" });
    await expect(rows).toHaveCount(2);
    await expect(page.getByText("Moved to Surveys")).toBeVisible();

    await folder(page, "Surveys").click();
    await expect(rows).toHaveCount(1);
    await expect(rows.first()).toContainText("Your October newsletter");
});

test("deleting the last message leaves nothing open", async ({ page }) => {
    await withCore(page, { accounts: [robin], mail: inbox.slice(0, 1) });
    await messages(page).getByRole("option").first().click();
    await expect(body(page).getByText("2 pm on Thursday")).toBeVisible();

    await page.keyboard.press("Delete");

    await expect(page.getByText("Nothing here.")).toBeVisible();
    await expect(page.getByText("Select a message to read it.")).toBeVisible();
});

test("a new message is written, kept as it goes, and sent", async ({ page }) => {
    await withCore(page, { accounts: [robin], mail: inbox });
    await page.getByRole("button", { name: "New message" }).click();
    const form = page.getByRole("form", { name: "New message" });

    await expect(form.getByRole("combobox", { name: "To" })).toBeFocused();
    await form.getByRole("combobox", { name: "To" }).fill("Jo Nguyen <jo.nguyen@example.test>, sam.taylor@example.test");
    await form.getByRole("textbox", { name: "Subject" }).fill("Lunch on Friday");
    await form.getByRole("textbox", { name: "Message" }).click();
    await page.keyboard.type("Are you ");
    await page.keyboard.press("Control+b");
    await page.keyboard.type("both");
    await page.keyboard.press("Control+b");
    await page.keyboard.type(" free?");
    await picture(page, "compose");

    // It is kept without being asked, and shows among what is being written.
    await expect(form.getByText("Draft kept on this computer")).toBeVisible();
    await expect(page.getByRole("list", { name: "Being written" })).toContainText("Lunch on Friday");

    await form.getByRole("button", { name: "Send" }).click();

    await expect(form).toHaveCount(0);
    await expect(page.getByText("Sending.")).toBeVisible();
    await expect(page.getByRole("list", { name: "Being written" })).toHaveCount(0);
    const went = await sent(page);
    expect(went).toHaveLength(1);
    expect(went[0]).toMatchObject({
        subject: "Lunch on Friday",
        format: "html",
        to: [
            { name: "Jo Nguyen", address: "jo.nguyen@example.test" },
            { name: null, address: "sam.taylor@example.test" },
        ],
    });
    expect(String(went[0].body)).toMatch(/Are you <(b|strong)>both<\/(b|strong)>(&nbsp;| )free\?/);
});

test("a reply is addressed and titled, and starts in the words", async ({ page }) => {
    await withCore(page, { accounts: [robin], mail: inbox });
    await messages(page).getByRole("option").first().click();
    await page.getByRole("button", { name: "Reply", exact: true }).click();
    const form = page.getByRole("form", { name: "Reply" });

    await expect(form.getByRole("combobox", { name: "To" })).toHaveValue("Sam Taylor <sam.taylor@example.test>");
    await expect(form.getByRole("textbox", { name: "Subject" })).toHaveValue("RE: Budget review moved to Thursday");
    await expect(form.getByRole("textbox", { name: "Message" })).toBeFocused();
    await expect(form.getByText("The original message is added underneath")).toBeVisible();

    await page.keyboard.type("Thursday is fine.");
    await page.keyboard.press("Control+Enter");

    await expect(form).toHaveCount(0);
    const reply = (await sent(page))[0];
    expect(reply).toMatchObject({ kind: "reply", message_id: "m1" });
    expect(String(reply.body)).toContain("Thursday is fine.");
    // The message that was being read is back.
    await expect(body(page).getByText("2 pm on Thursday")).toBeVisible();
});

test("a draft put aside is there to come back to, and can be thrown away", async ({ page }) => {
    await withCore(page, { accounts: [robin], mail: inbox });
    await page.keyboard.press("Control+n");
    const form = page.getByRole("form", { name: "New message" });
    await form.getByRole("textbox", { name: "Subject" }).fill("Half written");
    await form.getByRole("textbox", { name: "Message" }).click();
    await page.keyboard.type("To be finished");

    await page.keyboard.press("Escape");
    await expect(form).toHaveCount(0);

    await page.getByRole("list", { name: "Being written" }).getByRole("button", { name: "Half written" }).click();
    await expect(form.getByRole("textbox", { name: "Message" })).toHaveText("To be finished");

    // Throwing it away takes asking twice.
    await form.getByRole("button", { name: "Discard" }).click();
    await expect(form).toBeVisible();
    await form.getByRole("button", { name: "Discard for good?" }).click();
    await expect(form).toHaveCount(0);
    await expect(page.getByRole("list", { name: "Being written" })).toHaveCount(0);
    expect(await sent(page)).toHaveLength(0);
});

test("a slip in an address, or nobody to send to, is said and nothing is sent", async ({ page }) => {
    await withCore(page, { accounts: [robin], mail: inbox });
    await page.getByRole("button", { name: "New message" }).click();
    const form = page.getByRole("form", { name: "New message" });
    await form.getByRole("textbox", { name: "Subject" }).fill("Hello");

    await form.getByRole("button", { name: "Send" }).click();
    await expect(form.getByRole("alert")).toContainText("at least one recipient");

    await form.getByRole("combobox", { name: "To" }).fill("jo at example");
    await form.getByRole("button", { name: "Send" }).click();
    await expect(form.getByRole("alert")).toContainText("`jo at example` is not an email address");
    await expect(form.getByRole("combobox", { name: "To" })).toHaveValue("jo at example");
    expect(await sent(page)).toHaveLength(0);
});

test("a message Microsoft 365 will not send comes back with the reason", async ({ page }) => {
    await withCore(page, { accounts: [robin], mail: inbox, sendRefused: "The mailbox is over its sending limit." });
    await page.getByRole("button", { name: "New message" }).click();
    const form = page.getByRole("form", { name: "New message" });
    await form.getByRole("combobox", { name: "To" }).fill("jo@example.test");
    await form.getByRole("textbox", { name: "Subject" }).fill("Hello");
    await form.getByRole("textbox", { name: "Message" }).click();
    await page.keyboard.type("Nothing of this is lost.");
    await form.getByRole("button", { name: "Send" }).click();

    const waiting = page.getByRole("list", { name: "Being written" });
    await expect(waiting).toContainText("Not sent");
    await waiting.getByRole("button", { name: /Hello/ }).click();

    await expect(form.getByRole("alert")).toContainText("This was not sent. The mailbox is over its sending limit.");
    await expect(form.getByRole("textbox", { name: "Message" })).toHaveText("Nothing of this is lost.");
});

test("a signature is set on the accounts screen and starts new messages", async ({ page }) => {
    await withCore(page, { accounts: [robin], mail: inbox });
    await page.getByRole("button", { name: "Accounts" }).click();
    await page.getByRole("button", { name: "Signature" }).click();
    const editor = page.getByRole("form", { name: "Signature for robin@example.test" });

    await editor.getByRole("textbox", { name: "Signature" }).fill("Robin Example\nExample Pty Ltd");
    await editor.getByRole("button", { name: "Save" }).click();
    await expect(page.getByText("Signature saved.")).toBeVisible();

    await page.getByRole("button", { name: "Mail" }).click();
    await page.getByRole("button", { name: "New message" }).click();
    const form = page.getByRole("form", { name: "New message" });
    const words = form.getByRole("textbox", { name: "Message" });
    await expect(words).toContainText("Robin Example");
    await expect(words).toContainText("Example Pty Ltd");

    // Typing goes above it.
    await words.focus();
    await page.keyboard.type("Hello");
    await expect.poll(async () => (await words.innerText()).replace(/\s+/g, " ").trim()).toBe("Hello Robin Example Example Pty Ltd");

    // As plain text it is the same words, and stays plain for the next message.
    await form.getByRole("checkbox", { name: "Plain text" }).check();
    await expect(words).toHaveValue(/^Hello\n+Robin Example\nExample Pty Ltd$/);
    await expect(form.getByRole("toolbar", { name: "Formatting" }).getByRole("button")).toHaveCount(0);

    // An answer has no signature unless asked for.
    await page.keyboard.press("Escape");
    await messages(page).getByRole("option").first().click();
    await page.getByRole("button", { name: "Reply", exact: true }).click();
    await expect(page.getByRole("form", { name: "Reply" }).getByRole("textbox", { name: "Message" })).toHaveValue("");
});

test("mail shows as it arrives, while a long first sync is still going", async ({ page }) => {
    await page.clock.install();
    await withCore(page, { accounts: [robin], mail: [], syncNeverEnds: true });
    await expect(page.getByText("Fetching your mail…")).toBeVisible();

    // Mail lands on this computer part-way through the sync.
    await page.evaluate(() => (window as unknown as { arrive: () => void }).arrive());
    await page.clock.fastForward(2_500);

    await expect(messages(page).getByRole("option")).toHaveCount(1);
    await expect(page.getByRole("button", { name: "Syncing…" })).toBeDisabled();
});

test("search filters as the words are typed", async ({ page }) => {
    await withCore(page, { accounts: [robin], mail: inbox });
    const rows = messages(page).getByRole("option");
    await expect(rows).toHaveCount(4);

    await page.getByRole("searchbox", { name: "Search mail" }).pressSequentially("invo");
    await expect(rows).toHaveCount(1);
    await expect(rows.first()).toContainText("Invoice 2041");

    await page.getByRole("searchbox", { name: "Search mail" }).fill("");
    await expect(rows).toHaveCount(4);
    await expect(page.getByText("4 messages")).toBeVisible();
});

test("files are attached from the file dialog, can be removed, and go with the message", async ({ page }) => {
    await withCore(page, {
        accounts: [robin],
        mail: inbox,
        pickFiles: ["/home/robin/Documents/Quarterly report.pdf", "/home/robin/huge-video.mov", "/home/robin/Documents/Budget.xlsx"],
    });
    await page.getByRole("button", { name: "New message" }).click();
    const form = page.getByRole("form", { name: "New message" });
    await form.getByRole("combobox", { name: "To" }).fill("jo@example.test");

    await form.getByRole("button", { name: "Attach…" }).click();

    const attached = form.getByRole("list", { name: "Attached files" });
    // The one that is too big is said, and does not stop the others.
    await expect(attached.getByRole("listitem")).toHaveCount(2);
    await expect(attached).toContainText("Quarterly report.pdf");
    await expect(form.getByRole("alert")).toContainText("`huge-video.mov` is over 25 MB");
    await picture(page, "compose-with-files");

    await attached.getByRole("button", { name: "Remove Budget.xlsx" }).click();
    await expect(attached.getByRole("listitem")).toHaveCount(1);

    await form.getByRole("button", { name: "Send" }).click();
    await expect(form).toHaveCount(0);
    const went = (await sent(page))[0] as { files: { name: string }[] };
    expect(went.files.map((each) => each.name)).toEqual(["Quarterly report.pdf"]);
});

test("a message can be formatted: lists, a link, and plain pasting", async ({ page, context, browserName }) => {
    // Only Chromium lets a test put something on the clipboard to paste.
    const canPaste = browserName === "chromium";
    if (canPaste) await context.grantPermissions(["clipboard-read", "clipboard-write"]);
    await withCore(page, { accounts: [robin], mail: inbox });
    await page.getByRole("button", { name: "New message" }).click();
    const form = page.getByRole("form", { name: "New message" });
    const words = form.getByRole("textbox", { name: "Message" });
    const tools = form.getByRole("toolbar", { name: "Formatting" });
    await form.getByRole("combobox", { name: "To" }).fill("jo@example.test");

    await words.click();
    await tools.getByRole("button", { name: "Bulleted list" }).click();
    await page.keyboard.type("Milk");
    await page.keyboard.press("Enter");
    await page.keyboard.type("Bread");
    await expect(words.locator("ul > li")).toHaveCount(2);

    // A link is added without leaving the message, and Enter there does not send it.
    await page.keyboard.press("Enter");
    await page.keyboard.press("Enter");
    await tools.getByRole("button", { name: "Link" }).click();
    await form.getByRole("textbox", { name: "Link address" }).fill("https://example.test/list");
    await page.keyboard.press("Enter");
    await expect(words.locator('a[href="https://example.test/list"]')).toHaveCount(1);
    await expect(form).toBeVisible();

    if (canPaste) {
        // Formatting pasted from elsewhere comes in as plain words.
        await page.evaluate(() =>
            navigator.clipboard.write([
                new ClipboardItem({
                    "text/html": new Blob(['<h1 style="color:red">Loud</h1><img src="https://tracker.test/p.gif">'], { type: "text/html" }),
                    "text/plain": new Blob(["Loud"], { type: "text/plain" }),
                }),
            ]),
        );
        await words.focus();
        await page.keyboard.press("Control+v");
        await expect(words).toContainText("Loud");
        await expect(words.locator("h1, img")).toHaveCount(0);
    }

    await form.getByRole("button", { name: "Send" }).click();
    await expect(form).toHaveCount(0);
    const body = String((await sent(page))[0].body);
    expect(body).toContain("<li>Milk</li>");
    expect(body).toContain('href="https://example.test/list"');
    expect(body).not.toContain("tracker");
});

test("people are suggested as an address is typed, and chosen by key or click", async ({ page }) => {
    await withCore(page, { accounts: [robin], mail: inbox });
    await page.getByRole("button", { name: "New message" }).click();
    const form = page.getByRole("form", { name: "New message" });
    const to = form.getByRole("combobox", { name: "To" });
    const offered = form.getByRole("listbox", { name: "Suggested people" });

    await to.pressSequentially("sa");
    await expect(offered.getByRole("option")).toHaveCount(1);
    await expect(offered).toContainText("sam.taylor@example.test");

    // Enter takes the suggestion. It does not send the message.
    await page.keyboard.press("Enter");
    await expect(to).toHaveValue("Sam Taylor <sam.taylor@example.test>, ");
    await expect(offered).toHaveCount(0);
    await expect(form).toBeVisible();

    // A second person goes after the first.
    await to.pressSequentially("alex");
    await offered.getByRole("option").first().click();
    await expect(to).toHaveValue("Sam Taylor <sam.taylor@example.test>, Alex Chen <alex.chen@example.test>, ");
    await picture(page, "compose-suggestions");

    await form.getByRole("textbox", { name: "Subject" }).fill("Hello");
    await form.getByRole("button", { name: "Send" }).click();
    await expect(form).toHaveCount(0);
    expect(((await sent(page))[0].to as { address: string }[]).map((p) => p.address)).toEqual(["sam.taylor@example.test", "alex.chen@example.test"]);
});

test("in Deleted Items a message is deleted for good, after asking twice", async ({ page }) => {
    await withCore(page, { accounts: [robin], mail: inbox.map((m) => ({ ...m, folder_id: "deleted" })) });
    await folder(page, "Deleted Items").click();
    const rows = messages(page).getByRole("option");
    const pane = page.getByRole("region", { name: "Message" });
    await rows.first().click();
    await expect(pane.getByRole("heading")).toHaveText("Budget review moved to Thursday");

    await pane.getByRole("button", { name: "Delete permanently" }).click();
    await expect(rows).toHaveCount(4);
    await pane.getByRole("button", { name: "Keep" }).click();
    await expect(pane.getByRole("button", { name: "Delete permanently" })).toBeVisible();

    await pane.getByRole("button", { name: "Delete permanently" }).click();
    await pane.getByRole("button", { name: /Delete for good/ }).click();

    await expect(rows).toHaveCount(3);
    await expect(page.getByText("Deleted permanently.")).toBeVisible();
    // There is nothing to undo.
    await expect(page.getByRole("button", { name: "Undo" })).toHaveCount(0);
    // The next message is not left half-way to being deleted.
    await expect(pane.getByRole("button", { name: "Delete permanently" })).toBeVisible();
});

test("a message can be sent to junk and brought back", async ({ page }) => {
    await withCore(page, { accounts: [robin], mail: inbox });
    const rows = messages(page).getByRole("option");
    const pane = page.getByRole("region", { name: "Message" });
    await rows.nth(3).click();
    await expect(pane.getByRole("heading")).toHaveText("Action needed: confirm your password");

    await pane.getByRole("button", { name: "Junk", exact: true }).click();
    await expect(rows).toHaveCount(3);
    await expect(page.getByText("Moved to Junk Email")).toBeVisible();

    await folder(page, "Junk Email").click();
    await rows.first().click();
    await pane.getByRole("button", { name: "Not junk" }).click();
    await expect(page.getByText("Moved to the inbox")).toBeVisible();
    await folder(page, "Inbox").click();
    await expect(rows).toHaveCount(4);
});

test("a message is given categories, shown in their colours, and that can be undone", async ({ page }) => {
    await withCore(page, { accounts: [robin], mail: inbox });
    const first = messages(page).getByRole("option").first();
    const pane = page.getByRole("region", { name: "Message" });
    await first.click();

    await pane.getByText("Categories", { exact: true }).click();
    const menu = pane.getByRole("list", { name: "Categories" });
    await menu.getByRole("checkbox", { name: "Clients" }).check();
    await menu.getByRole("checkbox", { name: "Red category" }).check();

    await expect(pane.getByLabel("This message's categories")).toContainText("Clients");
    await expect(pane.getByLabel("This message's categories")).toContainText("Red category");
    await expect(first.getByLabel("Category Clients")).toBeVisible();
    await expect(first.getByLabel("Category Clients")).toHaveCSS("background-color", "rgb(47, 111, 206)");
    await picture(page, "categories");

    await page.getByRole("button", { name: "Undo" }).click();
    await expect(first.getByLabel("Category Red category")).toHaveCount(0);
    await expect(first.getByLabel("Category Clients")).toBeVisible();

    await menu.getByRole("checkbox", { name: "Clients" }).uncheck();
    await expect(first.getByLabel("Category Clients")).toHaveCount(0);
    await expect(pane.getByLabel("This message's categories")).toHaveCount(0);
});

test("a draft in the Drafts folder opens to be carried on with, and is saved to the mailbox when put aside", async ({ page }) => {
    await withCore(page, {
        accounts: [robin],
        mail: [
            ...inbox,
            {
                id: "d1",
                folder_id: "drafts",
                is_draft: true,
                subject: "Started in Outlook",
                preview: "Dear Jo,",
                from: { name: "Robin Example", address: "robin@example.test" },
                received_at: "2026-10-09T02:00:00Z",
                is_read: true,
                has_attachments: false,
                body: "<p>Dear <b>Jo</b>,</p>",
            },
        ],
    });
    await folder(page, "Drafts").click();
    await messages(page).getByRole("option").first().click();

    const form = page.getByRole("form", { name: "New message" });
    await expect(form.getByRole("textbox", { name: "Subject" })).toHaveValue("Started in Outlook");
    await expect(form.getByRole("combobox", { name: "To" })).toHaveValue("Jo Nguyen <jo.nguyen@example.test>");
    await expect(form.getByRole("textbox", { name: "Message" }).locator("b")).toHaveText("Jo");
    // It is not shown as a message to read, with Reply and the rest.
    await expect(page.getByRole("button", { name: "Reply", exact: true })).toHaveCount(0);

    await form.getByRole("textbox", { name: "Message" }).click();
    await page.keyboard.press("End");
    await page.keyboard.type(" see you Friday.");
    await page.keyboard.press("Escape");

    await expect(form).toHaveCount(0);
    await expect.poll(async () => (await ran(page)).some((r) => r.command === "app.outbox.flush")).toBe(true);
    // Opened again from the folder, it is the same draft with what was added.
    await messages(page).getByRole("option").first().click();
    await expect(form.getByRole("textbox", { name: "Message" })).toContainText("see you Friday.");
    await expect(page.getByRole("list", { name: "Being written" }).getByRole("button")).toHaveCount(1);
});

test("a flag can be given a day to follow up by, changed and taken off", async ({ page }) => {
    // A Friday afternoon in Brisbane.
    await page.clock.setFixedTime(new Date("2026-10-09T06:00:00Z"));
    await withCore(page, { accounts: [robin], mail: inbox });
    const pane = page.getByRole("region", { name: "Message" });
    const actions = page.getByRole("toolbar", { name: "Message actions" });
    await messages(page).getByRole("option").first().click();

    await actions.getByText("Flag for…").click();
    const days = actions.getByRole("list", { name: "Follow up by" });
    await expect(days.getByRole("button", { name: /This week/ })).toContainText(/Fri,? (9 Oct|Oct 9)/);
    await expect(days.getByRole("button", { name: /Next week/ })).toContainText(/Mon,? (12 Oct|Oct 12)/);
    await days.getByRole("button", { name: /Tomorrow/ }).click();

    await expect(actions).toContainText(/Due Sat,? (10 Oct|Oct 10)/);
    await expect(page.getByText(/Flagged for Sat,? (10 Oct|Oct 10)/)).toBeVisible();
    expect((await ran(page)).find((r) => r.command === "mail.messages.set_flag")?.args).toMatchObject({ flag: "flagged", due: "2026-10-10" });
    await picture(page, "flag-due");

    // Another day, typed.
    await actions.getByText("Due…").click();
    await days.getByLabel("Another day").fill("2026-11-02");
    await expect(actions).toContainText(/Due Mon,? (2 Nov|Nov 2)/);

    await actions.getByText("Due…").click();
    await days.getByRole("button", { name: "No day" }).click();
    await expect(actions).toContainText("Flagged");
    await expect(actions).not.toContainText("Due Mon");

    // Undo brings the day back with the flag.
    await page.getByRole("button", { name: "Undo" }).click();
    await expect(actions).toContainText(/Due Mon,? (2 Nov|Nov 2)/);
    await expect(pane).toBeVisible();
});

test("a message is forwarded as an attachment: a new message with it attached whole", async ({ page }) => {
    await withCore(page, { accounts: [robin], mail: inbox });
    await messages(page).getByRole("option").first().click();

    await page.getByRole("button", { name: "Forward as attachment" }).click();

    const form = page.getByRole("form", { name: "New message" });
    await expect(form.getByRole("textbox", { name: "Subject" })).toHaveValue("FW: Budget review moved to Thursday");
    await expect(form.getByRole("list", { name: "Attached files" })).toContainText("Budget review moved to Thursday.eml");
    // The original is attached, not quoted underneath.
    await expect(form.getByText("The original message is added underneath")).toHaveCount(0);
});
