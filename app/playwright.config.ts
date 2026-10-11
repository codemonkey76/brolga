import { defineConfig, devices } from "@playwright/test";

// The window's frontend, built and served on its own, with a stand-in for the
// Rust core behind it. Nothing here needs a desktop, an account or a network.
export default defineConfig({
    testDir: "tests",
    fullyParallel: true,
    reporter: "list",
    use: { baseURL: "http://localhost:4173", viewport: { width: 1280, height: 760 } },
    // The app's window is WebKit on Linux and macOS and Chromium on Windows.
    // WebKit is run in CI, where Playwright has a build of it for the system.
    projects: [
        { name: "chromium", use: { ...devices["Desktop Chrome"], viewport: { width: 1280, height: 760 } } },
        ...(process.env.CI ? [{ name: "webkit", use: { ...devices["Desktop Safari"], viewport: { width: 1280, height: 760 } } }] : []),
    ],
    webServer: {
        command: "bun run build && bunx vite preview --port 4173 --strictPort",
        url: "http://localhost:4173",
        reuseExistingServer: !process.env.CI,
    },
});
