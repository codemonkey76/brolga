import { type Page, expect, test } from "@playwright/test";
import { inbox, notified, ran, robin, week, withCore } from "./core";

const picture = async (page: Page, name: string) => {
    if (process.env.SCREENSHOTS) await page.screenshot({ path: `../docs/screenshots/${name}.png` });
};

/** The calendar, on Wednesday 14 October 2026, a little after ten in the morning in Brisbane. */
async function calendar(page: Page, events = week) {
    await page.clock.setFixedTime(new Date("2026-10-14T00:20:00Z"));
    await withCore(page, { accounts: [robin], events });
    await page.getByRole("button", { name: "Calendar", exact: true }).click();
    return page.getByRole("region", { name: "Calendar" });
}

const event = (page: Page, name: string) => page.getByRole("region", { name: "Calendar" }).getByRole("button", { name: new RegExp(name) });

test("the working week opens on this week, each event on its own day at its own time", async ({ page }) => {
    const view = await calendar(page);

    await expect(view.getByRole("heading", { level: 1 })).toHaveText(/12 Oct – 16 Oct 2026|Oct 12 – Oct 16 2026/);
    const wednesday = view.getByRole("list", { name: /Wednesday/ });
    await expect(wednesday.getByRole("listitem")).toHaveCount(4);
    await expect(wednesday).toContainText("Stand-up");
    // Nine in the morning in Brisbane is 23:00 UTC the day before: it is on Wednesday, not Tuesday.
    await expect(view.getByRole("list", { name: /Tuesday/ }).getByRole("listitem")).toHaveCount(0);
    // A later hour sits lower on the day.
    const top = async (name: string) => (await event(page, name).boundingBox())!.y;
    expect(await top("Stand-up")).toBeLessThan(await top("Budget review"));
    expect(await top("Budget review")).toBeLessThan(await top("London sync"));
    await picture(page, "calendar-week");
});

test("events that overlap sit side by side, and neither hides the other", async ({ page }) => {
    await calendar(page);

    const budget = (await event(page, "Budget review").boundingBox())!;
    const supplier = (await event(page, "Supplier call").boundingBox())!;

    expect(supplier.x).toBeGreaterThanOrEqual(budget.x + budget.width - 1);
    expect(Math.abs(budget.width - supplier.width)).toBeLessThan(3);
});

test("an all-day event is on each of its days, by the days it names and not by the zone", async ({ page }) => {
    const view = await calendar(page);

    // Thursday and Friday, as written; not shifted a day by being read as midnight UTC.
    await expect(view.getByRole("list", { name: /All day, .*Thu/ })).toContainText("Conference");
    await expect(view.getByRole("list", { name: /All day, .*Fri/ })).toContainText("Conference");
    await expect(view.getByRole("list", { name: /All day, .*Wed/ }).getByRole("listitem")).toHaveCount(0);
});

test("the day, the week, the month and the agenda each show what is theirs", async ({ page }) => {
    const view = await calendar(page);
    const views = view.getByRole("group", { name: "View" });

    await views.getByRole("button", { name: "Day" }).click();
    await expect(view.getByRole("heading", { level: 1 })).toContainText("Wednesday");
    await expect(event(page, "School pickup")).toHaveCount(0);
    await expect(event(page, "Budget review")).toBeVisible();

    await views.getByRole("button", { name: "Week", exact: true }).click();
    await expect(view.getByRole("list", { name: /Saturday|Sunday/ })).toHaveCount(2);
    await expect(event(page, "School pickup")).toBeVisible();

    await views.getByRole("button", { name: "Month" }).click();
    await expect(view.getByRole("heading", { level: 1 })).toHaveText("October 2026");
    await expect(event(page, "Old plan")).toBeVisible();
    // A crowded day shows the first few and counts the rest.
    await expect(view.getByRole("button", { name: "1 more" })).toBeVisible();
    await picture(page, "calendar-month");

    await views.getByRole("button", { name: "Agenda" }).click();
    await expect(view.getByRole("region", { name: /Wednesday/ })).toContainText("Budget review");
    await expect(view.getByText("Today ·")).toBeVisible();
    // Thirty days on from today: next month's event is in, Monday's is not.
    await expect(event(page, "Next month")).toBeVisible();
    await expect(event(page, "Old plan")).toHaveCount(0);

    // The view chosen is the one it opens on next time.
    await page.reload();
    await page.getByRole("button", { name: "Calendar", exact: true }).click();
    await expect(views.getByRole("button", { name: "Agenda" })).toHaveAttribute("aria-pressed", "true");
});

