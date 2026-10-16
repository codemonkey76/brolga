import { expect, test } from "@playwright/test";
import { inbox, ran, robin, withCore } from "./core";

test("tasks are listed due first, added, changed, done, deleted and undone", async ({ page }) => {
    await withCore(page, {
        accounts: [robin],
        mail: inbox,
        taskLists: [{ id: "list-home", name: "Home" }],
        tasks: [
            { id: "t-1", title: "Pay the Telstra bill", due: "2026-10-20", importance: "high", links: [{ application: "Brolga", name: "Your Telstra bill", external_id: "m1", message_id: "m1" }] },
            { id: "t-2", title: "Book flights" },
        ],
    });
    await page.getByRole("button", { name: "Tasks" }).click();
    const lists = page.getByRole("complementary", { name: "Task lists" });
    await expect(lists.getByRole("button")).toContainText(["Tasks", "Flagged email", "Home"]);
    const tasks = page.getByRole("list", { name: "Tasks" });
    await expect(tasks.getByRole("listitem")).toHaveCount(2);
    await expect(tasks.getByRole("listitem").nth(0)).toContainText("from an email");

    await page.getByRole("textbox", { name: "Add a task" }).fill("Call the plumber");
    await page.getByRole("textbox", { name: "Add a task" }).press("Enter");
    await expect(tasks.getByRole("listitem")).toHaveCount(3);
    expect((await ran(page)).find((r) => r.command === "tasks.create")?.args).toMatchObject({ title: "Call the plumber", list_id: "list-tasks" });

    // Changed in the task's own pane.
    await tasks.getByRole("button", { name: /Book flights/ }).click();
    const pane = page.getByRole("region", { name: "The task" });
    await pane.getByLabel("Due").fill("2026-10-25");
    await pane.getByLabel("Due").blur();
    await expect.poll(async () => (await ran(page)).find((r) => r.command === "tasks.update")?.args).toMatchObject({ task_id: "t-2", due: "2026-10-25" });

    // Done, and undone.
    await tasks.getByRole("checkbox", { name: "Book flights is done" }).click();
    await expect(tasks.getByRole("listitem")).toHaveCount(2);
    await page.getByRole("button", { name: "Undo" }).click();
    await expect(tasks.getByRole("listitem")).toHaveCount(3);

    // The email it was made from opens in Mail.
    await tasks.getByRole("button", { name: /Pay the Telstra bill/ }).click();
    await pane.getByRole("button", { name: "Open the email: Your Telstra bill" }).click();
    await expect(page.getByRole("navigation", { name: "Sections" }).getByRole("button", { name: "Mail", exact: true })).toHaveAttribute("aria-current", "true");
    await expect.poll(async () => (await ran(page)).filter((r) => r.command === "mail.messages.get").at(-1)?.args).toMatchObject({ message_id: "m1" });
    await expect(page.getByRole("heading", { name: "Budget review moved to Thursday" })).toBeVisible();

    // Flagged email is the mail's own flags, and is only shown.
    await page.getByRole("navigation", { name: "Sections" }).getByRole("button", { name: "Tasks" }).click();
    await lists.getByRole("button", { name: "Flagged email" }).click();
    await expect(page.getByText("These are your flagged messages.")).toBeVisible();
    await expect(page.getByRole("textbox", { name: "Add a task" })).toHaveCount(0);
});

test("an account signed in before Tasks is offered signing in again", async ({ page }) => {
    await withCore(page, { accounts: [robin], mail: inbox, tasksNeedSignIn: true, signIn: robin });
    await page.getByRole("button", { name: "Tasks" }).click();
    await expect(page.getByRole("alert")).toContainText("from before Tasks");
    await page.getByRole("button", { name: "Sign in again to use Tasks" }).click();
    await expect.poll(async () => (await ran(page)).some((r) => r.command === "accounts.sign_in")).toBe(true);
});
