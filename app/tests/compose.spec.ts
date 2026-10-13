import { type Page, expect, test } from "@playwright/test";
import { isAddress, lastStarts, lineOf, peopleOn } from "../src/lib/people-line";
import { type FakeMail, inbox, ran, robin, sent, withCore } from "./core";

const PIXEL = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==";

/** Someone whose name is written surname first, with a comma in it. */
const taylor = { name: "Taylor, Sam", address: "sam.taylor@example.test" };
const fromTaylor: FakeMail[] = [{ ...inbox[0], from: taylor }];

const messages = (page: Page) => page.getByRole("listbox", { name: "Messages" });
const edits = async (page: Page) => (await ran(page)).filter((r) => r.command === "mail.drafts.edit").map((r) => r.args);

async function newMessage(page: Page, core: Parameters<typeof withCore>[1] = {}) {
    await withCore(page, { accounts: [robin], mail: inbox, ...core });
    await page.getByRole("button", { name: "New message" }).click();
    return page.getByRole("form", { name: "New message" });
}

test("a line of people is written and read back, whatever is in a name", () => {
    const people = [
        { name: "Taylor, Sam", address: "sam@example.test" },
        { name: 'Jo "JJ" Nguyen; Sales', address: "jo@example.test" },
        { name: "Alex <Ops>", address: "alex@example.test" },
        { name: "Plain Name", address: "plain@example.test" },
        { name: null, address: "bare@example.test" },
    ];
    const line = lineOf(people);

    expect(line).toBe('"Taylor, Sam" <sam@example.test>, "Jo \\"JJ\\" Nguyen; Sales" <jo@example.test>, "Alex <Ops>" <alex@example.test>, Plain Name <plain@example.test>, bare@example.test');
    expect(peopleOn(line)).toEqual(people);
    // A name that is only the address again is not written twice.
    expect(lineOf([{ name: "jo@example.test", address: "jo@example.test" }])).toBe("jo@example.test");
});

test("a line of people is read as it is typed: commas, semicolons, new lines, and what is unfinished", () => {
    expect(peopleOn("a@example.test; b@example.test\nSam Taylor <sam@example.test>,, ")).toEqual([
        { name: null, address: "a@example.test" },
        { name: null, address: "b@example.test" },
        { name: "Sam Taylor", address: "sam@example.test" },
    ]);
    // What is not a name and an address is all address, for whoever checks it to refuse.
    expect(peopleOn("sam")).toEqual([{ name: null, address: "sam" }]);
    expect(peopleOn("Sam <sam@example.test> junk")).toEqual([{ name: null, address: "Sam <sam@example.test> junk" }]);
    expect(peopleOn('"Taylor, S')).toEqual([{ name: null, address: '"Taylor, S' }]);
    expect(peopleOn("<sam@example.test>")).toEqual([{ name: null, address: "sam@example.test" }]);

    // The one being typed starts after the last comma that parts two people, not one within a name.
    const line = '"Taylor, Sam" <sam@example.test>, jo';
    expect(line.slice(lastStarts(line))).toBe(" jo");
    expect(lastStarts('"Taylor, Sa')).toBe(0);

    expect(isAddress("sam@example.test")).toBe(true);
    for (const wrong of ["sam", "sam@example", "@example.test", "sam@.test", "sam@example.", "sam taylor@example.test", "sam@example.test>"]) expect(isAddress(wrong)).toBe(false);
});

test("a reply to someone with a comma in their name is addressed to them, and sent", async ({ page }) => {
    await withCore(page, { accounts: [robin], mail: fromTaylor });
    await messages(page).getByRole("option").first().click();
    await page.getByRole("button", { name: "Reply", exact: true }).click();
    const form = page.getByRole("form", { name: "Reply" });

    await expect(form.getByRole("combobox", { name: "To" })).toHaveValue('"Taylor, Sam" <sam.taylor@example.test>');
    await page.keyboard.type("Thursday is fine.");
    await expect(form.getByText("Draft kept on this computer")).toBeVisible();
    await form.getByRole("button", { name: "Send" }).click();

    await expect(form).toHaveCount(0);
    expect((await sent(page))[0]).toMatchObject({ kind: "reply", to: [taylor] });
});