test("stepping on and back, and the small month, move what is shown; Today brings it back", async ({ page }) => {
    const view = await calendar(page);

    await view.getByRole("button", { name: "Later" }).click();
    await expect(view.getByRole("heading", { level: 1 })).toHaveText(/19 Oct – 23 Oct 2026|Oct 19 – Oct 23 2026/);
    await expect(event(page, "Budget review")).toHaveCount(0);

    await page.getByLabel("Month to steer by").getByRole("button", { name: /^(3 November 2026|November 3, 2026)$/ }).click();
    await expect(event(page, "Next month")).toBeVisible();

    await view.getByRole("button", { name: "Today" }).click();
    await expect(event(page, "Budget review")).toBeVisible();
    // Asked only for the days shown, each time.
    const asked = (await ran(page)).filter((r) => r.command === "calendar.events.list").at(-1)!.args;
    expect(asked.start).toBe("2026-10-11T14:00:00.000Z");
    expect(asked.end).toBe("2026-10-16T14:00:00.000Z");
});

test("an event opens with when, where, who and what, and a way to join", async ({ page }) => {
    await calendar(page);
    await event(page, "Budget review").click();
    const details = page.getByRole("complementary", { name: "Event" });

    await expect(details.getByRole("heading", { level: 2 })).toHaveText("Budget review");
    await expect(details).toContainText(/2:00\s?pm to 3:00\s?pm/i);
    await expect(details).toContainText("Harbour Room");
    await expect(details).toContainText("Sam Taylor");
    await expect(details).toContainText("Accepted");
    await expect(details).toContainText("optional");
    await expect(details).toContainText("Bring the Q3 numbers.");
    await picture(page, "calendar-event");

    await details.getByRole("button", { name: "Join online" }).click();
    expect((await ran(page)).find((r) => r.command === "app.open_link")?.args).toEqual({ url: "https://meet.example.test/budget" });

    await page.keyboard.press("Escape");
    await expect(details).toHaveCount(0);
});

test("an event made in another zone says so, and a cancelled one is struck through", async ({ page }) => {
    await calendar(page);

    await event(page, "London sync").click();
    const details = page.getByRole("complementary", { name: "Event" });
    // Eight in the morning in London is five in the afternoon in Brisbane.
    await expect(details).toContainText(/5:00\s?pm/i);
    await expect(details).toContainText("Made in Europe/London; shown in your time.");

    await page.getByRole("group", { name: "View" }).getByRole("button", { name: "Week", exact: true }).click();
    await event(page, "Old plan").click();
    await expect(details).toContainText("Cancelled");
    await expect(event(page, "Old plan")).toHaveClass(/cancelled/);
});

test("a calendar can be hidden, and its events go with it", async ({ page }) => {
    await calendar(page);
    await expect(event(page, "School pickup")).toBeVisible();

    await page.getByRole("complementary", { name: "Calendars" }).getByRole("checkbox", { name: "Family" }).uncheck();

    await expect(event(page, "School pickup")).toHaveCount(0);
    await expect(event(page, "Budget review")).toBeVisible();
    expect((await ran(page)).find((r) => r.command === "calendar.calendars.set_shown")?.args).toMatchObject({ calendar_id: "family", shown: false });
});

test("the calendar is dark when the desktop is", async ({ page }) => {
    await page.emulateMedia({ colorScheme: "dark" });
    await calendar(page);
    await expect(event(page, "Budget review")).toBeVisible();
    await picture(page, "calendar-dark");

    const ground = await page.evaluate(() => getComputedStyle(document.body).backgroundColor);
    const [r, g, b] = (ground.match(/[\d.]+/g) ?? []).map(Number);
    expect((r + g + b) / 3).toBeLessThan(ground.startsWith("color(") ? 0.32 : 80);
});

