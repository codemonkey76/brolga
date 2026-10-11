import { expect, test } from "@playwright/test";
import { ran, robin, withCore } from "./core";

const person = (id: string, given: string, surname: string, more: Record<string, unknown> = {}) => ({
    id,
    folder_id: "",
    name: `${given} ${surname}`,
    given_name: given,
    surname,
    nickname: "",
    company: "",
    job_title: "",
    department: "",
    office: "",
    emails: [{ address: `${given.toLowerCase()}@example.test`, name: "" }],
    business_phones: [],
    home_phones: [],
    mobile: "",
    notes: "",
    birthday: null,
    ...more,
});
const contacts = [
    person("c1", "Sam", "Taylor", { company: "Northwind", job_title: "Buyer", mobile: "0400 000 001", notes: "Met at the trade show.", birthday: "1980-05-17" }),
    person("c2", "Jo", "Nguyen", { folder_id: "suppliers", company: "Harbour Freight" }),
    person("c3", "Alex", "Chen"),
];
const directory = [{ name: "Samira Okafor", address: "samira.okafor@example.test", job_title: "Engineer", department: "Platform" }];

test("contacts are listed, shown, searched with the organisation, and kept by folder", async ({ page }) => {
    await withCore(page, { accounts: [robin], contacts, directory });
    await page.getByRole("button", { name: "People" }).click();
    const list = page.getByRole("list", { name: "Contacts" });
    const card = page.getByRole("region", { name: "Contact" });

    await expect(list.getByRole("button")).toHaveText([/Alex Chen/, /Jo Nguyen.*Harbour Freight/, /Sam Taylor.*Buyer · Northwind/]);
    await list.getByRole("button", { name: /Sam Taylor/ }).click();
    await expect(card.getByRole("heading", { level: 2 })).toHaveText("Sam Taylor");
    await expect(card).toContainText("sam@example.test");
    await expect(card).toContainText("0400 000 001");
    await expect(card).toContainText("May 17, 1980");
    await expect(card).toContainText("Met at the trade show.");
    if (process.env.SCREENSHOTS) await page.screenshot({ path: "../docs/screenshots/people.png" });

    // A folder holds only its own.
    await page.getByRole("complementary", { name: "Folders of contacts" }).getByRole("button", { name: "Suppliers" }).click();
    await expect(list.getByRole("button")).toHaveText([/Jo Nguyen/]);
    await page.getByRole("complementary", { name: "Folders of contacts" }).getByRole("button", { name: "All contacts" }).click();

    // Searching looks in the organisation too, and someone found there can be kept.
    await page.getByRole("searchbox", { name: "Search people" }).fill("sam");
    await expect(list.getByRole("button")).toHaveText([/Sam Taylor/]);
    const organisation = page.getByRole("list", { name: "In your organisation" });
    await expect(organisation).toContainText("Samira Okafor");
    await expect(organisation).toContainText("Engineer · Platform");
    await organisation.getByRole("button", { name: "Add Samira Okafor to contacts" }).click();
    const form = card.getByRole("form", { name: "New contact" });
    await expect(form.getByLabel("First name")).toHaveValue("Samira");
    await expect(form.getByLabel("Email addresses")).toHaveValue("samira.okafor@example.test");
    await form.getByRole("button", { name: "Save" }).click();
    expect((await ran(page)).filter((r) => r.command === "contacts.create").at(-1)?.args).toMatchObject({
        given_name: "Samira",
        surname: "Okafor",
        job_title: "Engineer",
        emails: ["samira.okafor@example.test"],
    });
    await expect(card.getByRole("heading", { level: 2 })).toHaveText("Samira Okafor");
});

test("a contact is made, changed and deleted", async ({ page }) => {
    await withCore(page, { accounts: [robin], contacts });
    await page.keyboard.press("Control+3");
    const card = page.getByRole("region", { name: "Contact" });
    const list = page.getByRole("list", { name: "Contacts" });

    await page.getByRole("button", { name: "New contact" }).click();
    const form = card.getByRole("form", { name: "New contact" });
    await form.getByLabel("First name").fill("Kim");
    await form.getByLabel("Last name").fill("Lee");
    await form.getByLabel("Email addresses").fill("kim@example.test\nkim@home.test");
    await form.getByLabel("Mobile").fill("0400 000 009");
    await form.getByRole("button", { name: "Save" }).click();
    expect((await ran(page)).filter((r) => r.command === "contacts.create").at(-1)?.args).toMatchObject({
        given_name: "Kim",
        surname: "Lee",
        emails: ["kim@example.test", "kim@home.test"],
        mobile: "0400 000 009",
    });
    await expect(list.getByRole("button", { name: /Kim Lee/ })).toBeVisible();

    // Changed: only what was changed is sent.
    await card.getByRole("button", { name: "Change" }).click();
    const change = card.getByRole("form", { name: "Change contact" });
    await change.getByLabel("Job title").fill("Director");
    await change.getByRole("button", { name: "Save" }).click();
    const sent = (await ran(page)).filter((r) => r.command === "contacts.update").at(-1)?.args as Record<string, unknown>;
    expect(Object.keys(sent).sort()).toEqual(["account_id", "contact_id", "job_title"]);
    await expect(card).toContainText("Director");

    await card.getByRole("button", { name: "Delete" }).click();
    await card.getByRole("button", { name: "Delete for good?" }).click();
    await expect(list.getByRole("button", { name: /Kim Lee/ })).toHaveCount(0);
});

test("with the organisation out of reach, contacts are still found and it says so", async ({ page }) => {
    await withCore(page, { accounts: [robin], contacts, directory, offline: true });
    await page.getByRole("button", { name: "People" }).click();
    await page.getByRole("searchbox", { name: "Search people" }).fill("sam");
    await expect(page.getByRole("list", { name: "Contacts" }).getByRole("button")).toHaveText([/Sam Taylor/]);
    await expect(page.getByText("Your organisation could not be searched just now")).toBeVisible();
});
