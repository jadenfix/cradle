# Beater Connect Architecture

Beater Connect is the bridge between ordinary websites and agent-native
interaction.

## Product Role

Beater has two existing anchors:

- `beater.js`: build and serve agent-ready apps.
- `beater-agents`: observe, replay, evaluate, and improve agent behavior.

Beater Connect sits between them. It describes what agents can read, search, and
safely do inside an app, then generates the public and authenticated surfaces
needed for interoperability.

## Surfaces

One registry emits:

| Surface | Purpose |
| --- | --- |
| `/.well-known/beater.json` | Beater manifest and endpoint discovery. |
| `/.well-known/agent-card.json` | A2A-style discovery for agent clients. |
| `/openapi.json` | Standard HTTP API contract. |
| `/mcp` | Tool/resource/prompt metadata for MCP transport integration. |
| `/llms.txt` | Curated LLM navigation file. |
| `/robots.txt` | Crawler policy and sitemap pointer. |
| `/sitemap.xml` | Crawlable URL inventory. |

## Registry Types

`Resource` describes agent-readable data: docs, products, support articles,
orders, tickets, or any other object with stable URLs.

`Action` describes agent-callable operations: search, draft, add to cart, book a
demo, create a ticket, send a message, purchase, delete, or publish.

`Policy` is carried directly on actions through:

- `Auth`
- scopes
- side-effect level
- confirmation requirement
- dry-run support
- idempotency requirement

`Receipt` is not implemented yet, but the registry is designed so every action
call can be traced and signed later with input/output hashes, approval state,
actor identity, and Beater Agents trace IDs.

## Side-Effect Model

The side-effect ladder is intentionally blunt:

```text
read -> draft -> write -> send -> purchase -> delete
```

Default policy:

- `read`: allowed when auth passes.
- `draft`: safe to preview.
- `write`: confirmation recommended.
- `send`: confirmation required.
- `purchase`: confirmation and spending limits required.
- `delete`: confirmation and elevated scope required.

The generator exposes this metadata to every surface so hosts can present clear
approval UI.

## Next Milestones

1. Add a live MCP JSON-RPC adapter backed by this registry.
2. Add OAuth/consent screens and action approval tokens.
3. Add receipt storage and Beater Agents trace export.
4. Add adapters for `beater.js`, Next.js, Express, Remix, and plain Rust Axum.
5. Add structured search resources and markdown extraction for crawl views.
