---
title: Bitbucket Integration
description: Use the Bitbucket Cloud API from Refact Agent.
---

The Bitbucket integration exposes the `bitbucket` tool to the agent. It uses the Bitbucket Cloud API with your username, workspace, and app password.

## Setup

Configure:

- **App password**: a Bitbucket app password with the permissions needed for repository and pull request operations.
- **Username**: your Bitbucket username.
- **Workspace**: the Bitbucket workspace that owns the repositories.

Store app passwords in secrets when possible.

## What it can do

The tool supports repository-focused operations such as:

- Listing repositories in a workspace.
- Listing pull requests for a repository.
- Reading a pull request by id.
- Creating a pull request from a source branch to a destination branch.
- Reading a file from a repository at a commit or branch.

## Safety

Use confirmation rules for operations that create pull requests or read private repository content. Make sure the configured app password has only the permissions the agent needs.