test("a reminder shows in the window and on the desktop, once, and can be put off or dismissed", async ({ page }) => {
    // Ten to two on Wednesday afternoon in Brisbane: the budget review is in ten minutes.
    await page.clock.install({ time: new Date("2026-10-14T03:50:00Z") });
    await withCore(page, { accounts: [robin], events: week });
    const reminders = page.getByRole("region", { name: "Reminders" });

    // It shows wherever in the app you are, not only in the calendar. This
    // morning's stand-up was never dismissed, so it is still there too.
    await expect(reminders.getByRole("article")).toHaveCount(2);
    await expect(reminders).toContainText("started 5 hours ago");
    await reminders.getByRole("article").filter({ hasText: "Stand-up" }).getByRole("button", { name: "Dismiss" }).click();
    await expect(reminders.getByRole("article")).toHaveCount(1);
    await expect(reminders).toContainText("Budget review");
    await expect(reminders).toContainText("in 10 minutes");
    await expect(reminders).toContainText("Harbour Room");
    expect((await notified(page)).find((n) => n.title === "Budget review")?.body).toMatch(/2:00\s?PM · in 10 minutes · Harbour Room/i);
    await page.screenshot({ path: process.env.SCREENSHOTS ? "../docs/screenshots/reminder.png" : "test-results/reminder.png" });

    // Looked for again, the desktop is not told a second time.
    await page.clock.fastForward(31_000);
    expect(await notified(page)).toHaveLength(2);

    await reminders.getByRole("combobox", { name: "Snooze Budget review" }).selectOption("5");
    await expect(reminders).toHaveCount(0);
    // Five minutes on it is back, and the desktop is told again.
    await page.clock.fastForward(5 * 60_000 + 31_000);
    await expect(reminders.getByRole("article")).toHaveCount(1);
    await expect(reminders).toContainText("in 4 minutes");
    expect((await notified(page)).filter((n) => n.title === "Budget review")).toHaveLength(2);

    // Its name goes to the calendar; dismissed, it is gone for good.
    await reminders.getByRole("button", { name: "Budget review" }).click();
    await expect(page.getByRole("region", { name: "Calendar" })).toBeVisible();
    await reminders.getByRole("button", { name: "Dismiss", exact: true }).click();
    await expect(reminders).toHaveCount(0);
    await page.clock.fastForward(60_000);
    await expect(reminders).toHaveCount(0);
});

test("an invitation in the inbox reads as one, says what it clashes with, and leads to its day", async ({ page }) => {
    await page.clock.setFixedTime(new Date("2026-10-09T06:00:00Z"));
    await withCore(page, {
        accounts: [robin],
        events: week,
        mail: [
            {
                id: "inv",
                folder_id: "inbox",
                subject: "Budget review",
                preview: "Please join the budget review.",
                from: { name: "Sam Taylor", address: "sam.taylor@example.test" },
                received_at: "2026-10-09T05:00:00Z",
                is_read: false,
                has_attachments: false,
                body: "<p>Please join the budget review.</p>",
                meeting: {
                    kind: "invitation",
                    start: "2026-10-14T04:00:00Z",
                    end: "2026-10-14T05:00:00Z",
                    is_all_day: false,
                    location: "Harbour Room",
                    event_id: "e2",
                    response: "notResponded",
                    conflicts: [{ subject: "Supplier call", start: "2026-10-14T04:30:00Z", end: "2026-10-14T05:30:00Z" }],
                    complete: true,
                },
            },
            ...inbox,
        ],
    });
    await page.getByRole("listbox", { name: "Messages" }).getByRole("option").first().click();
    const card = page.getByRole("region", { name: "Meeting" });

    await expect(card).toContainText("Meeting invitation");
    // Two in the afternoon on the Wednesday, in Brisbane.
    await expect(card).toContainText(/Wed.*2:00\s?PM to 3:00\s?PM/i);
    await expect(card).toContainText("Harbour Room");
    await expect(card).toContainText("Clashes with Supplier call");
    if (process.env.SCREENSHOTS) await page.screenshot({ path: "../docs/screenshots/invitation.png" });

    await card.getByRole("button", { name: "Show in calendar" }).click();
    const calendar = page.getByRole("region", { name: "Calendar" });
    await expect(calendar.getByRole("heading", { level: 1 })).toHaveText(/12 Oct – 16 Oct 2026|Oct 12 – Oct 16 2026/);
    await expect(calendar.getByRole("button", { name: /Budget review/ })).toBeVisible();
});

test("a cancellation and an answer each say what they are", async ({ page }) => {
    const about = (id: string, subject: string, from: string, meeting: Record<string, unknown>) => ({
        id,
        folder_id: "inbox",
        subject,
        preview: "",
        from: { name: from, address: "someone@example.test" },
        received_at: `2026-10-09T0${id.length}:00:00Z`,
        is_read: true,
        has_attachments: false,
        body: "<p></p>",
        meeting: { start: "2026-10-14T04:00:00Z", end: "2026-10-14T05:00:00Z", conflicts: [], complete: true, ...meeting },
    });
    await withCore(page, {
        accounts: [robin],
        mail: [about("a", "Cancelled: Budget review", "Sam Taylor", { kind: "cancellation" }), about("bb", "Accepted: Team lunch", "Jo Nguyen", { kind: "accepted" })],
    });
    const rows = page.getByRole("listbox", { name: "Messages" }).getByRole("option");
    const card = page.getByRole("region", { name: "Meeting" });

    await rows.filter({ hasText: "Cancelled" }).click();
    await expect(card).toContainText("This meeting was cancelled");
    await rows.filter({ hasText: "Accepted" }).click();
    await expect(card).toContainText("Jo Nguyen: accepted");
    // An answer is not an invitation: there is nothing to say about clashes.
    await expect(card).not.toContainText("calendar then");
});

