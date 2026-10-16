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

const helpdesk = { id: "mcp-1", name: "Helpdesk", url: "https://helpdesk.example.com/mcp/helpdesk", signed_in: "token", tools: 2, tools_read_at: null };
const processor = { device_id: "device-here", device_name: "titan", since: "2026-10-15T08:00:00Z" };

test("an instruction is given a tool with an input fixed, and how it uses the tool is chosen in the window", async ({ page }) => {
    const instructions = [{ id: "i-1", name: "Support", text: "A support request becomes a helpdesk ticket.", enabled: true, actions: ["set_read"] }];
    await withCore(page, { accounts: [robin], instructions, instructionsProcessor: processor, mcpServers: [helpdesk] });
    await page.getByRole("button", { name: "Settings" }).click();
    await page.getByRole("button", { name: "Instructions" }).click();
    const section = page.getByRole("region", { name: "Mail instructions for robin@example.test" });

    await section.getByRole("button", { name: "Change Support" }).click();
    const form = section.getByRole("form", { name: "Change instruction" });
    await form.getByLabel("Call an outside tool it names").check();
    const tools = form.getByRole("group", { name: "Tools it may call" });
    await tools.getByRole("combobox", { name: "Tool to add" }).selectOption({ label: "Helpdesk create_ticket" });
    await tools.getByRole("button", { name: "Add tool" }).click();
    await tools.getByLabel("What create_ticket is always called with").fill("client 7");
    await form.getByRole("button", { name: "Save instruction" }).click();
    await expect(section.getByRole("alert")).toContainText("must be written as a JSON object");
    await tools.getByLabel("What create_ticket is always called with").fill('{"client_id": 7}');
    await form.getByRole("button", { name: "Save instruction" }).click();
    await expect(form).toBeHidden();
    expect((await ran(page)).filter((r) => r.command === "mail.instructions.set").at(-1)?.args).toMatchObject({
        instruction_id: "i-1",
        tools: [{ server_id: "mcp-1", tool: "create_ticket", fixed: { client_id: 7 } }],
    });

    const row = section.getByRole("listitem").filter({ hasText: "Support" });
    const use = row.getByRole("combobox", { name: "How Support uses create_ticket" });
    await expect(use).toHaveValue("ask");
    await use.selectOption("unattended");
    await expect(use).toHaveValue("unattended");
    expect((await ran(page)).filter((r) => r.command === "mail.instructions.tools.set_unattended").at(-1)?.args).toMatchObject({ instruction_id: "i-1", server_id: "mcp-1", tool: "create_ticket", unattended: true, lookup: false });
    await use.selectOption("lookup");
    await expect(use).toHaveValue("lookup");
    expect((await ran(page)).filter((r) => r.command === "mail.instructions.tools.set_unattended").at(-1)?.args).toMatchObject({ unattended: false, lookup: true });
});

test("a call waiting for approval is shown with what it would send, and approved or rejected", async ({ page }) => {
    const call = (id: string, subject: string) => ({
        id, state: "waiting", note: null, decided_by: null, decided_at: null, subject, from: "sam@example.test", message_id: "m-1",
        instruction: "Support", steps: [{ server: "Helpdesk", tool: "create_ticket", inputs: { client_id: 7, subject } }], looked: [], reason: "A support request.", outcome: null,
    });
    const instructions = [{ id: "i-1", name: "Support", text: "A support request becomes a helpdesk ticket.", enabled: true, actions: ["call_tool"] }];
    // On a computer that does not carry out the instructions: approvals are made from any.
    await withCore(page, { accounts: [robin], instructions, instructionsProcessor: { device_id: "device-laptop", device_name: "laptop", since: "2026-10-15T08:00:00Z" }, approvals: [call("a-1", "Printer jammed"), call("a-2", "Free phones")] });
    await page.getByRole("button", { name: "Settings" }).click();
    await page.getByRole("button", { name: "Instructions" }).click();
    const waiting = page.getByRole("region", { name: "Mail instructions for robin@example.test" }).getByLabel("Calls waiting for approval");
    await expect(waiting).toContainText("2 calls wait for your approval");
    const printer = waiting.getByRole("listitem", { name: "Helpdesk create_ticket for Printer jammed" });
    await expect(printer).toContainText("client_id 7");


    await expect(waiting.getByRole("button", { name: "Approve all" })).toBeVisible();
    await printer.getByRole("button", { name: "Approve", exact: true }).click();
    await waiting.getByRole("listitem", { name: "Helpdesk create_ticket for Free phones" }).getByRole("button", { name: "Reject", exact: true }).click();
    await expect(waiting).toBeHidden();
    const decided = (await ran(page)).filter((r) => r.command === "mail.instructions.approvals.decide").map((r) => r.args);
    expect(decided).toMatchObject([{ approval_id: "a-1", approve: true }, { approval_id: "a-2", approve: false }]);
    await page.getByText("Calls decided").click();
    await expect(page.getByRole("list", { name: "Calls decided" })).toContainText("Approved, waiting for laptop");
    await expect(page.getByRole("list", { name: "Calls decided" })).toContainText("Rejected");
});

test("a plan of calls is shown step by step, with what was looked up first and where a step takes an earlier one's value", async ({ page }) => {
    const from = (step: number) => ({ from_step: step, field: "id" });
    const plan = {
        id: "a-1", state: "waiting", note: null, decided_by: null, decided_at: null, subject: "Help", from: "sam@newco.example", message_id: "m-1",
        instruction: "Support", reason: "A support request.", outcome: null,
        steps: [
            { server: "Helpdesk", tool: "create_client", inputs: { name: "NewCo" } },
            { server: "Helpdesk", tool: "create_client_user", inputs: { client_id: from(1), name: "Sam", email: "sam@newco.example" } },
            { server: "Helpdesk", tool: "create_ticket", inputs: { client_id: from(1), user_id: from(2), subject: "Printer jammed" } },
        ],
        looked: [{ server: "Helpdesk", tool: "list_clients", inputs: { contact_email: "sam@newco.example" }, ok: true, result: '{"data":[]}' }],
    };
    const instructions = [{ id: "i-1", name: "Support", text: "Requests become tickets.", enabled: true, actions: ["call_tool"] }];
    await withCore(page, { accounts: [robin], instructions, instructionsProcessor: processor, approvals: [plan] });
    await page.getByRole("button", { name: "Settings" }).click();
    await page.getByRole("button", { name: "Instructions" }).click();
    const item = page.getByRole("listitem", { name: "Helpdesk create_client, then Helpdesk create_client_user, then Helpdesk create_ticket for Help" });
    const steps = item.getByRole("list", { name: "Steps" }).getByRole("listitem");
    await expect(steps).toHaveCount(3);
    await expect(steps.nth(1)).toContainText("client_id what step 1 returns (id)");
    await expect(steps.nth(2)).toContainText("user_id what step 2 returns (id)");
    await item.getByText("Looked up first (1)").click();
    await expect(item.getByRole("list", { name: "Looked up first" })).toContainText('list_clients {"contact_email":"sam@newco.example"}');
    await item.getByRole("button", { name: "Approve", exact: true }).click();
    expect((await ran(page)).find((r) => r.command === "mail.instructions.approvals.decide")?.args).toMatchObject({ approval_id: "a-1", approve: true });
});
