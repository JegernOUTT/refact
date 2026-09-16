# Multi-agent parity: final certification against the coverage ledger

This document answers one question: **is the program finished?** It walks the coverage
ledger of `docs/multi-agent-parity-plan.md` row by row and applies that plan's own
completion definition, which is deliberately strict:

> Completion requires each coverage-ledger row to link to an implemented path, a
> public/agent-facing reachability test, a failure-path test and an observed passing
> verification result. A type definition, passing unit test of an unused helper, UI-only
> feature when the agent needs it, or a marketing statement is not parity.

Four columns, each independently satisfied. A row with an implementation and no
failure-path test is **partial**, not complete, and is written down as partial.

No production code was changed to produce this certification; the diff is this file.
Every count below came from a command executed in this worktree at the commit named in
the next section. Nothing is inherited from a card report or from a previous document.

## A note on section numbers

The O-5 card cites "plan §13 coverage ledger" and "§14 completion definition". Those are
the numbers of the **pre-amendment** revision. In the amended plan of record they are
**§11** (coverage ledger) and **§12** (M10, containing the completion definition quoted
above); §13 is now "Approval and execution discipline". This certification follows the
amended document.

## Environment

| | |
|---|---|
| Worktree | `/home/svakhreev/.cache/refact/worktrees/a5ef23b532caf0f6/58bf83ad-2ff1-442c-889c-bed0aa06c7dd` |
| Branch | `refact/task/d3bd0b85-.../card/O-5/a7f35f9f` |
| **Certified commit** | **`97ad671747783a1504c56420c1c6b70f975bb573`** (`97ad67174`, "O-4 merged into integration branch") |
| Tree state | clean — `git status --porcelain` produced no output (C12). Every command below ran against the tree at `97ad67174`; the only commit added afterwards is the one that introduces this file, which changes no source |
| Date | 2026-09-16 |
| Platform | Linux x86_64 |

## Executed evidence

Engine commands run from `refact-agent/engine`, GUI commands from `refact-agent/gui`.
The exit code column is the real exit status of the process.

| # | Command | Observed result | Exit |
|---|---|---|---|
| C0 | `cargo test --lib --no-run -p refact-lsp` | built the lib test binary, cold, 1256s (20m56s) | 0 |
| C1 | `cargo test --lib -p refact-lsp -- --list --format terse` | **6353** test names discovered | 0 |
| C2 | `cargo test --lib -p refact-lsp` | `ok. **6328 passed**; 0 failed; 25 ignored; 0 filtered out; finished in 135.62s` | 0 |
| C3 | `cargo test --doc -p refact-lsp` | `ok. **0 passed**; 0 failed; 0 ignored; 0 filtered out` — the crate has no doc tests; 968s, spent rebuilding for rustdoc | 0 |
| C4 | `cargo test -p refact-work-core -p refact-tasks -p refact-worktrees -p refact-chat-api -p refact-providers -p refact-sandbox` | **851 passed; 0 failed** over 16 test binaries (see breakdown below), 32.7s | 0 |
| C5 | `cargo fmt --all -- --check` | no diffs (only nightly-only-option warnings) | 0 |
| C6 | `npx vitest run --reporter=dot` | `Test Files **388 passed** (388)`, `Tests **4515 passed** \| 6 skipped (4521)`, 259.38s | 0 |
| C7 | `npm run types` | `tsc --noEmit`, no errors, 21.1s | 0 |
| C8 | `npm run lint` | eslint `--max-warnings 0`, clean, 57.1s | 0 |
| C9 | `npm run format:check` | prettier clean, 13.3s | 0 |
| C10 | `cargo test --lib -p refact-lsp -- --exact agents::tool_authority::tests::canonical_aliases_and_transport_are_not_ambient_authority` | `running 0 tests` — `ok. **0 passed**; 0 failed; 6353 filtered out`: **the name cited by the M1 ledger matches no test** | 0 |
| C11 | `cargo test --lib -p refact-lsp -- --exact agents::tool_authority::tests::canonical_aliases_and_mcp_transport_are_not_ambient_authority` | `ok. **1 passed**; 0 failed; 6352 filtered out` — the real name, with `mcp_` | 0 |
| C12 | `git status --porcelain` | empty — nothing uncommitted at the certified commit | 0 |
| C13 | citation audit — `grep -oE "[a-z_][a-z_0-9]*(::[a-z_0-9]+)+" docs/parity-certification.md`, then `grep -vxF -f` against C1's inventory, then `grep -vxFf` against the names found in the C4 log | every engine name cited below resolves in C1; all **22** crate-test names cited below appear in the C4 log (the "missing" list came back empty); the only deliberately non-resolving name in this file is the stale M1 citation in C10 | 0/1 (`grep -v` exits 1 on an empty result) |
| C14 | `grep -E "WorkPanel.test.tsx\|AttentionPanel.test.tsx\|CausalTracePanel.test.tsx" <C6 log>` | `✓ WorkPanel.test.tsx (13 tests)`, `✓ CausalTracePanel.test.tsx (10 tests)`, `✓ AttentionPanel.test.tsx (9 tests)` | 0 |

