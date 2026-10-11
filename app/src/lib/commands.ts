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

/** Ask which files to attach, with the system's own dialog. Empty if it was dismissed. */
export const chooseFiles = () => invoke<string[]>("choose_files");

/** Show a notification on the desktop. If the desktop will not, nothing is lost: the window shows it too. */
export const notify = (title: string, body: string) => invoke<void>("notify", { title, body }).catch(() => {});

/** A message a `mailto:` link asked to be started: who to, the subject and the words, as plain text. */
export interface Asked {
    to: { address: string; name: string | null }[];
    cc: { address: string; name: string | null }[];
    bcc: { address: string; name: string | null }[];
    subject: string;
    body: string;
}

/** The next message the window is asked to start. It answers when there is one, however long that is. */
export const nextRequest = () => invoke<Asked | null>("next_request");

export function isCommandFailure(value: unknown): value is CommandFailure {
    return typeof value === "object" && value !== null && "code" in value && "message" in value;
}

/** Why something failed, in words to show. */
export const say = (failure: unknown) => (isCommandFailure(failure) ? failure.message : String(failure));
