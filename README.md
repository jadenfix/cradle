# beater-connect

Beater Connect is the agent access layer for Beater apps.

It lets a site declare agent-readable resources and agent-callable actions once,
then emits the protocol and crawl surfaces agents need:

- `/.well-known/beater.json`
- `/.well-known/agent-card.json`
- `/openapi.json`
- `/mcp` catalog metadata
- `/llms.txt`
- `/robots.txt`
- `/sitemap.xml`
- clean markdown/JSON resource views
- action policy metadata for auth, confirmation, dry runs, and receipts

The goal is simple: a Beater app should be usable by humans, APIs, crawlers, and
AI agents without four separate integration projects.

## Quickstart

Generate the demo bundle:

```sh
cargo run -- demo --out .agent
```

Inspect generated files:

```sh
find .agent -type f | sort
cat .agent/llms.txt
cat .agent/openapi.json
```

Run tests:

```sh
cargo test
```

## Core Model

```rust
use beater_connect::{
    Action, Auth, ConnectApp, Field, FieldKind, Resource, Schema, SideEffect,
};

let app = ConnectApp::new(
    "Acme Store",
    "Product catalog and demo booking for AI agents.",
    "https://acme.example",
)
.resource(Resource::new(
    "products",
    "Products",
    "Browse public product information.",
    "/products",
    "/products.md",
))
.action(
    Action::new(
        "book_demo",
        "Book demo",
        "Schedule a product demo for the signed-in user.",
        "POST",
        "/agent/actions/book-demo",
        SideEffect::Write,
    )
    .auth(Auth::user(["demo:book"]))
    .confirm(true)
    .dry_run(true)
    .input(Schema::object([
        Field::new("email", FieldKind::String).required(true),
        Field::new("time", FieldKind::String).required(true),
    ])),
);
```

From that single registry, Beater Connect generates crawl metadata, OpenAPI,
A2A-style discovery, MCP-facing tools/resources metadata, and policy hints.

## MVP Boundaries

This repository currently implements the registry and static generators. It does
not yet implement a live MCP JSON-RPC transport, OAuth server, or Beater Agents
trace exporter. Those are the next integration layers.

## Design Rules

- MCP is one surface, not the only surface.
- Public crawl content and authenticated user data stay separate.
- Mutating actions require idempotency keys.
- Destructive, financial, messaging, publishing, and account-changing actions
  require confirmation by default.
- Every action has policy metadata: side-effect level, auth, scopes, dry-run
  support, confirmation requirement, and receipt expectations.
- OpenAPI, MCP metadata, A2A metadata, and crawl files are generated from the
  same registry so they cannot drift.
