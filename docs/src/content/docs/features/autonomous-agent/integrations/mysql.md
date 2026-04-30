---
title: MySQL Integration
description: Run MySQL queries from Refact Agent.
---

The MySQL integration exposes the `mysql` tool to the agent. It runs one SQL query per tool call through the `mysql` client using the connection settings configured in Refact.

## Setup

Configure:

- **Host and port**: where MySQL is reachable.
- **User, password, and database**: connection credentials and target database.
- **mysql path**: optional path to the `mysql` binary; leave empty to use `mysql` from `PATH`.

Values can reference variables or secrets so credentials do not need to be written directly into prompts.

## Query behavior

- Each tool call runs a single query.
- Queries time out after a short fixed timeout.
- Large result sets and long cells are truncated before being returned to chat.
- Errors from `mysql` are shown to the agent for debugging.

## Safety

Configure confirmation rules for writes, schema changes, and maintenance commands. Use read-only credentials when the agent only needs to inspect data.
