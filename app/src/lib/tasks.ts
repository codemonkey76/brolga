import { execute } from "./commands";

/** A Microsoft To Do list, as the core gives it out. */
export interface TaskList {
    id: string;
    name: string;
    well_known: "none" | "defaultList" | "flaggedEmails";
    built_in: boolean;
    /** Flagged email: the mail's own flags, changed by flagging mail. */
    read_only: boolean;
}

export interface Task {
    id: string;
    list_id: string;
    title: string;
    notes: string;
    /** `2026-10-20`. */
    due: string | null;
    /** In UTC. */
    reminder_at: string | null;
    importance: "low" | "normal" | "high";
    status: string;
    done: boolean;
    links: { application?: string; name?: string; external_id?: string; url?: string | null; message_id?: string }[];
    /** False until a task made here has reached To Do. */
    on_server: boolean;
}

export interface Lists {
    lists: TaskList[];
    needs_sign_in: boolean;
    changes_waiting: number;
    changes_refused: { change: string; id: string; error: string }[];
}

export const listTaskLists = (account_id: string) => execute<Lists>("tasks.lists.list", { account_id });

export const listTasks = async (account_id: string, list_id: string, include_done: boolean) =>
    (await execute<{ tasks: Task[] }>("tasks.list", { account_id, list_id, include_done })).tasks;

export const createTask = (account_id: string, list_id: string, title: string) =>
    execute<{ task: Task; undo_id: string }>("tasks.create", { account_id, list_id, title });

export const updateTask = (account_id: string, task_id: string, changes: Record<string, unknown>) =>
    execute<{ task: Task; undo_id: string }>("tasks.update", { account_id, task_id, ...changes });

export const completeTask = (account_id: string, task_id: string, done: boolean) =>
    execute<{ task: Task; undo_id: string }>("tasks.complete", { account_id, task_id, done });

export const deleteTask = (account_id: string, task_id: string) =>
    execute<{ deleted: boolean; undo_id: string }>("tasks.delete", { account_id, task_id });

export const undoTasks = (account_id: string, undo_id: string) => execute<{ undone: number }>("tasks.undo", { account_id, undo_id });

export const createTaskList = (account_id: string, name: string) =>
    execute<{ list: TaskList; undo_id: string }>("tasks.lists.create", { account_id, name });

export const renameTaskList = (account_id: string, list_id: string, name: string) =>
    execute<{ list: TaskList; undo_id: string }>("tasks.lists.rename", { account_id, list_id, name });

export const deleteTaskList = (account_id: string, list_id: string) => execute<{ deleted: boolean }>("tasks.lists.delete", { account_id, list_id });

/** The email a task was made from, where Brolga made it or one links to it. */
export const linkedMessage = (task: Task) => task.links.find((link) => link.application === "Brolga" && (link.message_id || link.external_id));