C4 breakdown, per test binary, all `0 failed`:

| Crate | Binary | Passed |
|---|---|---:|
| refact-chat-api | lib | 208 |
| refact-providers | lib | 318 |
| refact-sandbox | lib / `tests/enforcement.rs` | 7 / 6 |
| refact-tasks | lib | 82 |
| refact-work-core | lib | 37 |
| refact-work-core | `approval_settings_tests` / `approval_tests` | 1 / 10 |
| refact-work-core | `disagreement_tests` / `fairness_regressions` | 9 / 6 |
| refact-work-core | `priority_tests` / `residency_tests` | 7 / 4 |
| refact-work-core | `scheduler_tests` / `settings_tests` / `store_tests` | 30 / 17 / 21 |
| refact-worktrees | lib | 88 |

**How to read the row table.** Every test named below was resolved against the C1
inventory (so no row cites a test that does not exist) and executed: engine `src/` tests
in **C2**, crate tests in **C4**, GUI tests in **C6**. Where a row's surface is a GUI
file, the file is named and its tests ran in C6.

## The coverage ledger, row by row

### Surviving rows

| # | Capability family (plan §11) | Implemented path | Reachability test | Failure-path test | Verdict |
|---|---|---|---|---|---|
| 1 | New-chat live project overview; unrelated-root work; attention/current tool | `src/coordination/service.rs::overview`, `src/coordination/catalog.rs::build_catalog`, tool `src/tools/tool_work_overview.rs`, `src/coordination/presence.rs` | `coordination::service_tests::shared_projection_paginates_without_gaps_and_rejects_changed_state`; `http::routers::v1::work::tests::gui_route_and_model_tool_agree_on_the_same_projection`; `coordination::presence::tests::catalog_freshness_follows_real_peer_presence` | `tools::tool_work_overview::tests::strict_request_rejects_invalid_types_limits_and_spoofed_roots`; `coordination::presence::tests::an_idle_engine_is_never_advertised_as_running` | **complete** |
| 2 | Historical search/read, worktree-family scoping, source ranges, untrusted labels | `src/coordination/history.rs`, `src/agents/launch_spec.rs::resolve_message_range`, `src/chat/prompts.rs` memory injection | `coordination::history::tests::historical_rows_follow_the_index_without_per_request_body_reads_and_live_rows_win`; `agents::launch_spec::tests::resolve_message_range_includes_complete_tool_group`; `chat::prompts::tests::task_memory_injection_carries_provenance_and_trust_label` | `coordination::history::tests::legacy_or_denied_history_never_discloses_to_a_model`; `agents::launch_spec::tests::resolve_file_range_blocks_privacy_denied_paths`; `coordination::history::tests::one_malformed_index_message_invalidates_all_provenance` | **complete** |
| 3 | Shared project memory, session digests, corrections, consolidation, recoverable omitted output | `src/memories.rs::memories_add_dedup`/`memories_upsert_by_signal_key`/`memories_add_preference_if_new`, `src/tools/tool_task_memory.rs::MemoryAccessContext::decide`, `crates/refact-chat-api/src/deliverable.rs::DeliverableOmission` rendered by `src/agents/push.rs` | `tools::tool_task_memory::tests::task_mem_search_finds_tag_filename_and_content`; `tools::tool_task_memory::tests::recall_carries_writer_time_run_and_trust_label`; `memories::tests::memories_add_dedup_touches_exact_normalized_duplicate` | `tools::tool_task_memory::tests::task_mem_get_hides_another_agents_private_memory`; `tools::tool_task_memory::tests::denied_memory_leaks_no_title_and_no_count`; `deliverable::tests::insufficient_headroom_reports_a_recoverable_omission_instead_of_a_met_section` (C4) | **complete** |
| 4 | Read stamps, named writer warnings, per-path serialization, integration overlaps | `src/file_activity.rs`, `src/coordination/file_provenance.rs`, `src/tools/file_edit/auxiliary.rs` | `file_activity::tests::cooperating_processes_serialize_and_refuse_a_stale_visible_read`; `coordination::file_provenance::tests::a_write_recorded_by_one_engine_is_named_by_another_after_a_restart` | `tools::file_edit::auxiliary::tests::worktree_scope_tools::every_text_mutation_family_rejects_a_stale_model_visible_read`; `file_activity::tests::a_write_with_no_provenance_record_is_reported_as_external_or_unknown`; `coordination::service_tests::same_relative_path_in_a_separate_checkout_is_an_advisory_overlap_only` | **complete** |
| 5 | Cross-process claim, lease/fencing, stale-holder/PID-reuse detection | `src/tasks/board_lock.rs`, `src/tasks/storage.rs` guarded transaction, `src/coordination/leases.rs`, `src/coordination/presence.rs` | `tasks::storage::tests::cooperating_processes_cannot_both_claim_one_board_card`; `coordination::leases::tests::two_acquirers_race_and_exactly_one_owns_the_lease` | `coordination::leases::tests::a_replaced_holders_fenced_write_is_rejected_and_never_lands`; `coordination::presence::tests::a_reused_pid_classifies_dead_not_alive`; `coordination::leases::tests::takeover_is_refused_from_a_live_or_unclassified_holder_and_recorded_otherwise` | **complete** |
| 6 | Active concurrency vs launch cap; fair queues, parked parents, structured errors | `crates/refact-work-core/src/scheduler.rs::AdmissionScheduler`, `src/tasks/admission.rs`, `src/agents/execution.rs` | `agents::spawn_lifecycle_tests::parent_parks_its_only_permit_during_agent_wait_and_reacquires_before_continuing`; `tasks::admission::tests::board_and_chat_descendants_share_project_and_root_capacity`; `scheduler_tests::root_round_robin_prevents_one_roots_backlog_from_monopolizing_releases` (C4) | `scheduler_tests::queue_full_is_rejected_explicitly` and `outstanding_full_is_rejected_explicitly` (C4); `tasks::admission::tests::refused_reservation_leaves_no_phantom_child_and_allocates_no_setup_task` | **complete** |
| 7 | Per-provider AIMD, Retry-After, quota handling, rate/capacity limits | `crates/refact-providers/src/pressure.rs::PressureRegistry`, `src/llm/pressure.rs` | `pressure::tests::only_rate_limit_drives_concurrency_down_and_quota_is_not_retried`; `pressure::tests::retry_after_http_date_is_parsed_and_honoured`; `pressure::tests::sustained_success_grows_additively_up_to_the_ceiling` (all C4) | `pressure::tests::quota_exhaustion_neither_backs_off_nor_retries`; `pressure::tests::auth_failure_is_never_retried`; `pressure::tests::invalid_configuration_is_rejected_not_coerced` (C4); `llm::pressure::tests::invalid_settings_are_rejected_instead_of_coerced` | **complete** |
| 8 | Parent/child/peer live mail, broadcast, no-wake vs follow-up, interrupt, missed steer | `src/chat/delivery.rs`, `src/agents/delivery.rs`, `src/coordination/broadcast.rs`, `src/coordination/outbox.rs`, tool `src/tools/tool_agent_interact.rs` | `tools::tool_agent_interact::tests::mailbox_lists_the_callers_own_deliveries_bounded_and_paged`; `tools::tool_agent_interact::tests::child_messages_preserve_all_push_modes`; `coordination::outbox::tests::outbox_restart_redelivers_a_pending_message_exactly_once_logically` | `tools::tool_agent_interact::tests::agent_message_receipt_never_claims_a_dropped_message_was_queued`; `tools::tool_agent_interact::tests::parent_message_rejects_agent_outside_its_tree`; `coordination::outbox::tests::outbox_unknown_outcome_is_surfaced_instead_of_being_retried` | **complete** |
| 9 | Generic questions/waits, plan/permission escalation, graceful shutdown | `src/questions.rs`, `src/agents/approval.rs`, `src/tools/tool_ask_questions.rs` | `questions::tests::a_waiter_releases_its_permit_and_reacquires_before_continuing`; `tools::tool_agent_interact::tests::child_question_records_question_and_queues_parent_notice`; `agents::approval::tests::child_command_approval_parks_without_side_effects_then_resumes_with_exact_binding` | `questions::tests::unresolved_cross_tree_authority_refuses_rather_than_defaulting_to_allow`; `questions::tests::a_parent_cannot_grant_authority_it_lacks_and_escalates_instead`; `questions::tests::an_abandoned_wait_cancels_its_question_and_gives_the_permit_back` | **complete** |
| 10 | Fresh/fork/full or bounded history, resumable/cold children, background promotion | `src/agents/spawn.rs`, `src/agents/lifecycle.rs`, `src/tools/tool_subagent.rs`, `src/tools/tool_background_agents.rs` | `tools::tool_subagent::tests::public_fork_launch_uses_completed_snapshot_and_reports_queued_without_side_effects`; `agents::spawn_lifecycle_tests::cold_restart_resumes_authoritative_child_without_replaying_unknown_effects`; `agents::spawn_lifecycle_tests::completed_child_takes_a_continued_turn_without_rewriting_its_completion` | `tools::tool_background_agents::tests::agent_resume_refuses_plain_resume_on_a_completed_child`; `agents::spawn::resume_validation_tests::resume_requires_complete_nonduplicated_tool_pairs`; `agents::lifecycle::tests::closing_refuses_a_stale_generation_a_foreign_parent_and_a_finished_child` | **complete** |
| 11 | Profiles/personas, tool/model/effort selection, deny ceilings, mode/file constraints | `src/agents/profile.rs`, `src/agents/tool_authority.rs`, `src/agents/path_scope.rs`, `src/agents/sandbox_contract.rs` | `agents::profile::tests::profile_defaults_then_run_narrowing_produce_the_effective_config`; `tools::tool_subagent::tests::profile_reference_accepts_a_name_or_a_pinned_revision`; `agents::path_scope::tests::strict_scope_admits_declared_paths_and_names_the_scope_on_refusal` | `agents::profile::tests::run_instance_widening_the_profile_is_a_structured_refusal`; `agents::sandbox_contract::tests::an_unenforceable_request_fails_before_dispatch_rather_than_degrading`; `agents::tool_authority::tests::empty_and_inherited_ceilings_never_expand_concrete_authority` | **complete** |
| 12 | Worktree creation, inventory, nested integration, post-merge validation, conflict resolver | `crates/refact-worktrees/src/service.rs`, `src/coordination/merge_gate.rs::merge_under_lease`, `src/chat/post_merge_check.rs`, `src/tools/tool_task_merge_agent.rs` | `coordination::merge_gate::tests::two_concurrent_merges_into_one_checkout_both_land`; `coordination::service_tests::inspector_scopes_files_and_pages_worktree_inventory`; `tools::tool_task_merge_agent::worktree_merge_tool_tests::registered_force_merge_finalizes_after_post_merge_check_passes` | `coordination::merge_gate::tests::a_dirty_target_checkout_refuses_the_merge_and_the_user_changes_survive`; `coordination::merge_gate::tests::a_conflicting_merge_reports_its_files_and_preserves_both_worktrees`; `coordination::merge_gate::tests::a_merge_that_cannot_take_the_lease_fails_retryably_instead_of_blocking` | **complete** |
| 13 | Typed assignment, task DAG/claiming, parallel/sequence/pipeline/waves | `src/tasks/assignment.rs`, `crates/refact-tasks/src/waves.rs::derive_waves`/`advance_pipeline`, `src/tools/tool_task_batch.rs` | `tasks::assignment::tests::the_validated_assignment_is_the_dispatched_one_after_the_card_changes`; `waves::tests::waves_derive_deterministically_from_the_dependency_dag` and `a_repeated_agent_id_in_a_pipeline_advances_by_position_not_first_match` (C4); `tools::tool_task_batch::tests::spawn_agents_batch_schedule_dispatches_only_the_admitted_wave` | `waves::tests::a_failed_required_card_blocks_its_waves_successors_with_a_structured_reason` and `an_over_deep_pipeline_lane_is_refused` (C4); `tasks::storage::tests::claiming_a_blocked_card_reports_its_unsatisfied_dependencies` | **complete** |
| 14 | Structured child schemas and correction retry; mission log; reviewed outcome sections | `crates/refact-chat-api/src/deliverable.rs`, `crates/refact-tasks/src/types.rs::evaluate_finalize`/`finalize_card`, `src/tools/tool_task_agent_finish.rs`, `src/coordination/teams.rs` mission log | `tools::tool_subagent::tests::public_launch_binds_deliverable_contract_and_reports_structural_outcome`; `tools::tool_task_agent_finish::tests::agent_finish_records_the_outcome_document_and_leaves_done_agent_declared`; `coordination::teams::tests::the_mission_log_is_bounded_and_append_only` | `agents::spawn_lifecycle_tests::deliverable_contract_unmet_spends_one_bounded_correction_then_reports_unmet`; `types::tests::a_rejected_required_fragment_blocks_finalize_even_when_another_is_approved` and `an_unresolved_contradicted_required_fragment_blocks_finalize` (C4); `tools::tool_task_agent_finish::tests::an_empty_report_renders_cleanly_but_satisfies_no_declared_requirement` | **complete** |
| 15 | Explicit durable goals, unlimited defaults, pause/resume/quiescence, CAS revisions | `src/chat/session.rs` (`GoalBudget`), `src/chat/goal_verifier.rs`, tools `tool_set_goal`/`tool_update_goal`/`tool_validate_goal` | `tools::tool_validate_goal::tests::met_verdict_finalizes_and_formats_tool_message`; `chat::goal_verifier::tests::apply_goal_verdict_met_finalizes_completed_and_records_attempt`; `chat::session::tests::goal_budget::default_budget_never_exhausts` | `chat::goal_verifier::tests::apply_goal_verdict_discards_verdict_when_status_changed_after_epoch`; `chat::goal_verifier::tests::apply_goal_verdict_unmet_preserves_pause_requested_during_verification`; `chat::trajectories::tests::clamp_goal_snapshot_for_load_migrates_implicit_legacy_budget` | **complete** |
| 16 | Executed gates/artifacts/assertions, adversarial review, stale verdict guards, cached evidence | `src/chat/evidence.rs` (`VerificationSpec`, evidence service), `src/chat/evidence_cache.rs`, `src/chat/evidence_revision.rs`, `src/chat/goal_review.rs` | `chat::evidence::tests::executed_evidence_records_command_exit_fingerprint_and_content_revision`; `chat::goal_review::tests::the_goal_path_obtains_a_review_report_without_a_tool_call`; `chat::evidence_cache::tests::from_cache_reaches_the_consumer` | `chat::evidence::tests::a_failing_required_gate_blocks_completion_even_when_the_report_claims_success`; `chat::evidence::tests::verify_hint_prose_is_advisory_and_is_never_executed`; `chat::evidence_revision::tests::editing_a_declared_file_after_a_pass_supersedes_the_verdict`; `chat::evidence_cache::tests::a_replayed_failure_is_not_reported_as_a_fresh_run` | **complete** |
| 17 | Wait barriers on child/process/CI/deadline, no wasted continuation | `src/chat/goal_wait.rs`, `src/chat/goal_verifier.rs` (`GoalVerdict::Wait`) | `chat::goal_verifier::tests::a_wait_verdict_parks_the_goal_without_completing_or_stalling_it`; `chat::goal_wait::tests::a_settled_subject_wakes_the_goal_exactly_once`; `chat::goal_monitor::tests::a_deadline_racing_a_settled_subject_resumes_the_goal_once` | `chat::goal_verifier::tests::a_wait_verdict_must_name_a_typed_subject`; `chat::goal_verifier::tests::a_wait_verdict_never_overrides_a_paused_goal`; `chat::goal_wait::tests::a_wake_for_another_subject_leaves_the_wait_standing` | **complete** |
| 18 | Plan approval, files/task state, bounded compaction-safe current plan + full audit | `src/tools/tool_set_plan.rs`, `src/tools/tool_update_plan.rs`, `src/tools/tool_get_plan.rs` (checkpoint projection), `src/chat/plan_role.rs` | `tools::tool_get_plan::tests::get_plan_checkpoints_two_hundred_deltas_without_losing_approved_constraints`; `tools::tool_get_plan::tests::get_plan_history_recovers_the_full_audit_trail_after_checkpointing`; `chat::cache_stability_tests::cache_stability_plan_checkpoint_appends_without_changing_the_prior_wire_prefix` | `tools::tool_set_plan::tests::requires_exactly_one_of_path_or_content`; `tools::tool_set_plan::tests::path_rejects_oversized_plan_file_before_reading`; `tools::tool_get_plan::tests::legacy_plan_trajectory_without_checkpoint_loads_and_projects` | **complete** |
| 19 | Hook control events/types and safe extensions | `src/ext/hooks_runner.rs`, surface `GET /v1/hooks` → `src/http/routers/v1/hooks.rs::handle_v1_hooks_surface` | `ext::hooks_runner::tests::test_hooks_surface_names_the_hook_that_denied_and_respects_deny_precedence`; `ext::hooks_runner::tests::test_every_event_builds_a_payload_naming_itself`; `http::routers::v1::hooks::tests::hooks_fire_hook_id_runs_matching_webhook_jobs_only` | `ext::hooks_runner::tests::test_timeout_is_a_deny_and_never_falls_through_to_allow`; `ext::hooks_runner::tests::test_untrusted_project_hook_is_not_executed_until_trusted`; `ext::hooks_runner::tests::test_rewrite_cannot_widen_child_tool_authority`; `http::routers::v1::hooks::tests::hooks_fire_auth_requires_daemon_bearer_when_configured` | **complete** |
| 20 | Governed code-mode orchestration, tool catalog search, bounded intermediate data | `src/code_mode/{mod,policy,session,hosts}.rs` | `code_mode::hosts::tests::code_mode_declares_a_closed_catalog_that_names_its_backing_tool`; `code_mode::session::tests::the_host_call_count_and_intermediate_size_are_both_capped` | `code_mode::session::tests::code_mode_refuses_to_run_when_no_sandbox_provider_can_enforce_it`; `code_mode::policy::tests::an_open_authority_is_refused_rather_than_read_as_everything`; `code_mode::session::tests::a_confined_worker_cannot_write_outside_the_sandbox_or_open_a_socket` | **complete** |
| 21 | Human surfaces and diagnostic visibility | GUI `features/Workspace/{WorkPanel,AttentionPanel,CausalTrace}`, HTTP `v1/{work,attention,causal,hooks}.rs`, `src/coordination/metrics.rs` | `http::routers::v1::work::tests::gui_route_and_model_tool_agree_on_the_same_projection`; GUI `WorkPanel.test.tsx` (13), `AttentionPanel.test.tsx` (9), `CausalTracePanel.test.tsx` (10) — C6/C14 | `WorkPanel.test.tsx` withheld/unavailable cases (C6); `http::routers::v1::attention::tests::the_attention_queue_requires_authorization_before_it_reveals_anything` | **partial** — see gap 21a/21b |
| 22 | **A** — Wait-graph deadlock detection with authorized break actions | `crates/refact-work-core/src/waitgraph.rs::detect`/`detect_stable`/`refuse`, `src/coordination/deadlock.rs::visible_view`, reachable as `work_inspect{selector:{kind:"wait_graph"}}` (`InspectSelector::WaitGraph`) | `coordination::deadlock::tests::a_parent_waiting_on_a_child_that_asks_it_back_is_reported_with_both_names`; `coordination::deadlock::tests::the_visible_view_names_only_runs_the_caller_may_see`; `waitgraph::tests::a_refusal_lands_on_the_wait_that_closed_the_loop` (C4) | `coordination::deadlock::tests::a_graph_past_its_bounds_is_truthfully_incomplete`; `coordination::deadlock::tests::a_wait_on_the_user_is_never_a_deadlock`; `coordination::deadlock::tests::a_long_chain_with_a_progressing_tail_is_never_reported` | **complete** |
| 23 | **B** — Handoff sufficiency feedback into later decomposition | `src/agents/handoff.rs` (`finding_for`, `metric_for`, `redact_metric`), consumed by `src/tools/tool_subagent.rs` | `tools::tool_subagent::tests::a_child_handoff_declaration_is_recorded_and_an_absent_one_stays_unknown`; `tools::tool_subagent::tests::prior_handoff_gaps_surface_in_the_preview_without_widening_scope_or_authority` | `agents::handoff::tests::a_handoff_finding_stays_insufficient_until_the_sample_is_large_enough`; `agents::handoff::tests::an_exploratory_card_never_reads_as_an_inadequate_brief_and_no_template_is_touched`; `agents::handoff::tests::declared_paths_are_filtered_for_the_parent_destination_before_they_reach_a_model` | **complete** |
| 24 | **C** — Human attention budget for approvals, questions and notifications | `src/attention.rs::AttentionBudget`, HTTP `src/http/routers/v1/attention.rs`, GUI `features/Workspace/AttentionPanel/AttentionPanel.tsx` | `http::routers::v1::attention::tests::the_queue_surface_reports_rank_and_what_was_merged_into_a_prompt`; `attention::tests::two_identical_questions_become_one_prompt_for_the_human`; GUI `AttentionPanel.test.tsx` (C6) | `attention::tests::an_approval_or_irreversible_question_never_defaults_however_long_it_waits`; `attention::tests::a_saturated_human_refuses_low_priority_questions_instead_of_queueing_them`; `http::routers::v1::attention::tests::the_attention_queue_requires_authorization_before_it_reveals_anything` | **complete** |
| 25 | **D** — Structured disagreement records and adjudication decisions | `src/coordination/disagreement.rs` (ledger + `completion_gate`), tools `disagreement_file`/`disagreement_list`/`disagreement_adjudicate` in `src/tools/tool_disagreement.rs` | `tools::tool_disagreement::tests::the_disagreement_tools_publish_both_filing_and_reading`; `tools::tool_disagreement::tests::the_disagreement_tool_surface_files_reads_and_adjudicates_without_losing_a_side` | `tools::tool_disagreement::tests::the_disagreement_tool_refuses_a_verdict_it_cannot_ground_in_evidence`; `coordination::disagreement::tests::an_unresolvable_disagreement_stays_open_and_a_moved_revision_goes_stale`; `coordination::disagreement::tests::the_blocking_policy_is_honoured_in_both_directions` | **complete** |
| 26 | **E** — Shared fact ledger with contradiction detection and trust labels | `src/coordination/fact_ledger.rs`, tools `claim_record`/`claim_list` in `src/tools/tool_fact_ledger.rs` | `tools::tool_fact_ledger::tests::the_claim_tools_publish_both_recording_and_reading`; `coordination::fact_ledger::tests::a_read_always_carries_provenance_confidence_and_the_untrusted_label` | `coordination::fact_ledger::tests::a_contradicting_claim_never_overwrites_and_files_a_disagreement`; `coordination::fact_ledger::tests::a_claim_without_provenance_or_a_subject_is_refused`; `coordination::fact_ledger::tests::a_claim_whose_source_revision_moved_is_stale_and_not_a_contradiction` | **complete** |
| 27 | **F** — Cross-agent causal trace and deterministic replay of recorded decisions | `src/coordination/causal.rs` (recording), `src/coordination/causal_view.rs` (replay), HTTP `src/http/routers/v1/causal.rs`, GUI `features/Workspace/CausalTrace/CausalTracePanel.tsx` | `http::routers::v1::causal::tests::a_scoped_trace_is_returned_and_authorization_precedes_any_reconstruction`; `coordination::causal_view::tests::replay_of_a_recorded_run_reproduces_the_recorded_coordination_outcomes`; GUI `CausalTracePanel.test.tsx` (C6) | `http::routers::v1::causal::tests::an_unrecorded_root_is_an_explicit_gap_rather_than_an_empty_success`; `http::routers::v1::causal::tests::a_privacy_denied_node_leaks_nothing_and_leaves_a_visible_gap`; `coordination::causal::tests::an_unknown_cause_is_recorded_as_unknown_and_never_guessed` | **complete** |
| 28 | **G** — Decomposition advisor proposals requiring acceptance | `src/coordination/decomposition.rs` (`advise_for_model`, `render`), consumed by `src/tools/tool_subagent.rs` | `coordination::decomposition::tests::single_and_fanned_out_runs_are_recorded_with_their_fan_out_context`; `coordination::decomposition::tests::aggregation_compares_wall_clock_and_tokens_for_both_cohorts`; `coordination::decomposition::tests::querying_the_advisor_changes_no_scheduling_state` | `coordination::decomposition::tests::below_the_minimum_sample_size_the_advisor_refuses_instead_of_averaging`; `coordination::decomposition::tests::without_a_store_the_advisor_reports_unavailable`; `coordination::decomposition::tests::outcomes_are_filtered_for_the_destination_model_before_a_model_sees_them` | **complete** |
| 29 | **H** — Value-based priority as a fairness weight in admission | `crates/refact-work-core/src/scheduler.rs` priority lane weighting | `priority_tests::an_urgent_run_outranks_a_lower_class_run_that_queued_first`; `priority_tests::the_class_is_assigned_or_defaulted_and_visible_in_the_projection` (C4) | `priority_tests::urgency_raises_no_engine_project_or_root_cap`; `priority_tests::urgency_never_cancels_or_unseats_a_running_lower_class_run`; `priority_tests::a_continuous_user_blocking_stream_leaves_speculative_its_minimum_share` (C4) | **complete** |

