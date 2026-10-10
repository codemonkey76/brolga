import { svelte } from "@sveltejs/vite-plugin-svelte";
import { defineConfig } from "vite";

// Tauri loads the dev server on a fixed port, so a busy one is an error
// rather than a quiet move to the next.
export default defineConfig({
    plugins: [svelte()],
    clearScreen: false,
    server: { port: 1420, strictPort: true, watch: { ignored: ["**/src-tauri/**"] } },
    build: { target: "es2022" },
});
