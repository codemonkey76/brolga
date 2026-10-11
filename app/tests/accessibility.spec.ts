import AxeBuilder from "@axe-core/playwright";
import { type Page, expect, test } from "@playwright/test";
import { inbox, robin, week, withCore } from "./core";

// Each screen is read the way a screen reader's rules would read it: every
// control named, every picture described or hidden, words that can be told
// from what is behind them. A rule broken anywhere fails the screen.
const faults = async (page: Page) => {
    const found = await new AxeBuilder({ page })
        .withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa"])
        // A message's own HTML is the sender's. It is shown as it came.
        .exclude('iframe[title="Message body"]')
        .analyze();

    return found.violations.map((v) => `${v.id}: ${v.help} — ${v.nodes.map((n) => n.target.join(" ")).slice(0, 4).join(" | ")}`);
};

const screens: Record<string, (page: Page) => Promise<void>> = {
    "mail, with a message open": async (page) => {
        await page.getByRole("listbox", { name: "Messages" }).getByRole("option").first().click();
        await expect(page.getByRole("region", { name: "Message" }).getByRole("heading")).toBeVisible();
    },
    "writing a message": async (page) => {
        await page.getByRole("button", { name: "New message" }).click();
        await expect(page.getByRole("textbox", { name: "Subject" })).toBeVisible();
    },
    "the calendar": async (page) => {
        await page.getByRole("button", { name: "Calendar" }).click();
        await expect(page.getByRole("button", { name: "New event" })).toBeVisible();
    },
    "writing an event": async (page) => {
        await page.getByRole("button", { name: "Calendar" }).click();
        await page.getByRole("button", { name: "New event" }).click();
        await expect(page.getByRole("textbox", { name: "Subject" })).toBeVisible();
    },
    people: async (page) => {
        await page.getByRole("button", { name: "People" }).click();
        await expect(page.getByRole("searchbox").first()).toBeVisible();
    },
    accounts: async (page) => {
        await page.getByRole("button", { name: "Accounts" }).click();
        await expect(page.getByRole("list", { name: "Accounts" })).toBeVisible();
    },
    "an account's rules, categories and calendar sharing": async (page) => {
        await page.getByRole("button", { name: "Accounts" }).click();
        for (const each of ["Rules", "Categories", "Calendar sharing"]) await page.getByRole("button", { name: each, exact: true }).first().click();
        await expect(page.getByRole("list", { name: "Accounts" })).toBeVisible();
    },
    "the palette": async (page) => {
        await page.keyboard.press("Control+k");
        await expect(page.getByRole("dialog", { name: "Commands" })).toBeVisible();
    },
    agents: async (page) => {
        await page.getByRole("button", { name: "Agents" }).click();
    },
};

for (const scheme of ["light", "dark"] as const) {
    for (const [name, open] of Object.entries(screens)) {
        test(`${name} can be read by a screen reader and seen, in ${scheme}`, async ({ page }) => {
            await page.emulateMedia({ colorScheme: scheme });
            await withCore(page, { accounts: [robin], mail: inbox, events: week });
            await open(page);

            expect(await faults(page)).toEqual([]);
        });
    }
}
