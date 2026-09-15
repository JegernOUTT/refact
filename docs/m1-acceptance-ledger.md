# M1 acceptance ledger

Certification of the 16 deterministic acceptance scenarios in
`docs/multi-agent-parity-plan.md` §4.9.

Every row names tests that exist in the tree and an **executed** result. Verdicts are
strict: a test that exercises a helper in isolation while the production path stays
untouched counts as `none`; a test that covers one half of a scenario counts as
`partial`, and the missing half is named.

No production code was changed to produce this ledger.

## Environment

| | |
|---|---|
| Worktree | `/home/svakhreev/.cache/refact/worktrees/a5ef23b532caf0f6/99f1fa34-06e9-4999-b9b1-69f9b86f119d` |
| Branch | `refact/task/d3bd0b85-.../card/C-1/92c00720` |
| HEAD | `20b01b767` — feat(m2): human attention budget (T-14) |
| Date | 2026-09-15 |
| Engine test binary | `cargo test --lib --no-run` → exit 0 (cold build, 20m45s) |
| Total lib tests discovered | 5941 |

## Executed test runs

All commands below were run from `refact-agent/engine` (GUI runs from
`refact-agent/gui`) and all exited 0.

| # | Command | Result |
|---|---|---|
| E1 | `cargo test --lib -- coordination::` | ok. **89 passed**; 0 failed; 0 ignored; 5852 filtered out; 0.59s |
| E2 | `cargo test --lib -- agents::` | ok. **312 passed**; 0 failed; 0 ignored; 5629 filtered out; 32.00s |
| E3 | `cargo test --lib -- tasks:: --quiet` | ok. **89 passed**; 0 failed; 0 ignored; 5852 filtered out; 0.52s |
| E4 | `cargo test --lib -- file_activity:: --quiet` | ok. **6 passed**; 0 failed; 5935 filtered out; 0.08s |
| E5 | `cargo test --lib -- tools::file_edit:: --quiet` | ok. **47 passed**; 0 failed; 5894 filtered out; 0.23s |
| E6 | `cargo test --lib -- cooperating_processes` | ok. **3 passed**; 0 failed; 5938 filtered out; 0.24s |
| E7 | `cargo test --lib -- runtime_settings:: --quiet` | ok. **16 passed**; 0 failed; 5925 filtered out; 0.03s |
| E8 | `cargo test --lib -- chat::goal_monitor:: --quiet` | ok. **34 passed**; 0 failed; 5907 filtered out; 0.01s |
| E9 | `cargo test --lib -- http::routers::v1::work:: --quiet` | ok. **3 passed**; 0 failed; 5938 filtered out; 0.26s |
| E10 | `cargo test --lib -- clamp_goal_snapshot_for_load restored_legacy_default_budget_heals_terminal_status zero_budget_limits_are_disabled default_budget_never_exhausts budget_exhaustion_by_turns_tokens_minutes_and_no_progress` | ok. **8 passed**; 0 failed; 5933 filtered out |
| E11 | `cargo test --lib -- tools::tool_spawn_ab:: tools::tool_background_agents:: tools::tool_work_overview:: tools::tool_work_inspect:: --quiet` | ok. **44 passed**; 0 failed; 5897 filtered out; 1.34s |
| E12 | `cargo test --lib -- background_tasks:: more_than_thirty_two_parallel_safe_tools_start_together subchat_worktree_tool_execution_context_inherits_thread_scope spawn_and_wait_times_out_when_runner_hangs restart_recovery_interrupts_active_records legacy_record_without_introspection_fields_deserializes overlap_warning_normalizes_equivalent_paths` | ok. **7 passed**; 0 failed; 5934 filtered out; 0.49s |
| E13 | `cargo test -p refact-work-core --lib --quiet` | ok. **27 passed**; 0 failed; 0 filtered out |
| E14 | `cargo test -p refact-work-core --test scheduler_tests --quiet` | ok. **29 passed**; 0 failed; 0 filtered out; 0.15s |
| E15 | `cargo test -p refact-worktrees --lib -- worktree_registry_list_prunes_missing_records worktree_registry_list_keeps_referenced_missing_records list_worktrees_prunes_missing_record_even_with_stale_references worktree_registry_create_cleans_branch_after_worktree_failure worktree_registry_create_list_get_delete_normal_and_stale_worktree` | ok. **5 passed**; 0 failed; 82 filtered out; 0.17s |
| E16 | `node_modules/.bin/vitest run src/features/Workspace/WorkPanel src/services/refact/workSse.test.ts src/services/refact/workSubscription.test.ts src/services/refact/workTypes.test.ts src/components/Tools/AgenticTool/WorkToolView.test.tsx src/components/Tools/AgenticTool/OverviewToolView.test.tsx src/components/Tools/AgenticTool/InspectToolView.test.tsx src/features/Performance/CoordinationSettingsPanel.test.tsx` | **Test Files 8 passed (8), Tests 39 passed (39)**; 11.74s |