const invitation = {
    id: "inv",
    folder_id: "inbox",
    subject: "Budget review",
    preview: "Please join the budget review.",
    from: { name: "Sam Taylor", address: "sam.taylor@example.test" },
    received_at: "2026-10-09T05:00:00Z",
    is_read: false,
    has_attachments: false,
    body: "<p>Please join the budget review.</p>",
    meeting: {
        kind: "invitation",
        start: "2026-10-14T04:00:00Z",
        end: "2026-10-14T05:00:00Z",
        is_all_day: false,
        location: "Harbour Room",
        event_id: "e2",
        response: "notResponded",
        conflicts: [],
        complete: true,
    },
};
const unanswered = week.map((each) => (each.id === "e2" ? { ...each, response: "notResponded" } : each));

test("an invitation is answered from the message, with a note to the organiser", async ({ page }) => {
    await page.clock.setFixedTime(new Date("2026-10-09T06:00:00Z"));
    await withCore(page, { accounts: [robin], events: unanswered, mail: [invitation, ...inbox] });
    await page.getByRole("listbox", { name: "Messages" }).getByRole("option").first().click();
    const card = page.getByRole("region", { name: "Meeting" });
    const answers = card.getByRole("group", { name: "Answer the invitation" });

    await answers.getByText("Options").click();
    await answers.getByRole("textbox", { name: "Note to the organiser" }).fill("I may be five minutes late");
    await answers.getByRole("button", { name: "Accept" }).click();

    await expect(card).toContainText("You accepted");
    await expect(answers.getByRole("button", { name: "Accept" })).toHaveAttribute("aria-pressed", "true");
    expect((await ran(page)).filter((r) => r.command === "calendar.events.respond").at(-1)?.args).toMatchObject({
        event_id: "e2",
        response: "accept",
        comment: "I may be five minutes late",
        send_response: true,
    });
    if (process.env.SCREENSHOTS) await page.screenshot({ path: "../docs/screenshots/invitation.png" });

    // Declined, it has left the calendar, and there is nothing more to answer.
    await answers.getByRole("button", { name: "Decline" }).click();
    await expect(card).toContainText("You declined");
    await expect(answers).toHaveCount(0);
});

test("an answer that cannot be sent says so and changes nothing", async ({ page }) => {
    await withCore(page, { accounts: [robin], events: unanswered, mail: [invitation], offline: true });
    await page.getByRole("listbox", { name: "Messages" }).getByRole("option").first().click();
    const card = page.getByRole("region", { name: "Meeting" });

    await card.getByRole("button", { name: "Accept" }).click();
    await expect(card.getByRole("alert")).toContainText("could not be reached");
    await expect(card).not.toContainText("You accepted");
    await expect(card.getByRole("button", { name: "Accept" })).toBeEnabled();
});

test("an invitation is answered from the calendar, quietly, and a declined one goes", async ({ page }) => {
    await page.clock.setFixedTime(new Date("2026-10-14T00:00:00Z"));
    await withCore(page, { accounts: [robin], events: unanswered });
    await page.getByRole("button", { name: "Calendar" }).click();
    const calendar = page.getByRole("region", { name: "Calendar" });
    await calendar.getByRole("button", { name: /Budget review/ }).click();
    const panel = page.getByRole("complementary", { name: "Event" });
    const answers = panel.getByRole("group", { name: "Answer the invitation" });

    await answers.getByText("Options").click();
    await answers.getByRole("checkbox", { name: "Tell the organiser" }).uncheck();
    await expect(answers.getByRole("textbox", { name: "Note to the organiser" })).toBeDisabled();
    await answers.getByRole("button", { name: "Tentative" }).click();
    await expect(answers.getByRole("button", { name: "Tentative" })).toHaveAttribute("aria-pressed", "true");
    expect((await ran(page)).filter((r) => r.command === "calendar.events.respond").at(-1)?.args).toMatchObject({ response: "tentative", send_response: false });

    await answers.getByRole("button", { name: "Decline" }).click();
    await expect(panel).toHaveCount(0);
    await expect(calendar.getByRole("button", { name: /Budget review/ })).toHaveCount(0);
});

