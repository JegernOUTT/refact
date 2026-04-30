---
title: PostgreSQL Integration
description: Run PostgreSQL queries from Refact Agent.
---

The PostgreSQL integration exposes the `postgres` tool to the agent. It runs one SQL query per tool call through `psql` using the connection settings configured in Refact.

## Setup

Configure:

- **Host and port**: where PostgreSQL is reachable.
- **User, password, and database**: connection credentials and target database.
- **psql path**: optional path to the `psql` binary; leave empty to use `psql` from `PATH`.

Values can reference variables or secrets so credentials do not need to be written directly into prompts.

## Query behavior

- Each tool call runs a single query.
- Queries time out after a short fixed timeout.
- Large result sets and long cells are truncated before being returned to chat.
- Errors from `psql` are shown to the agent for debugging.

## Safety

Configure confirmation rules for write queries, schema changes, or maintenance commands. The default configuration asks before commands that are not simple `SELECT`-style reads; adjust it to fit your environment.