test("someone suggested with a comma in their name is taken as one person, and the next is looked up alone", async ({ page }) => {
    const form = await newMessage(page, { mail: fromTaylor });
    const to = form.getByRole("combobox", { name: "To" });

    await to.pressSequentially("tay");
    await expect(form.getByRole("option")).toHaveCount(1);
    await page.keyboard.press("Enter");
    await expect(to).toHaveValue('"Taylor, Sam" <sam.taylor@example.test>, ');

    await to.pressSequentially("sam");
    await expect(form.getByRole("option")).toHaveCount(1);
    expect((await ran(page)).filter((r) => r.command === "mail.people.suggest").at(-1)?.args).toMatchObject({ typed: "sam" });
    await page.keyboard.press("Enter");
    // The same person again, after themselves and not in place of half their name.
    await expect(to).toHaveValue('"Taylor, Sam" <sam.taylor@example.test>, "Taylor, Sam" <sam.taylor@example.test>, ');
    await to.fill('"Taylor, Sam" <sam.taylor@example.test>, jo@example.test');
    await form.getByRole("button", { name: "Send" }).click();

    await expect(form).toHaveCount(0);
    expect((await sent(page))[0]).toMatchObject({ to: [taylor, { name: null, address: "jo@example.test" }] });
});

test("someone invited with a comma in their name is still one person when the meeting is changed", async ({ page }) => {
    await page.clock.setFixedTime(new Date("2026-10-14T00:20:00Z"));
    await withCore(page, {
        accounts: [robin],
        events: [{ id: "k1", subject: "Kick-off", start: "2026-10-14T03:00:00Z", end: "2026-10-14T04:00:00Z", mine: true, attendees: [{ ...taylor, kind: "required", response: "none" }] }],
    });
    await page.getByRole("button", { name: "Calendar", exact: true }).click();
    await page.getByRole("region", { name: "Calendar" }).getByRole("button", { name: /Kick-off/ }).click();
    const panel = page.getByRole("complementary", { name: "Event" });
    await panel.getByRole("button", { name: "Change" }).click();
    const form = panel.getByRole("form", { name: "Change event" });
    const invite = form.getByRole("combobox", { name: "Invite" });

    await expect(invite).toHaveValue('"Taylor, Sam" <sam.taylor@example.test>');
    await invite.fill('"Taylor, Sam" <sam.taylor@example.test>, jo@example.test');
    await form.getByRole("button", { name: "Save" }).click();

    await expect(form).toHaveCount(0);
    expect((await ran(page)).filter((r) => r.command === "calendar.events.update").at(-1)?.args.attendees).toEqual([
        { address: "sam.taylor@example.test", name: "Taylor, Sam", kind: "required" },
        { address: "jo@example.test", kind: "required" },
    ]);
});

test("Enter in a line of the heading goes on to the next, and does not send", async ({ page }) => {
    const form = await newMessage(page);
    const to = form.getByRole("combobox", { name: "To" });
    const subject = form.getByRole("textbox", { name: "Subject" });

    await to.fill("jo@example.test");
    await to.press("Enter");
    await expect(subject).toBeFocused();
    await page.keyboard.type("Lunch on Friday");
    await page.keyboard.press("Enter");
    await expect(form.getByRole("textbox", { name: "Message" })).toBeFocused();
    await page.keyboard.type("Are you free?");

    await expect(form.getByText("Draft kept on this computer")).toBeVisible();
    expect(await sent(page)).toHaveLength(0);
    await expect(form).toBeVisible();

    // With the copies showing, To goes on to Cc.
    await form.getByRole("button", { name: "Cc Bcc" }).click();
    await to.press("Enter");
    await expect(form.getByRole("combobox", { name: "Cc", exact: true })).toBeFocused();

    // Sending is still one key away.
    await page.keyboard.press("Control+Enter");
    await expect(form).toHaveCount(0);
    expect((await sent(page))[0]).toMatchObject({ subject: "Lunch on Friday", to: [{ name: null, address: "jo@example.test" }] });
});

