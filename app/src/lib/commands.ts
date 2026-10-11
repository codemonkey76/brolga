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

/**
 * Ask where to save a file, with the system's own dialog. Null if it was
 * dismissed. A dialog is the window's own business, so this is the one thing
 * here that is not a command; the saving that follows is.
 */
export const chooseSavePath = (suggestedName: string) => invoke<string | null>("choose_save_path", { suggestedName });

export function isCommandFailure(value: unknown): value is CommandFailure {
    return typeof value === "object" && value !== null && "code" in value && "message" in value;
}

/** Why something failed, in words to show. */
export const say = (failure: unknown) => (isCommandFailure(failure) ? failure.message : String(failure));
