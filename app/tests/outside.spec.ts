import { expect, test } from "@playwright/test";
import { ran, robin, withCore } from "./core";

test("an outside tool server is added, signed in to with the browser or a token, tried and removed", async ({ page }) => {
    await withCore(page, { accounts: [robin] });
    await page.getByRole("button", { name: "Settings" }).click();
    await page.getByRole("button", { name: "Outside tools" }).click();
    const section = page.getByRole("region", { name: "Outside tool servers" });

    await section.getByRole("button", { name: "Add a server" }).click();
    await section.getByLabel("Name").fill("Helpdesk");
    await section.getByLabel("Address of its MCP server").fill("https://helpdesk.example.com/mcp/helpdesk");
    await section.getByRole("button", { name: "Add", exact: true }).click();
    const server = section.getByRole("list", { name: "Servers" }).getByRole("listitem").first();
    await expect(server).toContainText("Not signed in");

    await server.getByRole("button", { name: "Sign in through the browser" }).click();
    await expect(server).toContainText("Signed in through the browser · 2 tools");
    await server.getByRole("button", { name: "Try it" }).click();
    await expect(server).toContainText("It works: 2 tools");
    await server.getByRole("button", { name: "Its tools" }).click();
    await expect(section.getByRole("list", { name: "Tools of Helpdesk" })).toContainText("Create a ticket");

    await server.getByRole("button", { name: "Use an access token" }).click();
    await section.getByRole("textbox", { name: "Access token" }).fill("pat-123");
    await section.getByRole("button", { name: "Save" }).click();
    await expect(server).toContainText("Signed in with an access token");
    expect((await ran(page)).find((r) => r.command === "mcp_servers.set_token")?.args).toMatchObject({ token: "pat-123" });

    await server.getByRole("button", { name: "Remove" }).click();
    await server.getByRole("button", { name: "Remove it?" }).click();
    await expect(section).toContainText("No servers yet.");
});
