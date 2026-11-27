# Raw events table doubles as the Stream event log

Events the **Daemon** produces itself (each **Recall**, including **Briefing** and **Handoff** deliveries, and every **Distillation** result) are stored as rows in `raw_events` next to the events **Clients** send through hooks. They are inserted with `distilled = 1`, so they never become **Distillation** input, and they never update the owning session. One table then serves the Stream tail, its filters, keyset paging and the existing retention prune, and these events survive a page reload.

## Considered Options

- **Separate table(s) read through a `UNION`**: keeps `raw_events` meaning "what the Client did", but every tail query, count and filter becomes a three-way union joined to `sessions` for client and project.
- **SSE only**: no schema change, but events vanish on reload or after the browser buffer fills, which is the gap the Stream review found (`docs/research/stream-page-review.md`).

## Consequences

- A query that means "Client activity" must filter by event type or `distilled = 0`. Distillation input already selects `distilled = 0`.
- Inserting a Daemon row must not touch `sessions.last_turn_at`. Otherwise a late "Distill now" revives an old session as an **Active Session** and as a **Handoff** source.
- Daemon rows are pruned with other distilled rows after the raw-event retention period.
- `usage_ledger` stays the source of truth for cost. The event row only carries what the tail displays.