test("an event is made, changed and deleted from the calendar", async ({ page }) => {
    await page.clock.setFixedTime(new Date("2026-10-14T00:00:00Z"));
    await withCore(page, { accounts: [robin], events: week });
    await page.getByRole("button", { name: "Calendar" }).click();
    const calendar = page.getByRole("region", { name: "Calendar" });
    const panel = page.getByRole("complementary", { name: "Event" });

    // Made: by the clock here, with the zone it was made in.
    await calendar.getByRole("button", { name: "New event" }).click();
    const form = panel.getByRole("form", { name: "New event" });
    await form.getByRole("textbox", { name: "Subject" }).fill("Dentist");
    await form.getByLabel("Start day").fill("2026-10-15");
    await form.getByLabel("Start time").fill("14:00");
    // The end keeps half an hour behind the start.
    await expect(form.getByLabel("End day")).toHaveValue("2026-10-15");
    await expect(form.getByLabel("End time")).toHaveValue("14:30");
    await form.getByLabel("End time").fill("15:00");
    await form.getByRole("textbox", { name: "Location" }).fill("Level 2");
    await form.getByLabel("Reminder").selectOption("60");
    await form.getByLabel("Private").check();
    if (process.env.SCREENSHOTS) await page.screenshot({ path: "../docs/screenshots/calendar-new-event.png" });
    await form.getByRole("button", { name: "Save" }).click();

    expect((await ran(page)).filter((r) => r.command === "calendar.events.create").at(-1)?.args).toMatchObject({
        calendar_id: "main",
        subject: "Dentist",
        start: "2026-10-15T14:00",
        end: "2026-10-15T15:00",
        time_zone: "Australia/Brisbane",
        is_all_day: false,
        reminder_minutes: 60,
        is_private: true,
        location: "Level 2",
    });
    await expect(calendar.getByRole("button", { name: /Dentist/ })).toBeVisible();
    await expect(panel).toContainText("Thu, Oct 15, 2026, 2:00 PM to 3:00 PM");

    // Changed: only what was changed is sent.
    await panel.getByRole("button", { name: "Change" }).click();
    const change = panel.getByRole("form", { name: "Change event" });
    await expect(change.getByRole("textbox", { name: "Subject" })).toHaveValue("Dentist");
    await change.getByRole("textbox", { name: "Subject" }).fill("Dentist (check-up)");
    await change.getByRole("button", { name: "Save" }).click();
    const sent = (await ran(page)).filter((r) => r.command === "calendar.events.update").at(-1)?.args as Record<string, unknown>;
    expect(sent).toMatchObject({ subject: "Dentist (check-up)" });
    expect(Object.keys(sent).sort()).toEqual(["account_id", "event_id", "subject"]);
    await expect(panel.getByRole("heading", { level: 2 })).toHaveText("Dentist (check-up)");

    // Made all day: its last day is the day it ends, to a person, and the day after to the server.
    await panel.getByRole("button", { name: "Change" }).click();
    await change.getByLabel("All day").check();
    await change.getByLabel("End day").fill("2026-10-16");
    await change.getByRole("button", { name: "Save" }).click();
    expect((await ran(page)).filter((r) => r.command === "calendar.events.update").at(-1)?.args).toMatchObject({ is_all_day: true, start: "2026-10-15", end: "2026-10-17" });
    await expect(panel).toContainText("Thu, Oct 15, 2026 to Fri, Oct 16, 2026, all day");

    // Deleted, once it has been asked.
    await panel.getByRole("button", { name: "Delete" }).click();
    await panel.getByRole("button", { name: "Keep" }).click();
    await panel.getByRole("button", { name: "Delete" }).click();
    await panel.getByRole("button", { name: "Delete for good?" }).click();
    await expect(panel).toHaveCount(0);
    await expect(calendar.getByText("Dentist")).toHaveCount(0);
});

test("an event that cannot be saved says so and the form is kept", async ({ page }) => {
    await page.clock.setFixedTime(new Date("2026-10-14T00:00:00Z"));
    await withCore(page, { accounts: [robin], events: week, offline: true });
    await page.getByRole("button", { name: "Calendar" }).click();
    await page.keyboard.press("n");
    const form = page.getByRole("form", { name: "New event" });
    await form.getByRole("textbox", { name: "Subject" }).fill("Dentist");
    await form.getByRole("button", { name: "Save" }).click();

    await expect(form.getByRole("alert")).toContainText("could not be reached");
    await expect(form.getByRole("textbox", { name: "Subject" })).toHaveValue("Dentist");
    await page.keyboard.press("Escape");
    await expect(form).toHaveCount(0);
});