### Struck rows

These four rows were in the coverage ledger of the pre-amendment revision
(`git show 9ada9e7b8:docs/multi-agent-parity-plan.md` §13). The user's scope amendment —
recorded in the plan's "Scope amendment" paragraph — dropped M7, M8 and M9 in full. T-10
(`04df6d0b2`) removed the rows from the table rather than leaving them visible, so they
are restored here. **Struck is not met, and struck counts toward nothing.**

| Capability family (original wording) | Original delivery | Status |
|---|---|---|
| External harness provider registry, explicit capability rejection, SDK backend contracts | M7 | **struck** — scope amendment: "M7 (external harness, ACP client/server, experimental A2A) … dropped in full"; the hook work it contained survived and is row 19 |
| Cross-host catalog, remote sandbox/PTY/streaming and failure reconciliation | M8 | **struck** — scope amendment: the program "is now local-only"; M8 "remote/federated coordination" dropped. The plan's §6 gate now reads "Coordination stays inside one machine; no cross-machine successor is planned" |
| Ralph/evolution, immutable seed, successor single-flight, progress/oscillation, tournaments | M9 | **struck** — scope amendment: M9 "autonomous evolution" dropped in full |
| Google A2A interoperable experimental surface | M7–M8 | **struck** — dropped with M7/M8 |

