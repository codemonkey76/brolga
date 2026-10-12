import { type Page, expect, test } from "@playwright/test";
import { bulk, inbox, lunch, notified, ran, robin, sam, sent, withCore } from "./core";

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

    await expect(pane.getByRole("status")).toContainText("To protect your privacy, 3 pictures in this message were not downloaded.");
    await expect(body(page).locator("img")).toHaveCount(0);
    await picture(page, "remote-pictures-blocked");

    await pane.getByRole("button", { name: "Show blocked content" }).click();

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
    await expect(pane).toContainText("This link goes somewhere other than it says: account-security.example · https://account-security.example/login");
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

test("the desktop is told of new mail that matters, once, and not of what was already there", async ({ page }) => {
    await page.clock.install({ time: new Date("2026-10-09T09:01:00Z") });
    const fresh = { ...inbox[0], id: "fresh", subject: "Just arrived", is_read: false, received_at: "2026-10-09T09:00:00Z" };
    const bulletin = { ...fresh, id: "bulletin", subject: "Weekly bulletin", focused: false, received_at: "2026-10-09T09:00:30Z" };
    await withCore(page, { accounts: [robin], mail: inbox, arrives: [fresh, bulletin] });
    await expect(messages(page).getByRole("option")).toHaveCount(4);
    await page.clock.fastForward(31_000);
    expect(await notified(page)).toEqual([]);

    await page.keyboard.press("F9");
    await expect(messages(page).getByRole("option")).toHaveCount(6);
    await page.clock.fastForward(31_000);

    await expect.poll(() => notified(page)).toEqual([{ title: "Sam Taylor", body: "Just arrived" }]);
    await page.clock.fastForward(61_000);
    expect(await notified(page)).toHaveLength(1);
});

test("a mailto link clicked elsewhere starts a message with what it names, and sends nothing", async ({ page }) => {
    const asked = { to: [{ address: "sam.taylor@example.test", name: null }], cc: [], bcc: [], subject: "Thursday", body: "See you <then>.\nRobin" };
    await withCore(page, { accounts: [robin], mail: inbox, asked });

    const form = page.getByRole("form", { name: "New message" });
    await expect(form.getByRole("textbox", { name: "Subject" })).toHaveValue("Thursday");
    await expect(form.getByRole("combobox", { name: "To" })).toHaveValue("sam.taylor@example.test");
    await expect(form).toContainText("See you <then>.");
    expect(await sent(page)).toEqual([]);
});

test("the palette lists what the window does with its keys, and does what is chosen", async ({ page }) => {
    await withCore(page, { accounts: [robin], mail: inbox });
    await messages(page).getByRole("option").first().click();

    await page.keyboard.press("Control+k");
    const palette = page.getByRole("dialog", { name: "Commands" });
    await expect(palette.getByRole("option").first()).toContainText("Go to Mail");
    await palette.getByRole("combobox").fill("all reply");
    await expect(palette.getByRole("option")).toHaveCount(1);
    await expect(palette.getByRole("option")).toContainText("Ctrl+Shift+R");
    await picture(page, "palette");
    await page.keyboard.press("Enter");

    await expect(palette).toHaveCount(0);
    await expect(page.getByRole("form", { name: "Reply to all" })).toBeVisible();
});

test("the keys step through messages, and go to a new meeting and the address book", async ({ page }) => {
    await withCore(page, { accounts: [robin], mail: inbox });
    await messages(page).getByRole("option").first().click();
    const heading = page.getByRole("region", { name: "Message" }).getByRole("heading");
    await expect(heading).toHaveText("Budget review moved to Thursday");

    await page.keyboard.press("Control+.");
    await expect(heading).not.toHaveText("Budget review moved to Thursday");
    await page.keyboard.press("Control+,");
    await expect(heading).toHaveText("Budget review moved to Thursday");

    await page.keyboard.press("Control+Shift+Q");
    await expect(page.getByRole("form", { name: "New event" })).toBeVisible();
    await page.keyboard.press("Control+Shift+B");
    await expect(page.getByRole("button", { name: "People" })).toHaveAttribute("aria-current", "true");
});