`refact-work-core`'s scheduler suite (E14) lives in `tests/`, not `src/`, so
`cargo test --lib` does **not** run it. It needs its own invocation.

## The ledger

| # | Scenario | Test(s) — module::path | Covers it? | Evidence |
|---|---|---|---|---|
| 1 | New-chat awareness | `coordination::service_tests::shared_projection_paginates_without_gaps_and_rejects_changed_state`; `coordination::service_tests::board_catalog_bootstraps_once_and_guarded_writes_and_deletion_update_it`; `coordination::service_tests::inspector_scopes_files_and_pages_worktree_inventory`; `coordination::service_tests::same_relative_path_in_a_separate_checkout_is_an_advisory_overlap_only`; `coordination::catalog::tests::unbound_and_foreign_sessions_are_not_adopted_by_selected_project`; `agents::spawn_lifecycle_tests::queued_spawn_is_discoverable_only_with_permitted_launch_provenance` | **full** | E1 → ok. 89 passed; 0 failed. E2 → ok. 312 passed; 0 failed |
| 2 | Privacy isolation | `coordination::service_tests::denied_titles_paths_counts_and_changes_do_not_affect_visible_pages`; `…::changed_destination_invalidates_cursor_and_policy_failure_is_not_empty_success`; `…::board_model_disclosure_carries_writer_provenance_and_denied_counts_stay_hidden`; `…::inspection_keeps_operator_degradation_but_models_cannot_probe_hidden_source_failures`; `coordination::privacy_guard::tests::*` (4); `coordination::history::tests::legacy_or_denied_history_never_discloses_to_a_model`; `coordination::capsule_tests::model_policy_changes_force_recheck_inside_debounce_and_never_disclose_denied_peer`; `coordination::paging::tests::cursor_binds_project_principal_policy_model_and_filter` | **partial** — title, path, progress, count, capsule, pagination and model-switch re-evaluation are all covered. The **subscription (SSE) leg is not**: `http::routers::v1::work::subscribe` has no test asserting a denied peer never reaches an emitted frame | E1 → ok. 89 passed; 0 failed. E9 → ok. 3 passed; 0 failed |
| 3 | No history scan on each turn | `coordination::history::tests::thousands_of_indexed_histories_are_projected_without_any_body_read_per_turn` (2000 rows, all bodies overwritten with invalid JSON, then asserts `listing_counters_for_test().index_reads` and `.directory_scans` are unchanged across 5 turns); `…::historical_rows_follow_the_index_without_per_request_body_reads_and_live_rows_win`; `…::one_malformed_index_message_invalidates_all_provenance`; `coordination::catalog::tests::message_events_maintain_provenance_without_a_subscriber_or_catalog_body_scan` | **full** — real spy counters, real `build_catalog` hot path | E1 → ok. 89 passed; 0 failed |
| 4 | Real admission | `agents::spawn_lifecycle_tests::twelve_runs_across_both_runtimes_admit_four_queue_eight_and_rotate_roots` (6 chat-runtime spawns + 3 board A/B card pairs = 12 runs at cap 4; asserts `active == 4`, `pending == 8`, no facade `created`/`started`, no `cache/worktrees` directory, cards unclaimed, then alternating release proves both roots get turns); `tasks::admission::tests::board_and_chat_descendants_share_project_and_root_capacity`; `tasks::admission::tests::reserve_returns_promptly_even_when_full`; `scheduler_tests::root_round_robin_prevents_one_roots_backlog_from_monopolizing_releases`; `scheduler_tests::configurable_root_quantum_is_bounded_and_preserves_fifo_in_lanes`; `scheduler_tests::repeated_aged_resumes_cannot_starve_fresh_work_in_the_same_root` | **full** | E2 → ok. 312 passed; 0 failed. E3 → ok. 89 passed; 0 failed. E14 → ok. 29 passed; 0 failed |
| 5 | Nested waits | `agents::spawn_lifecycle_tests::parent_parks_its_only_permit_during_agent_wait_and_reacquires_before_continuing` (cap 1, real `execute_tools` → `agent_wait`, asserts parent `Parked` while child runs, then `Running` + `holds_permit`); `tools::tool_background_agents::tests::agent_wait_timeout_returns_status_with_prefix`; `…::agent_wait_any_cancellation_racing_a_timeout_settles_exactly_once`; `…::agent_wait_is_a_recognized_runner_wait_so_no_permit_is_held`; `agents::approval::tests::child_command_approval_parks_without_side_effects_then_resumes_with_exact_binding`; `…::board_tool_loop_relays_exact_command_and_executes_only_after_parent_decision` (board child ↔ chat parent = mixed runtime); `…::both_runtimes_reject_changed_environment_and_cancelled_execution`; `agents::execution::tests::suspend_waits_for_non_wait_operations_to_drain`; `…::pending_wait_blocks_fresh_operations`; `…::batch_registers_unpolled_operations_before_wait_can_park`; `chat::tools::tests::more_than_thirty_two_parallel_safe_tools_start_together` | **full** | E2 → ok. 312 passed; 0 failed. E11 → ok. 44 passed; 0 failed. E12 → ok. 7 passed; 0 failed |
| 6 | Idempotency / cleanup | `tasks::admission::tests::duplicate_reserve_for_same_agent_chat_id_is_rejected`; `…::cancel_queued_drops_reservation_without_touching_active_permits`; `…::dropped_acquisition_removes_adapter_and_scheduler_reservations`; `…::abort_pending_setup_joins_and_removes_acquiring_entry`; `…::live_setup_cannot_be_overwritten_or_reused_after_early_end`; `agents::spawn_lifecycle_tests::panicking_child_runner_releases_capacity_and_terminalizes_honestly`; `tools::tool_spawn_ab::tests::ab_full_capacity_returns_queued_and_variants_start_independently`; `…::ab_second_reservation_failure_does_not_claim_card_or_create_worktrees`; `…::ab_cancel_queued_setup_creates_no_worktree_and_records_failed_outcome`; `…::dropped_ab_claim_caller_cannot_strand_reservations_or_launch_workers`; `coordination::service_tests::missing_or_timed_out_board_writer_never_changes_persisted_board`; `refact-worktrees service::worktree_registry_tests::worktree_registry_create_cleans_branch_after_worktree_failure`; `scheduler_tests::queue_full_is_rejected_explicitly`; `scheduler_tests::outstanding_full_is_rejected_explicitly`; `scheduler_tests::panicking_setup_releases_permit_and_ticket_exactly_once` | **full** — duplicate spawn, queue-full, worktree failure, panic, cancelled queue and failed board write all present, each asserting no leaked permit / duplicate worker / orphaned claim | E1, E2, E3, E11, E14, E15 all ok; 0 failed |
| 7 | Intentional waiting | `agents::monitor::tests::monitor_never_flags_parked_or_queued_waiting_as_stalled` (real parked `RunExecution` + queued record aged 25 min, then asserts the 20-minute stall banner appears only after `resume()`); `chat::goal_monitor::tests::goal_monitor_ignores_paused_stopped_transferred_and_waiting_states`; `chat::goal_monitor::tests::goal_monitor_does_not_fire_when_user_message_queued`; `agents::spawn_lifecycle_tests::queued_resume_revalidates_scope_policy_history_and_pause_without_dispatch` (`"pause"` case: a paused goal survives a queued resume and the runner is never entered) | **partial** — queued, parked and paused are covered. **Not covered:** `BgAgentStatus::WaitingForApproval` is absent from the stall-monitor test, and no test shows that a *parked or queued execution* suppresses repeated goal nudges (the goal monitor only skips on `SessionState::{Paused, WaitingUserInput, WaitingIde}`, which a parked run does not enter) | E2 → ok. 312 passed; 0 failed. E8 → ok. 34 passed; 0 failed |
| 8 | Visible-read protection | `tools::file_edit::auxiliary::tests::…::every_text_mutation_family_rejects_a_stale_model_visible_read` (iterates every text mutation kind + `apply_patch`, each must error); `…::whole_replacement_requires_a_complete_visible_read_and_new_file_race_has_one_winner` (partial reads + new-file race); `…::write_file_rejects_buffer_disk_divergence`; `…::write_file_rechecks_preimage_after_waiting_for_guard`; `tools::file_edit::move_file::tests::stale_source_and_destination_refuse_without_modifying_either_file` (rename); `…::hardlink_alias_move_is_rejected_without_destroying_either_link`; `file_activity::tests::directory_mutation_refuses_when_any_descendant_visible_read_is_stale` (delete/tree); `tools::file_edit::tool_undo_textdoc::tests::external_edit_cannot_be_overwritten_by_undo_even_without_read_stamp` (undo); `tools::file_edit::tool_apply_patch::tests::patch_plan_rechecks_every_preimage_under_the_complete_lock_set` | **full** — every clause of the scenario (all mutation families, partial reads, new-file races, rename/delete, undo, buffer/disk divergence) has a dedicated assertion | E4 → ok. 6 passed; 0 failed. E5 → ok. 47 passed; 0 failed |
| 9 | Cooperating processes | `file_activity::tests::cooperating_processes_serialize_and_refuse_a_stale_visible_read` + helper `…::subprocess_file_writer`; `tools::file_edit::auxiliary::tests::cooperating_processes_cannot_both_apply_one_stale_edit` + helper `…::subprocess_stale_edit`; `tasks::storage::tests::cooperating_processes_cannot_both_claim_one_board_card` + helper `…::subprocess_board_claim`; `tasks::board_lock::tests::foreign_holder_blocks_acquisition_until_released`; `tools::file_edit::move_file::tests::competing_moves_have_one_winner_and_accurate_binary_observations` | **full** — real OS subprocesses (`std::env::current_exe()`), real file barriers, temporary repos only; each asserts exactly one winner and that the loser is refused as stale rather than overwriting | E6 → ok. 3 passed; 0 failed. E3 → ok. 89 passed; 0 failed. E5 → ok. 47 passed; 0 failed |
| 10 | Worktree distinction | `coordination::service_tests::same_relative_path_in_a_separate_checkout_is_an_advisory_overlap_only` (same `src/lib.rs` in two checkouts → two rows + `integration_overlaps == ["trajectory:peer"]`, i.e. advisory; the physical path query returns one row and no overlap); `coordination::capsule_render::tests::peer_checkout_paths_stay_relative_and_separate_checkouts_do_not_conflict`; `tasks::board_lock::tests::symlink_alias_maps_to_the_same_lock`; `tasks::board_lock::tests::distinct_lock_files_do_not_block_each_other`; `tools::file_edit::move_file::tests::hardlink_alias_move_is_rejected_without_destroying_either_link`; `coordination::scope::tests::absent_leaf_under_foreign_symlink_cannot_bypass_scope`; `agents::path_scope::tests::a_symlink_out_of_the_scope_cannot_be_used_to_escape_it`; `…::a_hard_link_is_the_same_file_identity_as_its_out_of_scope_original` | **full** — advisory-vs-lock distinction and alias-cannot-bypass-serialization are both directly asserted | E1, E2, E3, E5 all ok; 0 failed |
| 11 | Freshness and restart | `coordination::service_tests::board_catalog_bootstraps_once_and_guarded_writes_and_deletion_update_it` (a disk-only board row reports `WorkStateTag::Interrupted` and `stale == true`, never live); `agents::tests::restart_recovery_interrupts_active_records`; `agents::spawn_lifecycle_tests::killed_engine_process_leaves_a_recoverable_child_without_replay` (real SIGKILL of a child engine process); `…::cold_restart_resumes_authoritative_child_without_replaying_unknown_effects`; `coordination::history::tests::historical_rows_follow_the_index_without_per_request_body_reads_and_live_rows_win`; `refact-worktrees service::worktree_registry_tests::worktree_registry_list_keeps_referenced_missing_records` / `…::worktree_registry_list_prunes_missing_records` / `…::list_worktrees_prunes_missing_record_even_with_stale_references` | **partial** — "disk-only running row is not live" and "restart reports interrupted honestly" are fully covered. **Not covered:** the removed-worktree clause is only proven at the `WorktreeService` registry level; no test asserts that a *coordination* projection (`overview` / `inspect(ProjectWorktrees)`) reports a deleted worktree as historical rather than active | E1, E2, E15 all ok; 0 failed |
| 12 | UX | Engine: `http::routers::v1::work::tests::gui_route_and_model_tool_agree_on_the_same_projection` (asserts the `/work` GUI route and `tool_work_overview` return identical `total`, `coverage` and `(id, state, title, freshness)` tuples); `…::auth_and_trusted_scope_are_checked_before_discovery`; `…::no_token_mode_requires_real_loopback_transport`. GUI: `WorkPanel.test.tsx` — unavailable (403 `scope_denied`), honest empty, parked/attention/historical incl. `freshness: "stale"`, queued+running, one/many/paged with truthful totals, cache hidden after a visibility failure, refresh-reconnect; `workSubscription.test.ts` — bearer auth, split CRLF, duplicate suppression, **reset after a gap**, buffer cap, clean unmount, timeout-once; `workTypes.test.ts` — requires a fresh snapshot after gaps/restart/project change; `workSse.test.ts` — frame bounding, heartbeat handling, fail-closed | **partial** — zero/one/many, queued, parked, stale, unavailable and SSE gap/reconnect all render and are asserted; GUI↔tool agreement is asserted. **Not covered:** `overlap` and `withheld_rows` ("denied") have **no rendering path at all** in the GUI work surfaces (`grep -rn "overlap\|withheld" src/components/Tools/AgenticTool/ src/features/Workspace/WorkPanel/` → no matches), so two of the scenario's listed states are unrendered, not merely untested | E9 → ok. 3 passed; 0 failed. E16 → Test Files 8 passed (8), Tests 39 passed (39) |
| 13 | Compatibility | `agents::tests::legacy_record_without_introspection_fields_deserializes`; `chat::trajectories::tests::clamp_goal_snapshot_for_load_migrates_implicit_legacy_budget` / `…_preserves_explicit_legacy_budget`; `tasks::index::tests::missing_index_rebuilds_from_meta_files_and_creates_index` / `…::corrupt_index_rebuilds_from_disk`; `agents::spawn_lifecycle_tests::legacy_open_tool_authority_migrates_to_the_concrete_parent_ceiling_on_resume`; `agents::tool_authority::tests::registered_parent_ceiling_is_unavailable_legacy_or_concrete`; `…::canonical_aliases_and_mcp_transport_are_not_ambient_authority`; `tools::tool_background_agents::tests::parent_scoping_is_enforced_for_all_agent_tools`; `agents::push::tests::legacy_delegate_push_message_uses_unified_noun_and_includes_edited_files`; `agents::path_scope::tests::declared_scope_says_it_is_not_enforced_until_strict`; `…::refusal_text_names_the_scope_and_never_claims_unimplemented_enforcement`; `tools::file_edit::auxiliary::tests::…::non_strict_target_files_are_reported_as_declared_intent_only`; `runtime_settings::tests::old_yaml_without_new_keys_loads_with_new_defaults` | **full** — legacy load, root-scoped tool behaviour, old aliases and the "scope descriptions do not overclaim enforcement" clause each have a direct assertion | E2, E3, E5, E7, E10, E11, E12 all ok; 0 failed |
| 14 | Shutdown | `background_tasks::tests::abort_cancels_every_owned_task_and_joins_before_returning`; `tasks::admission::tests::shutdown_joins_setup_after_holder_removed_and_closes_only_its_app`; `…::cooperative_setup_cancel_timeout_retains_handle_and_capacity`; `…::dropping_cancel_waiter_keeps_setup_visible_until_joined`; `coordination::deadlock::tests::the_detector_is_cancelled_and_joined_on_shutdown` (explicitly asserts joined, not detached); `agents::execution::tests::dropped_shutdown_waiter_retains_tasks_and_waits_for_all_cleanup`; `…::registration_racing_shutdown_has_no_unowned_task`; `agents::spawn_lifecycle_tests::unacknowledged_resume_cancellation_never_dispatches_or_changes_history` (both `shutdown = false/true`) | **partial** — "all waiting tasks wake/cancel" and "no monitor is detached" are fully covered. **Not covered:** "both background-task wiring paths are covered" has no test — nothing asserts that `background_tasks::start_background_tasks` *and* `background_tasks::start_full_soak_background_tasks` both spawn `agents::monitor::run_background_agent_monitor` (which is what transitively starts `DeadlockDetector`). A future edit could drop it from one path silently | E1, E2, E3, E12 all ok; 0 failed |
| 15 | Structural board authority | `tasks::storage::tests::board_persistence_has_no_writer_outside_the_guarded_transaction` (source-call-site inventory); `…::board_write_rejects_lock_for_another_task_dir`; `…::board_write_rejects_planner_id_lock`; `…::concurrent_updates_do_not_lose_cards`; `…::save_board_checked_rejects_stale_revision`; `…::create_board_refuses_to_clobber_existing_board`; `…::concurrent_claims_leave_exactly_one_winner_and_one_board_revision`; `…::concurrent_planner_chat_id_reservations_are_unique` (8 concurrent allocations, all distinct); `…::planner_chat_ids_are_reserved_durably_without_created_chats`; `tasks::board_lock::tests::symlink_alias_maps_to_the_same_lock`; `coordination::service_tests::known_board_writer_waits_for_provenance_before_taking_the_board_lock` | **partial** — raw-persistence inaccessibility, the call-site inventory and concurrent planner-ID uniqueness are covered. **Not covered:** the scenario's "under canonical/**symlink-alias** task paths" qualifier. `concurrent_planner_chat_id_reservations_are_unique` uses one canonical path only; `symlink_alias_maps_to_the_same_lock` proves the lock domain collapses aliases but never allocates planner IDs concurrently through both a canonical and an aliased task directory | E1 → ok. 89 passed; 0 failed. E3 → ok. 89 passed; 0 failed |
| 16 | Settings and depth | Precedence: `runtime_settings::tests::a_sparse_project_override_resolves_over_the_global_settings`; `…::an_empty_project_override_inherits_every_global_value`; `…::a_partial_coordination_block_fills_the_rest_from_defaults`; `…::yaml_without_a_coordination_block_keeps_every_coordination_default`; `…::a_missing_or_empty_project_override_file_reads_as_no_overrides`. Invalid values: `…::out_of_range_and_invariant_violating_coordination_values_are_rejected`; `…::project_overrides_cannot_introduce_unknown_or_out_of_range_values`; `…::the_bounds_table_covers_exactly_the_serialized_coordination_leaves`; `…::every_default_sits_inside_its_published_range`; `…::root_weights_are_validated_in_global_and_sparse_project_settings`; `scheduler_tests::configure_rejects_invalid_settings_and_keeps_previous`. Live cap reduction: `scheduler_tests::cap_reduction_never_evicts_running_work_but_stops_new_admissions`; `scheduler_tests::configure_raises_cap_and_wakes_waiters`; `coordination::board_index::tests::aggregate_limits_fail_closed_and_live_reduction_never_discards_accepted_rows`; `coordination::provenance::tests::byte_caps_and_live_reduction_never_return_truncated_clearance`. Queue/resident bounds: `scheduler_tests::queue_full_is_rejected_explicitly`; `…::outstanding_full_is_rejected_explicitly`; `…::parked_residency_does_not_consume_new_queue_admission`; `…::depth_limit_and_disabled_scheduler_reject_reservations`. Default nesting: `runtime_settings::tests::coordination_defaults_are_generous_and_survive_a_yaml_round_trip` pins `max_subchat_depth == 5`. Goal budgets: `chat::trajectories::tests::clamp_goal_snapshot_for_load_migrates_implicit_legacy_budget` / `…_preserves_explicit_legacy_budget`; `chat::session::tests::restored_legacy_default_budget_heals_terminal_status`; `chat::session::tests::goal_budget::zero_budget_limits_are_disabled`; `…::default_budget_never_exhausts`; `…::budget_exhaustion_by_turns_tokens_minutes_and_no_progress` | **full** — every clause, including "explicit finite and unlimited zero/None goal budgets both survive migration", has a direct assertion | E1, E7, E10, E14 all ok; 0 failed |