Three clauses inside surviving rows were also struck rather than unmet, and this
certification did not test them:

- Row 1's original delivery read "M1; cross-process M3; **cross-host M8**". The cross-host
  clause is struck; cross-process presence within one machine is certified above.
- Row 7 originally read "… quota handling, **tree cost/turn/token/time accounting**". The
  amendment struck aggregate whole-tree budgets: "Capacity limits are not budgets."
  Row 7 is certified only for pressure control and capacity limits.
- Row 10's original delivery read "M2, M3, **M7**"; the external-backend leg is struck.

## Totals

| | |
|---|---|
| **complete** | **28** |
| **partial** | **1** (row 21) |
| **open** | **0** |
| **struck** | **4** (plus 3 struck clauses inside surviving rows) |
| Surviving rows assessed | 29 |
| Certified commit | `97ad671747783a1504c56420c1c6b70f975bb573` (`97ad67174`) |

## Residual gaps, stated precisely

### 21a — the metrics and the auditable explanation log have no reader

Plan §12 requires "structured metrics and traces for queue latency, permit counts,
provider pressure, unknown attribution, stale-write rejections, delivery lag/duplicates/
expiry, abandoned claims, recovery decisions, evidence cache validity, generation progress
and context payload size", and an "auditable explanation for automatic pause, denial,
retry, merge refusal and goal completion".