test("a link that says one site and goes to another is asked about before it opens", async ({ page }) => {
    const bait = { ...inbox[0], id: "bait", body: '<p><a href="https://login.evil.example/x">mybank.example/login</a> and <a href="https://docs.mybank.example/help">mybank.example</a></p>' };
    await withCore(page, { accounts: [robin], mail: [bait] });
    await messages(page).getByRole("option").first().click();
    const pane = page.getByRole("region", { name: "Message" });
    const opened = async () => (await ran(page)).filter((r) => r.command === "app.open_link").map((r) => r.args.url);

    await body(page).getByRole("link", { name: "mybank.example/login" }).click();
    await expect(pane.getByRole("alert")).toContainText("This link says mybank.example but goes to login.evil.example.");
    expect(await opened()).toEqual([]);
    await pane.getByRole("button", { name: "Don't open" }).click();
    await expect(pane.getByRole("alert")).toHaveCount(0);
    expect(await opened()).toEqual([]);

    await body(page).getByRole("link", { name: "mybank.example/login" }).click();
    await pane.getByRole("button", { name: "Open it anyway" }).click();
    await expect.poll(opened).toEqual(["https://login.evil.example/x"]);

    // A part of the site it names is the site it names.
    await body(page).getByRole("link", { name: "mybank.example", exact: true }).click();
    await expect.poll(opened).toEqual(["https://login.evil.example/x", "https://docs.mybank.example/help"]);
});

test("a file that can run is asked about before it is saved, and its ending is always in sight", async ({ page }) => {
    const name = `invoice.pdf${" ".repeat(60)}.exe`;
    const trap = { ...inbox[0], id: "trap", has_attachments: true, attachments: [{ id: "a1", name, content_type: "application/pdf", size: 2048, is_inline: false, kind: "file" }] };
    await withCore(page, { accounts: [robin], mail: [trap] });
    await messages(page).getByRole("option").first().click();
    const pane = page.getByRole("region", { name: "Message" });
    const saves = async () => (await ran(page)).filter((r) => r.command === "mail.attachments.save").length;

    const chip = pane.getByRole("list", { name: "Attachments" }).getByRole("button").first();
    await expect(chip.locator(".end")).toBeInViewport();
    await expect(chip.locator(".end")).toContainText(".exe");
    await chip.click();
    await expect(pane.getByRole("alert")).toContainText("is a program, or a file that can run one");
    expect(await saves()).toBe(0);
    await picture(page, "risky-attachment");

    await pane.getByRole("button", { name: "Save anyway" }).click();
    await expect.poll(saves).toBe(1);
});

