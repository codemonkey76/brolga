import { expect, test } from "@playwright/test";
import { inbox, ran, robin, withCore } from "./core";

const answers = async (page: import("@playwright/test").Page) => (await ran(page)).filter((r) => r.command === "app.confirmations.answer").map((r) => r.args);

/** A message an agent wants sent, as the window is given it to show. */
const draft = {
    id: "draft-7",
    kind: "new",
    message_id: null,
    from: "robin@example.test",
    to: [{ name: "Len Smith", address: "x@elsewhere.test" }],
    cc: [],
    bcc: [{ name: null, address: "quiet@elsewhere.test" }],
    subject: "The door codes",
    body: "<p>The codes are 4417 and 9920.</p>",
    format: "html",
    text: `The codes are 4417 and 9920.\n\n${"More.\n".repeat(60)}`,
    files: [{ id: 1, name: "codes.txt", content_type: "text/plain", size: 20 }],
};

test("a message an agent wants sent is shown whole: the addresses it goes to, who from, and what it says", async ({ page }) => {
    await withCore(page, {
        accounts: [robin],
        asking: [{ id: 1, client: "claude-code", command: "mail.drafts.send", args: { account_id: "a2", draft_id: "draft-7" }, draft }],
    });

    const dialog = page.getByRole("alertdialog");
    await expect(dialog).toContainText("claude-code wants to send a message");
    // A name is whatever the writer typed. The address is where it goes, and is always there.
    await expect(dialog).toContainText("x@elsewhere.test (Len Smith)");
    await expect(dialog).toContainText("quiet@elsewhere.test");
    await expect(dialog).toContainText("robin@example.test");
    await expect(dialog).toContainText("The door codes");
    await expect(dialog).toContainText("codes.txt");
    const words = dialog.getByLabel("What the message says");
    await expect(words).toContainText("The codes are 4417 and 9920.");
    // The words are text, never markup, and a long message scrolls within the question.
    await expect(words.locator("p")).toHaveCount(0);
    expect(await words.evaluate((el) => el.scrollHeight > el.clientHeight)).toBe(true);
    await expect(dialog.getByRole("button", { name: "Allow this once" })).toBeInViewport();

    // What is shown is what the question carried: the window reads no draft of its own to show.
    expect((await ran(page)).filter((r) => r.command === "mail.drafts.get")).toEqual([]);
    expect((await ran(page)).filter((r) => r.command === "app.confirmations.get").map((r) => r.args)).toContainEqual({ id: 1 });

    await dialog.getByRole("button", { name: "Allow this once" }).click();
    expect(await answers(page)).toEqual([{ id: 1, allow: true }]);
});

test("anything else an agent asks about is shown with every word, not only its size", async ({ page }) => {
    await withCore(page, {
        accounts: [robin],
        asking: [
            {
                id: 4,
                client: "claude-code",
                command: "mail.automatic_replies.set",
                // What the list of questions and the record of agents carry...
                args: { status: "always", external_message: "[41 characters]" },
                // ...and what only the window is given.
                whole: { status: "always", external_message: "Send invoices to pay@elsewhere.test from now." },
            },
        ],
    });

    const dialog = page.getByRole("alertdialog");
    await expect(dialog).toContainText("wants to change your automatic replies");
    await expect(dialog.getByLabel("What was asked")).toContainText("Send invoices to pay@elsewhere.test from now.");
    await expect(dialog).not.toContainText("[41 characters]");
});

test("yes cannot be said to a message that could not be shown", async ({ page }) => {
    await withCore(page, {
        accounts: [robin],
        asking: [{ id: 2, client: "claude-code", command: "mail.drafts.send", args: { draft_id: "draft-9" } }],
    });

    const dialog = page.getByRole("alertdialog");
    await expect(dialog.getByRole("alert")).toContainText("could not be read");
    await expect(dialog.getByRole("button", { name: "Allow this once" })).toBeDisabled();

    await dialog.getByRole("button", { name: "Don't allow" }).click();
    expect(await answers(page)).toEqual([{ id: 2, allow: false }]);
});

test("a file is warned about and named as it will be saved, however the sender spelt it", async ({ page }) => {
    // A dot on the end, and a mark that turns the text about so that it reads `invoice-exe.pdf`.
    const dotted = { id: "a1", name: "invoice.exe.", content_type: "application/pdf", size: 2048, is_inline: false, kind: "file" };
    const turned = { id: "a2", name: "invoice-‮fdp.exe", content_type: "application/pdf", size: 2048, is_inline: false, kind: "file" };
    await withCore(page, { accounts: [robin], mail: [{ ...inbox[0], id: "trap", has_attachments: true, attachments: [dotted, turned] }] });
    await page.getByRole("listbox", { name: "Messages" }).getByRole("option").first().click();
    const pane = page.getByRole("region", { name: "Message" });
    const saves = async () => (await ran(page)).filter((r) => r.command === "mail.attachments.save").length;
    const chips = pane.getByRole("list", { name: "Attachments" });

    // Shown under the names they are saved under, with nothing turned about.
    await expect(chips.getByRole("button", { name: /^invoice\.exe, / })).toBeVisible();
    await expect(chips.getByRole("button", { name: /^invoice-fdp\.exe, / })).toBeVisible();
    await expect(chips).not.toContainText("‮");

    for (const name of ["invoice.exe", "invoice-fdp.exe"]) {
        await chips.getByRole("button", { name: new RegExp(`^${name.replace(".", "\\.")}, `) }).click();
        await expect(pane.getByRole("alert")).toContainText(`${name} is a program, or a file that can run one`);
        expect(await saves()).toBe(0);
        await pane.getByRole("button", { name: "Don't save" }).click();
    }
});