`src/coordination/metrics.rs` implements all of that as a recorder: `MetricsRecorder`,
`Explanation`, `record_queue_latency`, `record_stale_write_rejection`,
`record_delivery_lag`, `record_recovery_decision`, secret redaction, bounded series. Call
sites exist in `merge_gate.rs`, `leases.rs`, `outbox.rs`, `file_provenance.rs`,
`evidence.rs`, `llm/pressure.rs`, `progress_monitor.rs` and `causal.rs`.

**Nothing reads it.** `grep -rn "metrics::recorder()\|MetricsSnapshot" src` resolves only
inside `metrics.rs` itself plus one assertion in a `merge_gate.rs` test. O-2's commit
(`4d11a27bd`) touched no HTTP router, no tool and no GUI file. There is no route, tool or
panel from which a user or an agent can read a counter or an explanation record.

By the plan's own completion definition — "a passing unit test of an unused helper … is
not parity" — the metrics half of this row is **implemented and tested but not reachable**,
so the row is partial. The write path is genuinely wired; what is missing is exactly one
thing: a read surface.

### 21b — the TUI carries none of the shared vocabulary

Plan §12's first sentence requires the same concepts in "tools, HTTP, GUI, TUI and IDE
clients". `crates/refact-tui/src/app/surfaces/` contains `activity`, `board`, `browser`,
`history`, `settings` and `worktree`; none of them exposes queued/running/parked/stale,
waiting questions, capacity-limit reasons, evidence verdicts, attention consumers, the
causal trace or recovery actions. The only matches for "attention" in the TUI are two
English sentences in the worktree surface.

