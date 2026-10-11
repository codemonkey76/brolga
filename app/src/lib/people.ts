import { execute } from "./commands";

export interface Contact {
    id: string;
    /** The folder it is in. Empty for the main Contacts folder. */
    folder_id: string;
    name: string;
    given_name: string;
    surname: string;
    nickname: string;
    company: string;
    job_title: string;
    department: string;
    office: string;
    emails: { address: string; name: string }[];
    business_phones: string[];
    home_phones: string[];
    mobile: string;
    notes: string;
    /** `YYYY-MM-DD`. */
    birthday: string | null;
}

/** What a contact is made of, to make one or to change one. What is left out is left as it is. */
export interface ContactFields {
    given_name?: string;
    surname?: string;
    company?: string;
    job_title?: string;
    department?: string;
    emails?: string[];
    business_phones?: string[];
    home_phones?: string[];
    mobile?: string;
    notes?: string;
}

export interface ContactFolder {
    id: string;
    name: string;
    is_default: boolean;
}

/** Someone found by looking them up: a contact of one's own, or a person of the organisation. */
export interface Found {
    name: string;
    address: string;
    job_title: string;
    department: string;
    company: string;
    phones: string[];
    source: "contact" | "directory";
    contact_id?: string;
}

export const listContacts = async (account_id: string, query: string, folder_id: string | null) =>
    (await execute<{ contacts: Contact[] }>("contacts.list", { account_id, ...(query ? { query } : {}), ...(folder_id === null ? {} : { folder_id }) })).contacts;

export const listContactFolders = async (account_id: string) => (await execute<{ folders: ContactFolder[] }>("contacts.folders.list", { account_id })).folders;

export const createContact = async (account_id: string, folder_id: string | null, fields: ContactFields) =>
    (await execute<{ contact: Contact }>("contacts.create", { account_id, ...(folder_id ? { folder_id } : {}), ...fields })).contact;

export const updateContact = async (account_id: string, contact_id: string, fields: ContactFields) =>
    (await execute<{ contact: Contact }>("contacts.update", { account_id, contact_id, ...fields })).contact;

export const deleteContact = (account_id: string, contact_id: string) => execute("contacts.delete", { account_id, contact_id });

export const searchPeople = (account_id: string, query: string) => execute<{ people: Found[]; directory_searched: boolean }>("people.search", { account_id, query });

/** The letters to stand for someone where there is no picture of them. */
export function initials(name: string): string {
    const words = name.trim().split(/\s+/).filter(Boolean);
    return ((words[0]?.[0] ?? "") + (words.length > 1 ? (words[words.length - 1][0] ?? "") : "")).toUpperCase() || "?";
}
