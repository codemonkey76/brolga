import { expect, test } from "@playwright/test";
import { inbox, ran, robin, withCore } from "./core";

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