## Scenarios not fully covered

Five scenarios are `partial`. None is a fabricated gap — each is a clause of §4.9 with
no assertion behind it.

### 2 — Privacy isolation: the subscription leg
`http::routers::v1::work::subscribe` (the SSE route) has no test. The three existing
router tests cover `GET /work` only.
**Missing test:** in `src/http/routers/v1/work.rs::tests`, drive `subscribe` with a
caller whose model is privacy-incompatible with one session, read the emitted frames,
and assert the denied peer's title/path never appears and that the row count matches
the cleared projection — plus that flipping `caller.model` mid-stream re-evaluates
disclosure on the next frame. Roughly the shape of
`denied_titles_paths_counts_and_changes_do_not_affect_visible_pages`, applied to the
stream instead of the page.

### 7 — Intentional waiting: approval state and nudge suppression
Two clauses are unasserted.
**Missing test A:** extend `agents::monitor::tests::monitor_never_flags_parked_or_queued_waiting_as_stalled`
(or add a sibling) with a third record in `BgAgentStatus::WaitingForApproval`, aged past
the stall threshold, asserting it is neither flagged with the "20+ minutes" banner nor
terminalised.
**Missing test B:** a `chat::goal_monitor` test proving a session whose execution is
`ExecutionState::Parked` or whose admission is still `Queued` does not receive repeated
goal nudges. Today `goal_monitor` only skips on `SessionState::{Paused, WaitingUserInput,
WaitingIde}`; a parked run is in none of those, so this may be a real behaviour gap and
not only a test gap — worth confirming before writing the test.

