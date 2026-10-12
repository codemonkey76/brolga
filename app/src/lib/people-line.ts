/** One person on a line of people: an address, and a name where one was given. */
export interface Named {
    name: string | null;
    address: string;
}

/**
 * One person as written on a line: `Sam Taylor <sam@example.test>`, or the
 * address alone. A name that holds a comma, a semicolon, an angle bracket or
 * a quote is put in quotes, so that it reads back as one name.
 */
export function personWritten(person: { name?: string | null; address?: string | null }): string {
    const address = person.address ?? "";
    if (!person.name || person.name === address) return address;
    const name = /[,;<>"]/.test(person.name) ? `"${person.name.replace(/(["\\])/g, "\\$1")}"` : person.name;
    return `${name} <${address}>`;
}

/** People as a line to change: `Sam Taylor <sam@example.test>, jo@example.test`. */
export const lineOf = (people: { name?: string | null; address?: string | null }[]) => people.map(personWritten).join(", ");

/**
 * Each stretch of a line that is one person, and where it starts. People are
 * parted by a comma, a semicolon or a new line, but not by one within quotes
 * or within angle brackets.
 */
function pieces(line: string): { text: string; from: number }[] {
    const found: { text: string; from: number }[] = [];
    let from = 0;
    let quoted = false;
    let angled = false;
    for (let at = 0; at < line.length; at++) {
        const char = line[at];
        if (quoted) {
            // A backslash in quotes takes the next character as it is.
            if (char === "\\") at++;
            else if (char === '"') quoted = false;
        } else if (angled) {
            if (char === ">") angled = false;
        } else if (char === '"') quoted = true;
        else if (char === "<") angled = true;
        else if (char === "," || char === ";" || char === "\n") {
            found.push({ text: line.slice(from, at), from });
            from = at + 1;
        }
    }
    found.push({ text: line.slice(from), from });
    return found;
}

/** One stretch of a line as a person. What does not read as `name <address>` is all address, to be checked as one. */
function personFrom(piece: string): Named {
    let name = "";
    let address = "";
    let quoted = false;
    let angled = false;
    let closed = false;
    for (let at = 0; at < piece.length; at++) {
        const char = piece[at];
        if (quoted) {
            if (char === "\\" && at + 1 < piece.length) name += piece[++at];
            else if (char === '"') quoted = false;
            else name += char;
        } else if (angled) {
            if (char === ">") {
                angled = false;
                closed = true;
            } else address += char;
        } else if (closed) {
            // Something after the address: this is not a name and an address.
            if (char.trim()) return { name: null, address: piece };
        } else if (char === '"') quoted = true;
        else if (char === "<") angled = true;
        else name += char;
    }
    return closed ? { name: name.trim() || null, address: address.trim() } : { name: null, address: piece };
}

/** A line of people as typed, as the people on it. */
export const peopleOn = (line: string): Named[] =>
    pieces(line)
        .map((each) => each.text.trim())
        .filter(Boolean)
        .map(personFrom);

/** Where on a line the person being typed now starts: after the last comma or semicolon that parts two people. */
export const lastStarts = (line: string): number => pieces(line).at(-1)?.from ?? 0;

/** Whether this reads as an email address. It is the check the core makes before it keeps a recipient. */
export function isAddress(text: string): boolean {
    const at = text.lastIndexOf("@");
    if (at < 1) return false;
    const domain = text.slice(at + 1);
    return domain.includes(".") && !domain.startsWith(".") && !domain.endsWith(".") && !/[\s<>,;]/.test(text);
}
