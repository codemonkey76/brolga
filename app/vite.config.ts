import { readFileSync } from "node:fs";
import { svelte } from "@sveltejs/vite-plugin-svelte";
import { defineConfig } from "vite";

// The policy the app is built with. The preview server, which the frontend's
// tests load, sends the same one, so the tests run under it.
const csp: string = JSON.parse(readFileSync(new URL("./src-tauri/tauri.conf.json", import.meta.url), "utf8")).app.security.csp;

// Tauri loads the dev server on a fixed port, so a busy one is an error
// rather than a quiet move to the next.
export default defineConfig({
    plugins: [svelte()],
    clearScreen: false,
    server: { port: 1420, strictPort: true, watch: { ignored: ["**/src-tauri/**"] } },
    preview: { headers: { "Content-Security-Policy": csp } },
    build: { target: "es2022" },
});