test("an address half typed does not stop the rest being kept, and is said before sending", async ({ page }) => {
    const form = await newMessage(page);
    const to = form.getByRole("combobox", { name: "To" });

    await to.fill("jo@example.test, sam");
    await form.getByRole("textbox", { name: "Subject" }).fill("Half addressed");
    await form.getByRole("textbox", { name: "Message" }).click();
    await page.keyboard.type("None of this is to be lost.");

    // What can be kept is, and what cannot is said quietly and left where it was typed.
    await expect(form.getByText("Draft kept on this computer")).toBeVisible();
    await expect(form.getByText("`sam` is not an email address yet, so it is not kept with the draft.")).toBeVisible();
    await expect(form.getByRole("alert")).toHaveCount(0);
    await expect(to).toHaveValue("jo@example.test, sam");
    expect((await edits(page)).at(-1)).toMatchObject({ to: [{ name: null, address: "jo@example.test" }], subject: "Half addressed" });
    expect(String((await edits(page)).at(-1)?.body)).toContain("None of this is to be lost.");

    // It is not sent to some of those it was meant for.
    await form.getByRole("button", { name: "Send" }).click();
    await expect(form.getByRole("alert")).toContainText("`sam` is not an email address. Finish it or take it out before sending.");
    expect(await sent(page)).toHaveLength(0);

    // It can still be put aside, and is there to come back to.
    await page.keyboard.press("Escape");
    await expect(form).toHaveCount(0);
    await page.getByRole("list", { name: "Being written" }).getByRole("button", { name: "Half addressed" }).click();
    await expect(form.getByRole("textbox", { name: "Message" })).toHaveText("None of this is to be lost.");
    await expect(to).toHaveValue("jo@example.test");
});

test("throwing a draft away stops being one click off when the focus moves on, or a few seconds pass", async ({ page }) => {
    await page.clock.install();
    const form = await newMessage(page);

    await form.getByRole("button", { name: "Discard" }).click();
    await expect(form.getByRole("button", { name: "Discard for good?" })).toBeVisible();
    await form.getByRole("textbox", { name: "Subject" }).click();
    await expect(form.getByRole("button", { name: "Discard for good?" })).toHaveCount(0);

    await form.getByRole("button", { name: "Discard" }).click();
    await expect(form.getByRole("button", { name: "Discard for good?" })).toBeVisible();
    await page.clock.runFor(6_000);
    await expect(form.getByRole("button", { name: "Discard for good?" })).toHaveCount(0);
    await expect(form.getByRole("button", { name: "Discard" })).toBeVisible();
});

test("what was typed a moment before leaving a message is kept as it is left", async ({ page }) => {
    const form = await newMessage(page);
    await form.getByRole("textbox", { name: "Subject" }).fill("First");
    await expect(form.getByText("Draft kept on this computer")).toBeVisible();
    await page.getByRole("button", { name: "New message" }).click();
    await expect(page.getByRole("list", { name: "Being written" }).getByRole("button")).toHaveCount(2);
    await form.getByRole("textbox", { name: "Subject" }).fill("Second");
    await form.getByRole("textbox", { name: "Message" }).click();
    await page.keyboard.type("Typed just before going.");

    // Straight to the other draft, sooner than what is typed would have been kept by itself.
    await page.getByRole("list", { name: "Being written" }).getByRole("button", { name: "First" }).click();

    const last = (await edits(page)).at(-1);
    expect(last).toMatchObject({ subject: "Second" });
    expect(String(last?.body)).toContain("Typed just before going.");
});

test("a picture deleted and brought back with undo is still sent", async ({ page }) => {
    const form = await newMessage(page);
    const words = form.getByRole("textbox", { name: "Message" });
    await form.getByRole("combobox", { name: "To" }).fill("jo@example.test");
    await words.click();
    await form.getByLabel("Choose pictures").setInputFiles({ name: "chart.png", mimeType: "image/png", buffer: Buffer.from(PIXEL, "base64") });
    await expect(words.locator("img")).toHaveCount(1);
    await expect(form.getByText("Draft kept on this computer")).toBeVisible();

    await words.click();
    await page.keyboard.press("Control+a");
    await page.keyboard.press("Delete");
    await expect(words.locator("img")).toHaveCount(0);
    await expect(form.getByText("Draft kept on this computer")).toBeVisible();
    // Its file is not thrown away while the message is still being written.
    expect((await ran(page)).filter((r) => r.command === "mail.drafts.files.remove")).toHaveLength(0);

    await page.keyboard.press("Control+z");
    await expect(words.locator("img")).toHaveCount(1);
    await form.getByRole("button", { name: "Send" }).click();

    await expect(form).toHaveCount(0);
    const went = (await sent(page))[0] as { body: string; files: { content_id: string | null }[] };
    expect(went.body).toContain('src="cid:picture-1@brolga"');
    expect(went.files.map((file) => file.content_id)).toEqual(["picture-1@brolga"]);
});

