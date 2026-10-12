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