test("a message is shown as it was laid out: on a white page, or darkened to suit a dark desktop", async ({ page }) => {
    const report = {
        ...inbox[0],
        id: "report",
        subject: "Customer Balances",
        // A pale banner with white words on it, as one real report has.
        styles: "body{background:#f3f4f6;color:#374151}\n.wrapper{max-width:900px;margin:32px auto;background:#fff}\n.header{background:#e9d5ff;padding:24px 40px}\n.header h1{color:#fff}\nthead th.r{text-align:right}\ntbody td{color:#374151;border-bottom:1px solid #f3f4f6}\ntbody td.r{text-align:right}\n",
        body: '<div class="wrapper"><div class="header"><h1>Customer Balances</h1></div><table><thead><tr><th>Customer</th><th class="r">Total</th></tr></thead><tbody><tr><td>CarCoop Pty Ltd</td><td class="r" style="font-weight:700;color:#111827">$6,225.08</td></tr></tbody></table></div>',
    };
    await withCore(page, { accounts: [robin], mail: [report, inbox[2]] });
    const pane = page.getByRole("region", { name: "Message" });
    const look = (selector: string, what: string) => body(page).locator(selector).first().evaluate((node, what) => getComputedStyle(node).getPropertyValue(what), what);
    /** How light a colour is, from 0 to 255. */
    const light = async (selector: string, what: string) => {
        const [r, g, b] = (await look(selector, what)).match(/[\d.]+/g)!.map(Number);
        return (Math.max(r, g, b) + Math.min(r, g, b)) / 2;
    };

    await messages(page).getByRole("option").first().click();
    await expect(body(page).getByRole("heading", { name: "Customer Balances" })).toBeVisible();
    expect(await look(".header", "background-color")).toBe("rgb(233, 213, 255)");
    expect(await look("td.r", "text-align")).toBe("right");
    expect(await look("td.r", "font-weight")).toBe("700");
    expect(await look("body", "background-color")).toBe("rgb(243, 244, 246)");
    await expect(pane.getByRole("button", { name: "Show it as written" })).toHaveCount(0);

    // Dark: grounds go dark and dark words go light; white words stay white, and the layout is the same.
    await page.emulateMedia({ colorScheme: "dark" });
    await expect.poll(() => light("body", "background-color")).toBeLessThan(70);
    expect(await light(".wrapper", "background-color")).toBeLessThan(70);
    expect(await light(".header", "background-color")).toBeLessThan(90);
    expect(await look(".header h1", "color")).toBe("rgb(255, 255, 255)");
    expect(await light("td", "color")).toBeGreaterThan(170);
    expect(await light("td.r", "color")).toBeGreaterThan(170);
    expect(await look("td.r", "text-align")).toBe("right");
    await picture(page, "styled-message");

    await pane.getByRole("button", { name: "Show it as written" }).click();
    await expect.poll(() => look(".header", "background-color")).toBe("rgb(233, 213, 255)");
    await pane.getByRole("button", { name: "Show it dark" }).click();
    await expect.poll(() => light(".header", "background-color")).toBeLessThan(90);

    // One that says nothing of how it looks is simply dark, with nothing to switch.
    await messages(page).getByRole("option").nth(1).click();
    await expect(body(page).locator("body")).not.toHaveCSS("background-color", "rgb(255, 255, 255)");
    await expect(pane.getByRole("button", { name: /Show it/ })).toHaveCount(0);
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
    await expect(attached.getByRole("button", { name: /^Invoice 2041\.pdf/ })).toContainText("180 KB");
    // A link to a file kept elsewhere has nothing to save.
    await expect(attached.getByRole("button", { name: /Statement/ })).toHaveCount(0);
    await expect(attached).toContainText("Statement.xlsx");

    await attached.getByRole("button", { name: /^Invoice 2041\.pdf/ }).click();
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
    await pane.getByRole("button", { name: "Trust sender" }).click();

    await expect(body(page).locator("img")).toHaveCount(1);
    await expect(pane.getByText("is a trusted sender")).toBeVisible();

    // It holds the next time the message is opened, with nothing to click.
    await messages(page).getByRole("option").first().click();
    await expect(body(page).getByText("2 pm on Thursday")).toBeVisible();
    await messages(page).getByRole("option").nth(1).click();
    await expect(body(page).locator("img")).toHaveCount(1);

    await pane.getByRole("button", { name: "Stop trusting" }).click();
    await expect(body(page).locator("img")).toHaveCount(0);
    await expect(pane.getByRole("button", { name: "Show blocked content" })).toBeVisible();
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

    await page.getByRole("button", { name: "Delete", exact: true }).click();

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
    await expect(page.getByRole("button", { name: "Delete", exact: true })).toHaveCount(0);
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

    await page.getByRole("button", { name: "Mail", exact: true }).click();
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

    await page.getByRole("button", { name: "Delete permanently" }).click();
    await expect(rows).toHaveCount(4);
    await page.getByRole("button", { name: "Keep" }).click();
    await expect(page.getByRole("button", { name: "Delete permanently" })).toBeVisible();

    await page.getByRole("button", { name: "Delete permanently" }).click();
    await page.getByRole("button", { name: /Delete for good/ }).click();

    await expect(rows).toHaveCount(3);
    await expect(page.getByText("Deleted permanently.")).toBeVisible();
    // There is nothing to undo.
    await expect(page.getByRole("button", { name: "Undo" })).toHaveCount(0);
    // The next message is not left half-way to being deleted.
    await expect(page.getByRole("button", { name: "Delete permanently" })).toBeVisible();
});