test("pictures in a draft written in another mail program are shown, kept through a change, and sent", async ({ page }) => {
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
                // One attached to the draft, one carried in the body, and two attached that the core did not bring.
                body: `<p>Dear Jo,</p><img src="cid:logo@example"><img src="data:image/png;base64,${PIXEL}"><img src="CID:odd@example"><img src="cid:large@example">`,
                pictures: [{ id: 1, name: "logo.png", content_type: "image/png", size: 68, content_id: "logo@example", content_base64: PIXEL }],
                blocked: 2,
            },
        ],
    });
    await page.getByRole("list", { name: "Folders" }).getByRole("button", { name: /^Drafts/ }).click();
    await messages(page).getByRole("option").first().click();
    const form = page.getByRole("form", { name: "New message" });
    const words = form.getByRole("textbox", { name: "Message" });

    await expect(words.locator("img")).toHaveCount(4);
    await expect(words.locator("img").first()).toHaveAttribute("src", `data:image/png;base64,${PIXEL}`);
    await expect(form.getByText("with 2 pictures kept on the web")).toBeVisible();
    await expect(form.getByText("2 pictures in this draft cannot be shown here")).toBeVisible();
    // Only looked at, nothing of it is changed.
    expect((await ran(page)).filter((r) => r.command === "mail.drafts.files.add" || r.command === "mail.drafts.edit")).toHaveLength(0);

    await words.click();
    await page.keyboard.press("Control+End");
    await page.keyboard.type("See you Friday.");
    await expect(form.getByText("Draft kept on this computer")).toBeVisible();

    const kept = String((await edits(page)).at(-1)?.body);
    expect(kept).toContain("See you Friday.");
    for (const name of ["logo@example", "picture-2@brolga", "odd@example", "large@example"]) expect(kept).toContain(`src="cid:${name}"`);
    // The one carried in the body is a file of the draft now, as one put in here is.
    expect(kept).not.toContain("data:image");

    await form.getByRole("button", { name: "Send" }).click();
    await expect(form).toHaveCount(0);
    const went = (await sent(page))[0] as { body: string; files: { content_id: string | null }[] };
    expect(went.files.map((file) => file.content_id)).toEqual(["logo@example", "picture-2@brolga"]);
    expect((await ran(page)).filter((r) => r.command === "mail.drafts.files.remove")).toHaveLength(0);
});

test("a file dialog that cannot be opened is said", async ({ page }) => {
    const form = await newMessage(page, { pickFails: "The file dialog could not be opened." });

    await form.getByRole("button", { name: "Attach…" }).click();

    await expect(form.getByRole("alert")).toContainText("The file dialog could not be opened.");
    await expect(form.getByRole("button", { name: "Attach…" })).toBeEnabled();
});

test("a draft from an address that is no longer offered shows who it is from", async ({ page }) => {
    const form = await newMessage(page);
    await form.getByRole("textbox", { name: "Subject" }).fill("From elsewhere");
    await expect(form.getByText("Draft kept on this computer")).toBeVisible();
    // The draft was set, somewhere else, to go out under an address this mailbox no longer offers.
    await page.evaluate(() =>
        (window as unknown as { __TAURI_INTERNALS__: { invoke: (name: string, payload: unknown) => Promise<unknown> } }).__TAURI_INTERNALS__.invoke("execute_command", {
            command: "mail.drafts.edit",
            args: { account_id: "tenant-1.robin", draft_id: "draft-1", from: "sales@example.test" },
        }),
    );
    await page.keyboard.press("Escape");
    await expect(form).toHaveCount(0);

    await page.getByRole("list", { name: "Being written" }).getByRole("button", { name: "From elsewhere" }).click();

    const from = form.getByRole("combobox", { name: "From" });
    await expect(from).toHaveValue("sales@example.test");
    await expect(from.getByRole("option")).toHaveText(["robin@example.test", "sales@example.test"]);
});

