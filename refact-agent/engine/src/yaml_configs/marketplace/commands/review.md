---
description: Review the current change set and highlight issues.
tags:
  - review
  - quality
---
Review the current change set using independent `subagent()` reviewers with different concrete `model_name` values against the same pinned checkout, diff and acceptance criteria. Select relevant domains: diff-only correctness, integration and compatibility, duplication and reuse, security/privacy, concurrency/lifecycle, spec compliance, tests, and performance. Restrict tools to authorized inspection and verification; review directly if delegation is forbidden. After independent discovery, exchange findings with `agent_message()` while reviewers are running, or launch fresh challengers if they have finished, for one round aimed at disproving each claim. Adjudicate by source evidence and executed checks rather than model majority, distinguish source-backed claims from executed reproductions, preserve unresolved disputes, and report unchecked domains. List actionable issues by severity and suggest fixes.
