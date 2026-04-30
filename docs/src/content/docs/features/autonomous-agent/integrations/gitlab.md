---
title: GitLab Integration
description: Use the GitLab CLI from Refact Agent.
---

The GitLab integration exposes the `gitlab` tool to the agent. It runs GitLab CLI (`glab`) commands in the relevant project directory with the token configured in Refact settings.

## Setup

Configure:

- **GitLab token**: a personal access token with the scopes needed for issues, merge requests, and repository operations.
- **GitLab CLI path**: optional path to `glab`; leave empty to use `glab` from `PATH`.

Tokens can be referenced from secrets instead of being written directly into the integration file.

## What it is useful for

- Listing issues and merge requests.
- Creating issues or merge requests.
- Reading project metadata.
- Running other `glab` commands that fit your confirmation rules.

## Confirmation rules

Use confirmation rules to ask before destructive commands and deny commands that expose authentication tokens.