### 11 — Freshness and restart: removed worktrees in the coordination projection
**Missing test:** in `coordination::service_tests`, register a worktree, record activity
against it, delete its root from disk, then assert `inspect(InspectSelector::ProjectWorktrees)`
and `overview` report it as historical/stale (or omit it from live rows) rather than as an
active checkout. The existing proof lives one layer down in `WorktreeService::list`.

### 12 — UX: overlap and denied states do not render
`overlap` and `withheld_rows` have no rendering code in
`src/features/Workspace/WorkPanel/` or `src/components/Tools/AgenticTool/`. This is a GUI
implementation gap, not just a missing test.
**Missing work:** render integration overlaps and the withheld-row count in `WorkPanel`
(and/or `WorkToolView`), then add `WorkPanel.test.tsx` cases for a row carrying
`integration_overlaps` and for a page with `withheld_rows > 0`.

### 14 — Shutdown: both background-task wiring paths
**Missing test:** in `src/background_tasks.rs::tests`, assert that
`start_background_tasks` and `start_full_soak_background_tasks` both wire the
coordination-critical tasks (at minimum `agents::monitor::run_background_agent_monitor`,
which owns `DeadlockDetector`, plus `chat::start_agent_monitor` and
`chat::start_goal_monitor`). The cheapest honest form is a shared const list of required
task names that both constructors are built from, asserted once — that needs a small
production refactor, so it should be carded rather than bolted on here.

### 15 — Structural board authority: aliased planner-ID allocation
**Missing test:** in `tasks::storage::tests`, create a task directory plus a symlink alias
to it, spawn concurrent `next_planner_chat_id` calls addressing the task through *both*
the canonical and the aliased path, and assert every returned ID is still unique. The
lock-domain half already exists (`board_lock::tests::symlink_alias_maps_to_the_same_lock`);
the allocation half does not.

## Verdict

**M1 is not yet certifiable as complete.** 11 of 16 scenarios are fully covered by tests
that exercise real production paths and were executed green. 5 are partial, with 6 named
missing tests and 1 named missing GUI feature (scenario 12's overlap/denied rendering).

No scenario is `none`. Nothing in the ledger rests on a test that only exercises an
unused helper.
