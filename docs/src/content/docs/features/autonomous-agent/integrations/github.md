---
title: GitHub Integration
description: Use the GitHub CLI from Refact Agent.
---

The GitHub integration exposes the `github` tool to the agent. It runs GitHub CLI (`gh`) commands in the relevant project directory with the token configured in Refact settings.

## Setup

Configure:

- **GitHub token**: a personal access token with the scopes needed for the operations you want the agent to perform.
- **GitHub CLI path**: optional path to `gh`; leave empty to use `gh` from `PATH`.

Tokens can be referenced from secrets instead of being written directly into the integration file.

## What it is useful for

- Listing issues and pull requests.
- Creating issues or pull requests.
- Reading repository metadata.
- Running other `gh` commands that fit your confirmation rules.

## Confirmation rules

The integration can ask before risky commands such as delete or close operations and deny commands that expose authentication tokens. Adjust rules to match your repository policy.
