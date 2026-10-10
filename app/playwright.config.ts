import { defineConfig } from "@playwright/test";

// The window's frontend, built and served on its own, with a stand-in for the
// Rust core behind it. Nothing here needs a desktop, an account or a network.
export default defineConfig({
    testDir: "tests",
    fullyParallel: true,
    reporter: "list",
    use: { baseURL: "http://localhost:4173", viewport: { width: 1000, height: 700 } },
    webServer: {
        command: "bun run build && bunx vite preview --port 4173 --strictPort",
        url: "http://localhost:4173",
        reuseExistingServer: !process.env.CI,
    },
});
