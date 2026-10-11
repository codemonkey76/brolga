<script lang="ts">
    import { execute, say } from "./commands";

    interface Props {
        accountId: string;
        username: string;
        onClose: (said?: string) => void;
    }

    interface When {
        date_time: string;
        time_zone: string;
    }

    interface Setting {
        status: "off" | "on" | "scheduled";
        internal_message: string;
        external_message: string;
        external_audience: "nobody" | "contacts" | "everyone";
        start: When | null;
        end: When | null;
    }

    let { accountId, username, onClose }: Props = $props();
    let setting = $state<Setting | null>(null);
    /** The scheduled times as this computer's clock has them, for the date and time boxes. */
    let start = $state("");
    let end = $state("");
    let problem = $state<string | null>(null);
    let saving = $state(false);

    const pad = (n: number) => String(n).padStart(2, "0");
    /** A time for a date-and-time box: local, to the minute. */
    const boxed = (date: Date) => `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}T${pad(date.getHours())}:${pad(date.getMinutes())}`;
    /** A time from the server as local time, where it is in UTC. One in a named zone cannot be turned here, and is left for the person to set. */
    const local = (when: When | null) => (when && when.time_zone === "UTC" ? boxed(new Date(`${when.date_time}Z`)) : "");
    /** A local time from a box, as the UTC the command takes. */
    const utc = (box: string) => new Date(box).toISOString().replace(/\.\d+Z$/, "Z");

    $effect(() => {
        execute<Setting>("mail.automatic_replies.get", { account_id: accountId })
            .then((got) => {
                setting = got;
                start = local(got.start);
                end = local(got.end);
            })
            .catch((failure) => (problem = say(failure)));
    });

    async function save() {
        if (!setting) return;
        if (setting.status === "scheduled" && (!start || !end)) {
            problem = "Choose when the replies start and when they end.";
            return;
        }
        saving = true;
        problem = null;
        try {
            await execute("mail.automatic_replies.set", {
                account_id: accountId,
                status: setting.status,
                internal_message: setting.internal_message,
                external_message: setting.external_message,
                external_audience: setting.external_audience,
                ...(setting.status === "scheduled" ? { start: utc(start), end: utc(end) } : {}),
            });
            onClose(setting.status === "off" ? "Automatic replies are off." : setting.status === "on" ? "Automatic replies are on." : "Automatic replies are scheduled.");
        } catch (failure) {
            problem = say(failure);
        } finally {
            saving = false;
        }
    }
</script>

<form class="replies" aria-label="Automatic replies for {username}" onsubmit={(event) => (event.preventDefault(), save())}>
    {#if problem}
        <p class="problem" role="alert">{problem}</p>
    {/if}

    {#if setting}
        <fieldset>
            <legend>Automatic replies</legend>
            <label><input type="radio" bind:group={setting.status} value="off" /> Off</label>
            <label><input type="radio" bind:group={setting.status} value="on" /> On, until turned off</label>
            <label><input type="radio" bind:group={setting.status} value="scheduled" /> On between two times</label>
        </fieldset>

        {#if setting.status === "scheduled"}
            <div class="times">
                <label>From <input type="datetime-local" bind:value={start} /></label>
                <label>Until <input type="datetime-local" bind:value={end} /></label>
            </div>
        {/if}

        <label class="stacked">
            To people in your organisation
            <textarea bind:value={setting.internal_message} rows="4"></textarea>
        </label>

        <label class="stacked">
            Reply to people outside it
            <select bind:value={setting.external_audience}>
                <option value="nobody">Nobody outside</option>
                <option value="contacts">Only my contacts</option>
                <option value="everyone">Everyone</option>
            </select>
        </label>
        {#if setting.external_audience !== "nobody"}
            <label class="stacked">
                To people outside your organisation
                <textarea bind:value={setting.external_message} rows="4"></textarea>
            </label>
        {/if}

        <p class="muted">Sent by Microsoft 365 to everyone who writes while this is on, whether or not Brolga is running.</p>
        <div class="buttons">
            <button type="submit" class="primary" disabled={saving}>{saving ? "Saving…" : "Save"}</button>
            <button type="button" onclick={() => onClose()}>Cancel</button>
        </div>
    {:else if !problem}
        <p class="muted" role="status">Reading from Microsoft 365…</p>
    {:else}
        <div class="buttons"><button type="button" onclick={() => onClose()}>Close</button></div>
    {/if}
</form>

<style>
    .replies {
        margin: 0.75rem 0 0 3rem;
        display: flex;
        flex-direction: column;
        gap: 0.6rem;
        max-width: 36rem;
    }

    fieldset {
        margin: 0;
        padding: 0;
        border: 0;
        display: flex;
        flex-direction: column;
        gap: 0.25rem;
    }

    legend {
        padding: 0;
        font-weight: 600;
        margin-bottom: 0.25rem;
    }

    .times,
    .buttons {
        display: flex;
        flex-wrap: wrap;
        gap: 0.75rem;
    }

    .stacked {
        display: flex;
        flex-direction: column;
        gap: 0.25rem;
        font-size: 0.9rem;
    }

    textarea,
    select,
    input[type="datetime-local"] {
        font: inherit;
        padding: 0.4rem;
    }

    p {
        margin: 0;
        font-size: 0.8rem;
    }

    .problem {
        padding: 0.5rem 0.75rem;
        background: color-mix(in srgb, var(--danger) 14%, var(--surface));
        font-size: 0.85rem;
    }
</style>