test("two people suggested with one address are both listed, and the list still changes", async ({ page }) => {
    const twice = [
        { name: "Sam Taylor", address: "sam.taylor@example.test" },
        { name: "Sam (work)", address: "sam.taylor@example.test" },
        { name: "Jo Nguyen", address: "jo.nguyen@example.test" },
    ];
    const form = await newMessage(page, { suggested: twice });
    const to = form.getByRole("combobox", { name: "To" });

    await to.pressSequentially("sa");
    await expect(form.getByRole("option")).toHaveText(["Sam Taylor sam.taylor@example.test", "Sam (work) sam.taylor@example.test", "Jo Nguyen jo.nguyen@example.test"]);
    await page.keyboard.press("ArrowDown");
    await page.keyboard.press("Enter");
    await expect(to).toHaveValue("Sam (work) <sam.taylor@example.test>, ");
});

test("an event cannot be saved to end before it starts", async ({ page }) => {
    await page.clock.setFixedTime(new Date("2026-10-14T00:20:00Z"));
    await withCore(page, { accounts: [robin], events: [] });
    await page.getByRole("button", { name: "Calendar", exact: true }).click();
    await page.getByRole("button", { name: "New event" }).click();
    const form = page.getByRole("form", { name: "New event" });

    await form.getByRole("textbox", { name: "Subject" }).fill("Backwards");
    await form.getByLabel("Start day").fill("2026-10-15");
    await form.getByLabel("Start time").fill("14:00");
    await form.getByLabel("End time").fill("13:00");
    await form.getByRole("button", { name: "Save" }).click();

    await expect(form.getByRole("alert")).toHaveText("It ends before it starts. Set the end to after the start.");
    expect((await ran(page)).filter((r) => r.command === "calendar.events.create")).toHaveLength(0);

    await form.getByLabel("End time").fill("15:00");
    await form.getByRole("button", { name: "Save" }).click();
    await expect(form).toHaveCount(0);
    expect((await ran(page)).filter((r) => r.command === "calendar.events.create")).toHaveLength(1);
});

test("a day that is both the fourth and the last of its kind in the month can repeat as either", async ({ page }) => {
    await page.clock.setFixedTime(new Date("2026-10-14T00:20:00Z"));
    await withCore(page, { accounts: [robin], events: [] });
    await page.getByRole("button", { name: "Calendar", exact: true }).click();
    const sentRepeat = async () => ((await ran(page)).filter((r) => r.command === "calendar.events.create").at(-1)?.args as Record<string, unknown>).repeat;
    const form = page.getByRole("form", { name: "New event" });
    const on = form.getByLabel("On which day");

    // Tuesday 27 October 2026: the fourth Tuesday of its month, and the last.
    for (const [choice, said, week] of [
        ["weekday", "The fourth Tuesday", "fourth"],
        ["last", "The last Tuesday", "last"],
    ]) {
        await page.getByRole("button", { name: "New event" }).click();
        await form.getByRole("textbox", { name: "Subject" }).fill("Board");
        await form.getByLabel("Start day").fill("2026-10-27");
        await form.getByLabel("Repeats").selectOption("month");
        await expect(on.getByRole("option")).toHaveText(["Day 27 of the month", "The fourth Tuesday", "The last Tuesday"]);
        await on.selectOption(choice);
        await expect(on.locator("option:checked")).toHaveText(said);
        await form.getByRole("button", { name: "Save" }).click();
        await expect(form).toHaveCount(0);
        expect(await sentRepeat()).toEqual({ unit: "month", every: 1, week_of_month: week, days_of_week: ["tuesday"] });
    }

    // Friday 30 October 2026 is the fifth Friday, so it is only ever the last.
    await page.getByRole("button", { name: "New event" }).click();
    await form.getByLabel("Start day").fill("2026-10-30");
    await form.getByLabel("Repeats").selectOption("month");
    await expect(on.getByRole("option")).toHaveText(["Day 30 of the month", "The last Friday"]);
});
