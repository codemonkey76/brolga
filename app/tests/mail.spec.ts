import { type Page, expect, test } from "@playwright/test";
import { bulk, inbox, lunch, ran, robin, sam, withCore } from "./core";

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
