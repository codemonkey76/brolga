// A message written for a white page, recoloured for a dark one.
//
// Each colour is moved by what it is for: a light background is made dark,
// dark words are made light, and a colour that already suits a dark page is
// left as it is. So white words on a pale banner stay white, and the banner
// goes dark behind them. Hue is kept, so a brand's purple is still purple.

type Role = "ground" | "words" | "edge";

const role = (property: string): Role | null =>
    property === "color" ? "words" : property.startsWith("background") || property === "bgcolor" ? "ground" : property.startsWith("border") || property === "box-shadow" ? "edge" : null;

let probe: HTMLElement | undefined;
const read = new Map<string, [number, number, number, number] | null>();

/** A colour as the browser reads it: red, green, blue (0–255) and alpha. Null for what is not a colour. */
function parsed(token: string): [number, number, number, number] | null {
    const known = read.get(token);
    if (known !== undefined) return known;
    probe ??= document.body.appendChild(Object.assign(document.createElement("span"), { hidden: true }));
    probe.style.color = "";
    probe.style.color = token;
    let found: [number, number, number, number] | null = null;
    if (probe.style.color && !/^(inherit|initial|unset|revert|currentcolor|transparent)$/i.test(token)) {
        const parts = /^rgba?\(([\d.]+),\s*([\d.]+),\s*([\d.]+)(?:,\s*([\d.]+))?\)$/.exec(getComputedStyle(probe).color);
        if (parts) found = [Number(parts[1]), Number(parts[2]), Number(parts[3]), parts[4] === undefined ? 1 : Number(parts[4])];
    }
    read.set(token, found);
    return found;
}

function toHsl(r: number, g: number, b: number): [number, number, number] {
    r /= 255;
    g /= 255;
    b /= 255;
    const max = Math.max(r, g, b);
    const min = Math.min(r, g, b);
    const l = (max + min) / 2;
    if (max === min) return [0, 0, l];
    const d = max - min;
    const s = l > 0.5 ? d / (2 - max - min) : d / (max + min);
    const h = max === r ? (g - b) / d + (g < b ? 6 : 0) : max === g ? (b - r) / d + 2 : (r - g) / d + 4;
    return [h * 60, s, l];
}

/** One colour, moved to suit a dark page. Exported for the tests. */
export function suited(token: string, as: Role): string {
    const colour = parsed(token);
    if (!colour) return token;
    const [r, g, b, alpha] = colour;
    if (alpha === 0) return token;
    let [h, s, l] = toHsl(r, g, b);
    if (as === "ground" && l > 0.5) {
        // White becomes the page's dark grey; a pale tint becomes a dusky one.
        l = 0.16 + (1 - l) * 0.5;
        s *= 0.55;
    } else if (as === "words" && l < 0.6) {
        l = 0.9 - l * 0.45;
        s *= 0.8;
    } else if (as === "edge" && l > 0.5) {
        l = 0.26 + (1 - l) * 0.5;
        s *= 0.55;
    } else return token;

    return `hsla(${Math.round(h)},${Math.round(s * 100)}%,${Math.round(l * 100)}%,${alpha})`;
}

const COLOUR = /#[0-9a-f]{3,8}\b|(?:rgb|hsl)a?\([^)]*\)|[a-z]+/gi;

/** A list of declarations (`color:#111;background:#fff`), recoloured. */
export function declarations(css: string): string {
    return css
        .split(";")
        .map((each) => {
            const at = each.indexOf(":");
            if (at < 0) return each;
            const property = each.slice(0, at).trim().toLowerCase();
            const as = role(property);
            return as ? `${each.slice(0, at)}:${each.slice(at + 1).replace(COLOUR, (token) => suited(token, as))}` : each;
        })
        .join(";");
}

/** A whole style sheet, recoloured: every block of declarations in it. */
export const sheet = (css: string) => css.replace(/\{([^{}]*)\}/g, (_, inside: string) => `{${declarations(inside)}}`);

/** Recolour everything a message's own HTML says about colour, in place. */
export function recolour(body: HTMLElement) {
    for (const element of body.querySelectorAll<HTMLElement>("[style]")) element.setAttribute("style", declarations(element.getAttribute("style") ?? ""));
    for (const element of body.querySelectorAll("[bgcolor]")) element.setAttribute("bgcolor", suited(element.getAttribute("bgcolor") ?? "", "ground"));
    for (const element of body.querySelectorAll("font[color]")) element.setAttribute("color", suited(element.getAttribute("color") ?? "", "words"));
}
