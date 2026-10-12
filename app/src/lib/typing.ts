/** Whether a key was pressed where words are typed or a choice is made, so that it is not also a key for the window. */
export function typing(event: KeyboardEvent): boolean {
    const target = event.target;
    if (!(target instanceof HTMLElement)) return false;
    return target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement || target instanceof HTMLSelectElement || target.isContentEditable;
}

/** Whether a dialog has the keys: this one was pressed within one, or one is open over everything else. */
export function inDialog(event: KeyboardEvent): boolean {
    const target = event.target;
    if (target instanceof Element && target.closest('dialog, [role="dialog"], [role="alertdialog"]')) return true;
    return document.querySelector('dialog[open], [aria-modal="true"]') !== null;
}