O-1 recorded this as a deliberate omission rather than an oversight — the TUI renders one
chat session and names its own `SessionState`, which the catalog already maps into the
shared vocabulary; pinning it would assert a synonym rather than forbid one. That is a
defensible design decision, but it is still an unmet clause of the row, and it is recorded
as unmet rather than argued away.

IDE clients are **not** a separate gap: VS Code and JetBrains host the same React GUI in a
webview, so the GUI surfaces certified in C6 are the IDE surfaces.

Neither 21a nor 21b is a defect in shipped behaviour, so neither produced a defect card.
21a is a missing surface and is the one piece of work this certification would ask for
before calling row 21 complete.

## Cross-check against the M1 acceptance ledger

Per the card, M1 was not re-derived; `docs/m1-acceptance-ledger.md` (refreshed by G-6 at
`1412c51b1`) is taken as the M1 authority. Two observations from this run:

1. **Its headline still holds at this commit.** G-6 certified 16 full / 0 partial with a
   6249-test binary; C1/C2 at `97ad67174` discover 6353 tests and pass 6328 with 0 failed,
   so nothing it relied on regressed between `1412c51b1` and `97ad67174`.

2. **One stale citation survived the G-6 refresh.** Row 13 of that ledger cites
   `agents::tool_authority::tests::canonical_aliases_and_transport_are_not_ambient_authority`,
   and its own E18 note claims to have corrected the name while printing the same string
   twice. The test that exists is
   `agents::tool_authority::tests::canonical_aliases_and_**mcp_**transport_are_not_ambient_authority`.
   C10 and C11 above are the executed proof: the cited name matches no test, the real name
   passes. This is a citation defect in the M1 document, not a coverage gap — the clause it
   supports is genuinely tested.

## Verdict

**The program is complete except for one surface.**

At `97ad671747783a1504c56420c1c6b70f975bb573`, 28 of the 29 surviving coverage-ledger
families satisfy all four columns of the plan's completion definition with executed
evidence; four families are struck by the user's scope amendment and are recorded as
struck; none is open.

The single partial row is "Human surfaces and diagnostic visibility", for two named
reasons: the metrics/explanation log is recorded but unreadable (21a), and the TUI exposes
none of the shared vocabulary (21b). Both are stated above with the grep that establishes
them.

Stated honestly rather than implied:

1. All evidence is unit- and integration-level on **Linux x86_64**. The engine's own CI
   matrix (`cargo test --release` on 7 targets) is what would close the Windows/macOS
   qualification; this document does not.
2. C6 is the whole GUI suite (388 files, 4515 tests), not a subset — but it is jsdom-level.
   No browser run is included in this certification.
3. Rows whose evidence lives outside `refact-lsp`'s lib target are marked `(C4)`; those
   suites do **not** run under `cargo test --lib` and need their own invocation, which is
   the reason C4 exists as a separate command.
4. No row was marked complete because a card report claimed it. Every cited test name was
   resolved in the C1 inventory before it was written down.
