import { type Page, expect, test } from "@playwright/test";
import { notified, ran, robin, week, withCore } from "./core";

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
