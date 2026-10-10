import { invoke } from "@tauri-apps/api/core";

/** Why a command did not run: a fixed code, and words for a person. */
export interface CommandFailure {
    code: string;
    message: string;
}

/**
 * Run one of the client's commands by its id.
 *
 * This is the window's only way to do anything. The same commands are what
 * the CLI and agents run, so the window can do nothing they cannot.
 */
export function execute<T>(command: string, args: Record<string, unknown> = {}): Promise<T> {
    return invoke<T>("execute_command", { command, args });
}

export function isCommandFailure(value: unknown): value is CommandFailure {
    return typeof value === "object" && value !== null && "code" in value && "message" in value;
}
