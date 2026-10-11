<script lang="ts">
    import { execute } from "./commands";
    import { when } from "./mail";

    interface Props {
        accountId: string;
        address: string;
        /** What the person is called where they were hovered over, for until more is known. */
        name: string;
    }

    interface Card {
        person: { name: string; job_title: string; department: string; company: string; phones: string[]; source: "contact" | "directory" } | null;
        recent_mail: { id: string; subject: string; received_at: string }[];
        upcoming_meetings: { id: string; subject: string; start: string; is_all_day: boolean; location: string }[];
    }

    let { accountId, address, name }: Props = $props();
    let card = $state<Card | null>(null);
    let failed = $state(false);

    $effect(() => {
        card = null;
        failed = false;
        execute<Card>("people.card", { account_id: accountId, address })
            .then((found) => (card = found))
            .catch(() => (failed = true));
    });

    const day = (iso: string) => new Date(iso).toLocaleDateString(undefined, { weekday: "short", day: "numeric", month: "short" });
</script>

<div class="card" role="dialog" aria-label="About {card?.person?.name || name || address}">
    <p class="name">{card?.person?.name || name || address}</p>
    {#if card?.person}
        {@const person = card.person}
        {#if person.job_title || person.department}<p>{[person.job_title, person.department].filter(Boolean).join(" · ")}</p>{/if}
        {#if person.company}<p>{person.company}</p>{/if}
    {/if}
    <p class="muted">{address}</p>
    {#each card?.person?.phones ?? [] as phone (phone)}
        <p class="muted">{phone}</p>
    {/each}
    {#if card?.person}
        <p class="muted source">{card.person.source === "contact" ? "In your contacts" : "In your organisation"}</p>
    {/if}

    {#if card}
        {#if card.recent_mail.length > 0}
            <h3 id="card-mail">Recent mail</h3>
            <ul aria-labelledby="card-mail">
                {#each card.recent_mail as each (each.id)}
                    <li><span class="what">{each.subject || "(No subject)"}</span><span class="muted">{when(each.received_at)}</span></li>
                {/each}
            </ul>
        {/if}
        {#if card.upcoming_meetings.length > 0}
            <h3 id="card-meetings">Coming up together</h3>
            <ul aria-labelledby="card-meetings">
                {#each card.upcoming_meetings as each (each.id + each.start)}
                    <li><span class="what">{each.subject || "(No subject)"}</span><span class="muted">{day(each.start)}</span></li>
                {/each}
            </ul>
        {/if}
        {#if !card.person && card.recent_mail.length === 0 && card.upcoming_meetings.length === 0}
            <p class="muted">Nothing more is known of them here.</p>
        {/if}
    {:else if failed}
        <p class="muted">Nothing more could be found just now.</p>
    {:else}
        <p class="muted" role="status">Looking…</p>
    {/if}
</div>

<style>
    .card {
        position: absolute;
        z-index: 20;
        top: 100%;
        left: 0;
        width: 20rem;
        padding: 0.75rem;
        background: var(--surface);
        border: 1px solid var(--line);
        border-radius: 8px;
        box-shadow: 0 6px 24px rgb(0 0 0 / 18%);
        font-size: 0.85rem;
        font-weight: 400;
    }

    p {
        margin: 0;
    }

    .name {
        font-size: 1rem;
        font-weight: 600;
    }

    .source {
        margin-top: 0.25rem;
        font-size: 0.75rem;
    }

    h3 {
        margin: 0.6rem 0 0.2rem;
        font-size: 0.7rem;
        font-weight: 600;
        letter-spacing: 0.04em;
        text-transform: uppercase;
        color: var(--muted);
    }

    ul {
        margin: 0;
        padding: 0;
        list-style: none;
    }

    li {
        display: flex;
        justify-content: space-between;
        gap: 0.5rem;
    }

    .what {
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    }

    li .muted {
        flex: none;
    }
</style>