test("a message can be sent to junk and brought back", async ({ page }) => {
    await withCore(page, { accounts: [robin], mail: inbox });
    const rows = messages(page).getByRole("option");
    const pane = page.getByRole("region", { name: "Message" });
    await rows.nth(3).click();
    await expect(pane.getByRole("heading")).toHaveText("Action needed: confirm your password");

    await page.getByRole("button", { name: "Junk", exact: true }).click();
    await expect(rows).toHaveCount(3);
    await expect(page.getByText("Moved to Junk Email")).toBeVisible();

    await folder(page, "Junk Email").click();
    await rows.first().click();
    await page.getByRole("button", { name: "Not junk" }).click();
    await expect(page.getByText("Moved to the inbox")).toBeVisible();
    await folder(page, "Inbox").click();
    await expect(rows).toHaveCount(4);
});

test("a message is given categories, shown in their colours, and that can be undone", async ({ page }) => {
    await withCore(page, { accounts: [robin], mail: inbox });
    const first = messages(page).getByRole("option").first();
    const pane = page.getByRole("region", { name: "Message" });
    await first.click();

    await page.getByRole("toolbar", { name: "Message actions" }).locator('summary[aria-label="Categories"]').click();
    const menu = page.getByRole("list", { name: "Categories" });
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
    await expect(page.getByRole("button", { name: "Reply", exact: true })).toBeDisabled();

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

    await actions.getByLabel("Flag for…").click();
    const days = actions.getByRole("list", { name: "Follow up by" });
    await expect(days.getByRole("button", { name: /This week/ })).toContainText(/Fri,? (9 Oct|Oct 9)/);
    await expect(days.getByRole("button", { name: /Next week/ })).toContainText(/Mon,? (12 Oct|Oct 12)/);
    await days.getByRole("button", { name: /Tomorrow/ }).click();

    await expect(actions).toContainText(/Due Sat,? (10 Oct|Oct 10)/);
    await expect(page.getByText(/Flagged for Sat,? (10 Oct|Oct 10)/)).toBeVisible();
    expect((await ran(page)).find((r) => r.command === "mail.messages.set_flag")?.args).toMatchObject({ flag: "flagged", due: "2026-10-10" });
    await picture(page, "flag-due");

    // Another day, typed.
    await actions.getByLabel("Due…").click();
    await days.getByLabel("Another day").fill("2026-11-02");
    await expect(actions).toContainText(/Due Mon,? (2 Nov|Nov 2)/);

    await actions.getByLabel("Due…").click();
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

/** The smallest PNG there is: one clear pixel. */
const PIXEL = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";

test("a picture is put within a message, shown there, and sent by its name", async ({ page }) => {
    await withCore(page, { accounts: [robin], mail: inbox });
    await page.getByRole("button", { name: "New message" }).click();
    const form = page.getByRole("form", { name: "New message" });
    const words = form.getByRole("textbox", { name: "Message" });
    await form.getByRole("combobox", { name: "To" }).fill("jo@example.test");
    await words.click();
    await page.keyboard.type("Here is the chart:");

    await form.getByLabel("Choose pictures").setInputFiles({ name: "chart.png", mimeType: "image/png", buffer: Buffer.from(PIXEL, "base64") });

    // It shows in the message, and is not listed as a file beside it.
    const image = words.locator("img");
    await expect(image).toHaveCount(1);
    await expect(image).toHaveAttribute("src", /^data:image\/png;base64,/);
    await expect(form.getByRole("list", { name: "Attached files" })).toHaveCount(0);

    // Put aside and opened again, it is still shown.
    await page.keyboard.press("Escape");
    await page.getByRole("list", { name: "Being written" }).getByRole("button").first().click();
    await expect(words.locator("img")).toHaveAttribute("src", /^data:image\/png;base64,/);

    await form.getByRole("button", { name: "Send" }).click();
    await expect(form).toHaveCount(0);
    const went = (await sent(page))[0] as { body: string; files: { content_id: string | null }[] };
    // What is kept and sent refers to the picture by name, and does not carry it in the words.
    expect(went.body).toContain('src="cid:picture-1@brolga"');
    expect(went.body).not.toContain("base64");
    expect(went.files.map((file) => file.content_id)).toEqual(["picture-1@brolga"]);
});

test("a picture taken out of a message stops being one of its files", async ({ page }) => {
    await withCore(page, { accounts: [robin], mail: inbox });
    await page.getByRole("button", { name: "New message" }).click();
    const form = page.getByRole("form", { name: "New message" });
    const words = form.getByRole("textbox", { name: "Message" });
    await form.getByRole("combobox", { name: "To" }).fill("jo@example.test");
    await words.click();
    await form.getByLabel("Choose pictures").setInputFiles({ name: "chart.png", mimeType: "image/png", buffer: Buffer.from(PIXEL, "base64") });
    await expect(words.locator("img")).toHaveCount(1);

    await words.click();
    await page.keyboard.press("Control+a");
    await page.keyboard.type("No picture after all.");
    await form.getByRole("button", { name: "Send" }).click();

    await expect(form).toHaveCount(0);
    expect(((await sent(page))[0] as { files: unknown[] }).files).toHaveLength(0);
});

test("files dropped on a message being written are attached to it", async ({ page }) => {
    await withCore(page, { accounts: [robin], mail: inbox });
    await page.getByRole("button", { name: "New message" }).click();
    const form = page.getByRole("form", { name: "New message" });

    const carried = await page.evaluateHandle(() => {
        const data = new DataTransfer();
        data.items.add(new File(["Some notes."], "notes.txt", { type: "text/plain" }));
        data.items.add(new File(["a,b"], "figures.csv", { type: "text/csv" }));
        return data;
    });
    await form.dispatchEvent("dragover", { dataTransfer: carried });
    await expect(form).toHaveClass(/over/);
    await form.dispatchEvent("drop", { dataTransfer: carried });

    const attached = form.getByRole("list", { name: "Attached files" });
    await expect(attached.getByRole("listitem")).toHaveCount(2);
    await expect(attached).toContainText("notes.txt");
    await expect(attached).toContainText("figures.csv");
    await expect(form).not.toHaveClass(/over/);
    const added = (await ran(page)).filter((r) => r.command === "mail.drafts.files.add");
    expect(added[0].args).toMatchObject({ name: "notes.txt", content_base64: "U29tZSBub3Rlcy4=" });
});

test("the actions are icons along the top, each with its name on hover, greyed out until a message is open", async ({ page }) => {
    await withCore(page, { accounts: [robin], mail: inbox });
    const bar = page.getByRole("toolbar", { name: "Message actions" });

    // With nothing open there is nothing to reply to or delete; a new message can always be started.
    await expect(bar.getByRole("button", { name: "Reply", exact: true })).toBeDisabled();
    await expect(bar.getByRole("button", { name: "Delete", exact: true })).toBeDisabled();
    await expect(bar.getByRole("button", { name: "New message" })).toBeEnabled();
    await expect(bar.getByRole("button", { name: "Sync" })).toBeVisible();

    await messages(page).getByRole("option").first().click();
    const reply = bar.getByRole("button", { name: "Reply", exact: true });
    await expect(reply).toBeEnabled();
    // An icon alone, with its name and its key in the hover text.
    await expect(reply).toHaveText("");
    await expect(reply).toHaveAttribute("title", "Reply (Ctrl+R)");
    await expect(bar.getByRole("button", { name: "Delete", exact: true })).toHaveAttribute("title", "Delete (Del)");
    await picture(page, "mail");

    // While a message is being written, the actions for the one behind it are out of reach.
    await bar.getByRole("button", { name: "New message" }).click();
    await expect(reply).toBeDisabled();
});

test("a search can be of every folder or only the one shown", async ({ page }) => {
    await withCore(page, { accounts: [robin], mail: [...inbox, ...bulk(3)] });
    const rows = messages(page).getByRole("option");
    const box = page.getByRole("searchbox", { name: "Search mail" });
    const where = page.getByRole("combobox", { name: "Search in" });

    await box.fill("message");
    // Every folder: the three in the archive are found from the inbox.
    await expect(where).toHaveValue("all");
    await expect(rows).toHaveCount(3);
    await expect(page.getByText("in all folders")).toBeVisible();

    await where.selectOption("folder");
    await expect(page.getByText("0 results")).toBeVisible();
    await expect(page.getByText("in Inbox")).toBeVisible();
    expect((await ran(page)).filter((r) => r.command === "mail.search").at(-1)?.args).toMatchObject({ folder_id: "inbox" });

    // The choice is kept.
    await page.reload();
    await expect(where).toHaveValue("folder");
});

test("pausing over a sender's name shows who they are, their recent mail and the meetings to come", async ({ page }) => {
    await withCore(page, {
        accounts: [robin],
        mail: inbox,
        events: [{ id: "e9", subject: "Supplier review", start: "2026-10-20T01:00:00Z", end: "2026-10-20T02:00:00Z" }],
        directory: [{ name: "Sam Taylor", address: "sam.taylor@example.test", job_title: "Buyer", department: "Purchasing" }],
    });
    await messages(page).getByRole("option", { name: /Budget review moved/ }).click();
    const sender = page.getByRole("button", { name: "Sam Taylor", exact: true });

    await sender.hover();
    const card = page.getByRole("dialog", { name: "About Sam Taylor" });
    await expect(card).toContainText("Buyer · Purchasing");
    await expect(card).toContainText("sam.taylor@example.test");
    await expect(card).toContainText("In your organisation");
    await expect(card.getByRole("list", { name: "Recent mail" }).getByRole("listitem")).toContainText(["Budget review moved to Thursday"]);
    await expect(card.getByRole("list", { name: "Coming up together" }).getByRole("listitem")).toContainText(["Supplier review"]);
    expect((await ran(page)).find((r) => r.command === "people.card")?.args).toMatchObject({ account_id: robin.id, address: "sam.taylor@example.test" });
    await picture(page, "contact-card");

    // Moving away puts it down, and the name itself works by key as well as by mouse.
    await page.getByRole("searchbox").hover();
    await expect(card).toHaveCount(0);
    await sender.focus();
    await page.keyboard.press("Enter");
    await expect(card).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(card).toHaveCount(0);
});

test("a sender nobody knows still has a card, with their mail and nothing claimed about who they are", async ({ page }) => {
    await withCore(page, { accounts: [robin], mail: inbox });
    await messages(page).getByRole("option", { name: /Action needed/ }).click();
    await page.getByRole("button", { name: "IT Support", exact: true }).click();

    const card = page.getByRole("dialog", { name: "About IT Support" });
    await expect(card.getByRole("list", { name: "Recent mail" }).getByRole("listitem")).toHaveCount(1);
    await expect(card).not.toContainText("In your");
    await expect(card.getByRole("list", { name: "Coming up together" })).toHaveCount(0);
});

test("Shift and the arrow keys choose a run of messages, which are deleted together and brought back with one undo", async ({ page }) => {
    await withCore(page, { accounts: [robin], mail: inbox });
    const rows = messages(page).getByRole("option");
    const chosen = messages(page).getByRole("option", { selected: true });
    const picked = page.getByRole("region", { name: "Messages selected" });
    await rows.first().click();

    await page.keyboard.press("Shift+ArrowDown");
    await expect(chosen).toHaveCount(2);
    await expect(picked).toContainText("2 messages selected");
    await page.keyboard.press("Shift+ArrowDown");
    await expect(picked).toContainText("3 messages selected");
    // Back up the way it came: the run is from where it began to here.
    await page.keyboard.press("Shift+ArrowUp");
    await expect(chosen).toHaveCount(2);
    // Answering is for one message.
    await expect(page.getByRole("button", { name: "Reply", exact: true })).toBeDisabled();

    await page.keyboard.press("Delete");
    await expect(rows).toHaveCount(2);
    await expect(page.getByText("Moved to Deleted Items: 2 messages")).toBeVisible();
    expect((await ran(page)).find((r) => r.command === "mail.messages.move")?.args).toMatchObject({ message_ids: ["m1", "m2"], folder_id: "deleteditems" });
    // The next message takes their place, alone.
    await expect(chosen).toHaveCount(1);
    await expect(page.getByRole("region", { name: "Message" }).getByRole("heading")).toHaveText("Invoice 2041 attached");

    await page.getByRole("button", { name: "Undo" }).click();
    await expect(rows).toHaveCount(4);
});

test("Ctrl and Shift with a click choose messages, Ctrl+A chooses them all, and Escape goes back to one", async ({ page }) => {
    await withCore(page, { accounts: [robin], mail: inbox });
    const rows = messages(page).getByRole("option");
    const chosen = messages(page).getByRole("option", { selected: true });
    const picked = page.getByRole("region", { name: "Messages selected" });
    await rows.first().click();

    await rows.nth(2).click({ modifiers: ["ControlOrMeta"] });
    await expect(picked).toContainText("2 messages selected");
    await expect(rows.nth(1)).toHaveAttribute("aria-selected", "false");
    await rows.nth(2).click({ modifiers: ["ControlOrMeta"] });
    await expect(picked).toHaveCount(0);
    await expect(page.getByRole("region", { name: "Message" }).getByRole("heading")).toHaveText("Budget review moved to Thursday");

    await rows.nth(3).click({ modifiers: ["Shift"] });
    await expect(picked).toContainText("4 messages selected");
    await picture(page, "several-selected");

    await page.keyboard.press("Escape");
    await expect(chosen).toHaveCount(1);
    await expect(page.getByRole("region", { name: "Message" }).getByRole("heading")).toHaveText("Action needed: confirm your password");

    await page.keyboard.press("Control+a");
    await expect(picked).toContainText("4 messages selected");
    await picked.getByRole("button", { name: "Clear the selection" }).click();
    await expect(chosen).toHaveCount(1);
});

test("several messages are marked, flagged and given a category together, each keeping the categories it had", async ({ page }) => {
    await withCore(page, { accounts: [robin], mail: inbox });
    const rows = messages(page).getByRole("option");
    const actions = page.getByRole("toolbar", { name: "Message actions" });
    const menu = page.getByRole("list", { name: "Categories" });
    // The second has a category of its own before any are chosen together.
    await rows.nth(1).click();
    await actions.locator('summary[aria-label="Categories"]').click();
    await menu.getByRole("checkbox", { name: "Red category" }).check();
    await expect(rows.nth(1).getByLabel("Category Red category")).toBeVisible();

    await rows.nth(2).click({ modifiers: ["Shift"] });
    await expect(page.getByRole("region", { name: "Messages selected" })).toContainText("2 messages selected");

    await actions.getByRole("button", { name: "Mark as unread" }).click();
    await expect(page.getByText("Marked as unread: 2 messages")).toBeVisible();
    await expect(rows.nth(1)).toHaveClass(/unread/);
    await expect(rows.nth(2)).toHaveClass(/unread/);
    await page.keyboard.press("Control+q");
    await expect(rows.nth(2)).not.toHaveClass(/unread/);

    await page.keyboard.press("Insert");
    await expect(rows.nth(1).getByLabel("Flagged")).toBeVisible();
    await expect(rows.nth(2).getByLabel("Flagged")).toBeVisible();

    // The menu is still open. One of the two has it, so the box is neither ticked nor clear.
    await expect(menu.getByRole("checkbox", { name: "Red category" })).toHaveJSProperty("indeterminate", true);
    await menu.getByRole("checkbox", { name: "Clients" }).check();
    await expect(rows.nth(1).getByLabel("Category Clients")).toBeVisible();
    await expect(rows.nth(1).getByLabel("Category Red category")).toBeVisible();
    await expect(rows.nth(2).getByLabel("Category Clients")).toBeVisible();
    await expect(rows.nth(2).getByLabel("Category Red category")).toHaveCount(0);

    // One undo takes it back from both.
    await page.getByRole("button", { name: "Undo" }).click();
    await expect(rows.nth(1).getByLabel("Category Clients")).toHaveCount(0);
    await expect(rows.nth(2).getByLabel("Category Clients")).toHaveCount(0);
    await expect(rows.nth(1).getByLabel("Category Red category")).toBeVisible();
});

test("in Deleted Items several messages are deleted for good together, after asking twice", async ({ page }) => {
    await withCore(page, { accounts: [robin], mail: inbox.map((m) => ({ ...m, folder_id: "deleted" })) });
    await folder(page, "Deleted Items").click();
    const rows = messages(page).getByRole("option");
    await rows.first().click();
    await rows.nth(2).click({ modifiers: ["Shift"] });

    await page.getByRole("button", { name: "Delete permanently" }).click();
    await page.getByRole("button", { name: "Delete 3 for good? It cannot be undone" }).click();

    await expect(rows).toHaveCount(1);
    await expect(page.getByText("3 messages deleted permanently.")).toBeVisible();
    await expect(page.getByRole("button", { name: "Undo" })).toHaveCount(0);
});

test("each line has its sender's initials, and under the mouse a box to choose it and buttons to flag and delete it", async ({ page }) => {
    await withCore(page, { accounts: [robin], mail: inbox });
    const rows = messages(page).getByRole("option");
    const pane = page.getByRole("region", { name: "Message" });
    await expect(rows.first().locator(".circle")).toHaveText("ST");
    await rows.first().click();
    await expect(pane.getByRole("heading")).toHaveText("Budget review moved to Thursday");

    // Flagging another line from the line leaves what is being read alone.
    await rows.nth(2).hover();
    await rows.nth(2).getByTitle("Flag", { exact: true }).click();
    await expect(rows.nth(2).getByLabel("Flagged")).toBeVisible();
    await expect(pane.getByRole("heading")).toHaveText("Budget review moved to Thursday");

    await rows.nth(2).getByTitle("Delete").click();
    await expect(rows).toHaveCount(3);
    await expect(page.getByText("Moved to Deleted Items")).toBeVisible();
    await expect(pane.getByRole("heading")).toHaveText("Budget review moved to Thursday");

    // The box chooses a line along with the one being read.
    await rows.nth(1).hover();
    await rows.nth(1).getByTitle("Select").click();
    await expect(page.getByRole("region", { name: "Messages selected" })).toContainText("2 messages selected");
});

test("a message that has been answered or sent on is marked on its line", async ({ page }) => {
    const did = ["replied", "replied_all", "forwarded", undefined];
    await withCore(page, { accounts: [robin], mail: inbox.map((m, i) => ({ ...m, last_action: did[i] })) });
    const rows = messages(page).getByRole("option");

    await expect(rows.nth(0).getByLabel("Replied", { exact: true })).toBeVisible();
    await expect(rows.nth(1).getByLabel("Replied to all")).toBeVisible();
    await expect(rows.nth(2).getByLabel("Forwarded")).toBeVisible();
    await expect(rows.nth(3).locator(".done")).toHaveCount(0);
});
