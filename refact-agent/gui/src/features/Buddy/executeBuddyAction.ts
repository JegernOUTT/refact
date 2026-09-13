import type { AppDispatch } from "../../app/store";
import { push } from "../Pages/pagesSlice";
import {
  clearActiveSpeech,
  dismissBuddySuggestion,
  dismissRuntimeEvent,
  setActiveSpeech,
  setBuddySnapshot,
} from "./buddySlice";
import {
  openChatInModeAndStart,
  startBuddyInvestigation,
} from "../Chat/Thread";
import { isValidSetupMode } from "../Setup/setupModes";
import type {
  BuddyControl,
  BuddyPage,
  BuddyPulse,
  BuddySpeechItem,
  DraftKind,
  WorktreePulse,
} from "./types";
import type { DiagnosticContext } from "./types";
import { buddyApi } from "../../services/refact/buddy";
import type { ActionSpec } from "./operatorTypes";
import {
  buddyOperatorApi,
  buddyOperatorIdentity,
} from "../../services/refact/buddyOperator";

// Exact server catalog pairs, not an open-ended "anything mutating" dispatcher.
const RESOURCE_HANDLERS = [
  ["settings", "settings_trajectory_patch", "settings"],
  ["buddy_settings", "buddy_settings_patch", "settings"],
  ["default_models", "default_model_select", "default_models"],
  ["providers", "provider_enable_custom_model", "provider"],
  ["memory", "memory_lifecycle", "memory"],
  ["history", "export_backup", "trajectory"],
  ["project_code", "checkpoint_isolated_candidate", "worktree"],
  ["candidate_landing", "land_reviewed_source", "worktree"],
  ["candidate_artifact", "stage_exact_artifact", "candidate_artifact"],
  [
    "candidate_artifact_cleanup",
    "remove_exact_orphan_artifacts",
    "candidate_artifact_cleanup",
  ],
  ["hooks", "hook_project_apply", "hooks"],
  ["integrations", "mcp_project_apply", "mcp"],
  ["extensions", "markdown_project_apply", "native_markdown"],
  [
    "project_customization_file",
    "customization_file_apply",
    "project_customization_file",
  ],
] as const;

export function buddyRegisteredActionHandler(
  spec: ActionSpec,
): "resource" | "saved_evidence" | "unsupported" {
  if (spec.schema_version !== 1) return "unsupported";
  if (
    spec.adapter === "operator.saved_evidence" &&
    spec.kind === "check_saved_evidence" &&
    spec.resource.resource_kind === "saved_report" &&
    spec.authority_class === "read_only" &&
    spec.confirmation === "none"
  )
    return "saved_evidence";
  if (
    spec.authority_class === "mutating" &&
    spec.confirmation === "user_confirmation_required" &&
    RESOURCE_HANDLERS.some(
      ([adapter, kind, resource]) =>
        spec.adapter === adapter &&
        spec.kind === kind &&
        spec.resource.resource_kind === resource,
    )
  )
    return "resource";
  return "unsupported";
}

const LEGACY_SETUP_ACTION_TO_MODE = {
  open_setup_mcp: "setup_mcp",
  open_setup_skills: "setup_skills",
  open_setup_commands: "setup_commands",
  open_setup_agents_md: "setup_agents_md",
  open_setup_subagents: "setup_subagents",
} as const;

type LegacySetupAction = keyof typeof LEGACY_SETUP_ACTION_TO_MODE;

export interface BuddyActionContext {
  triggerText: string;
  triggerSource:
    | "thread"
    | "runtime"
    | "diagnostic"
    | "suggestion"
    | "frontend";
  sourceChatId?: string;
  diagnostic?: DiagnosticContext | null;
  suggestionId?: string;
}

export type BuddyActionOutcome =
  | { status: "ok" }
  | { status: "unsupported"; message: string }
  | { status: "failed"; message: string };

const ACTION_OK: BuddyActionOutcome = { status: "ok" };

function actionUnavailable(
  dispatch: AppDispatch,
  message: string,
): BuddyActionOutcome {
  dispatch(
    setActiveSpeech(buddyActionFeedbackSpeech("buddy-action-error", message)),
  );
  return { status: "unsupported", message };
}

function describeBuddyActionError(error: unknown): string {
  if (error instanceof Error && error.message) return error.message;
  if (typeof error === "string" && error) return error;
  if (typeof error === "object" && error !== null) {
    const record = error as Record<string, unknown>;
    for (const key of ["detail", "message", "error"]) {
      const value = record[key];
      if (typeof value === "string" && value) return value;
    }
    const data = record.data;
    if (typeof data === "string" && data) return data;
    if (typeof data === "object" && data !== null) {
      const nested = (data as Record<string, unknown>).detail;
      if (typeof nested === "string" && nested) return nested;
    }
    if (
      typeof record.status === "number" ||
      typeof record.status === "string"
    ) {
      return `request failed (${String(record.status)})`;
    }
  }
  return "the request failed";
}

export function buddyActionFeedbackSpeech(
  id: string,
  message: string,
): BuddySpeechItem {
  return {
    id,
    text: message,
    mood: "confused",
    scope: "action_feedback",
    persistent: false,
    ttl_seconds: 12,
    dedupe_key: id,
    created_at: new Date().toISOString(),
    controls: [
      {
        id: `${id}-dismiss`,
        label: "OK",
        action: "dismiss_speech",
        style: "secondary",
      },
    ],
    speech_intent: "error_alert",
  };
}

const WORKTREE_PULSE_FIELDS: [keyof WorktreePulse, string][] = [
  ["total", "Worktrees total"],
  ["total_registered", "Registered"],
  ["total_discovered", "Discovered"],
  ["clean", "Clean"],
  ["dirty", "Dirty"],
  ["unknown", "Unknown state"],
  ["stale", "Stale"],
  ["conflicted", "Conflicted"],
  ["shared", "Shared"],
  ["abandoned_clean", "Clean abandoned"],
  ["changed_files", "Changed files"],
  ["additions", "Line additions"],
  ["deletions", "Line deletions"],
  ["missing_registry_paths", "Missing registry paths"],
  ["unregistered_cache_dirs", "Unregistered cache dirs"],
  ["merged_branches", "Merged branches"],
  ["newest_age_hours", "Newest age (hours)"],
  ["oldest_age_hours", "Oldest age (hours)"],
  ["disk_usage_bytes", "Disk usage (bytes)"],
];

export function renderWorktreePulseReport(pulse: BuddyPulse): string {
  const worktrees = pulse.worktrees as Partial<WorktreePulse> | undefined;
  const rows = WORKTREE_PULSE_FIELDS.flatMap(([key, label]) => {
    const value = worktrees?.[key];
    return typeof value === "number" && Number.isFinite(value)
      ? [`- ${label}: ${value}`]
      : [];
  });
  const header = ["# Worktrees pulse report"];
  if (typeof pulse.generated_at === "string" && pulse.generated_at) {
    header.push(`Pulse generated at ${pulse.generated_at}.`);
  }
  header.push(
    rows.length > 0
      ? rows.join("\n")
      : "The pulse reported no worktree counters.",
  );
  return header.join("\n\n");
}

const SUGGESTION_PRESERVING_ACTIONS = new Set([
  "dismiss",
  "dismiss_speech",
  "dismiss_suggestion",
  "dismiss_runtime_event",
  "accept_quest",
  "reroll_personality",
]);

/**
 * Acting on a suggestion resolves it. Without this the underlying
 * BuddySuggestion stays undismissed and the same bubble is rebuilt on the
 * next render, so the user sees the nag again seconds after acting on it.
 */
export function buddyActionResolvesSuggestion(action: string): boolean {
  if (SUGGESTION_PRESERVING_ACTIONS.has(action)) return false;
  return !action.startsWith("care_");
}

export async function resolveBuddySuggestion(
  suggestionId: string,
  dispatch: AppDispatch,
): Promise<void> {
  dispatch(dismissBuddySuggestion(suggestionId));
  try {
    await dispatch(
      buddyApi.endpoints.dismissBuddySuggestion.initiate(suggestionId),
    ).unwrap();
  } catch {
    // local dismissal already hid the suggestion; server sync is best-effort
  }
}

async function createWorktreesPulseReport(
  dispatch: AppDispatch,
  isCurrent: () => boolean,
): Promise<BuddyActionOutcome> {
  try {
    const settings = await dispatch(
      buddyApi.endpoints.getBuddySettings.initiate(undefined, {
        subscribe: false,
        forceRefetch: true,
      }),
    ).unwrap();
    if (!isCurrent()) return ACTION_OK;
    if (!settings.enabled || settings.autonomy_level === "read_only") {
      return actionUnavailable(
        dispatch,
        "Buddy must be enabled and allowed to create drafts before creating a pulse report.",
      );
    }
    const pulse = await dispatch(
      buddyApi.endpoints.getPulse.initiate(undefined, {
        subscribe: false,
        forceRefetch: true,
      }),
    ).unwrap();
    if (!isCurrent()) return ACTION_OK;
    const draft = await dispatch(
      buddyApi.endpoints.createPulseReportDraft.initiate({
        title: "Worktrees pulse report",
        yaml_or_json: renderWorktreePulseReport(pulse),
        explanation: "Worktree counters read from the current Buddy pulse.",
      }),
    ).unwrap();
    if (!isCurrent()) return ACTION_OK;
    if (typeof draft.id !== "string" || !draft.id.trim()) {
      throw new Error("The server did not return a report draft ID.");
    }
    routeDraftByKind(
      { draft_kind: "pulse_report", draft_id: draft.id },
      dispatch,
    );
    dispatch(clearActiveSpeech());
    return ACTION_OK;
  } catch (error) {
    if (!isCurrent()) return ACTION_OK;
    const message = `Could not create the worktrees pulse report: ${describeBuddyActionError(
      error,
    )}`;
    dispatch(
      setActiveSpeech(
        buddyActionFeedbackSpeech("worktrees-pulse-error", message),
      ),
    );
    return { status: "failed", message };
  }
}

/**
 * Central executor for all Buddy control actions.
 *
 * Every Buddy surface (BuddyHome, BuddyPanel, BuddySpeechCloud,
 * NavigationRequest handler) must route through this single function
 * so that action semantics are defined in exactly one place.
 */
export async function executeBuddyAction(
  ctrl: BuddyControl,
  dispatch: AppDispatch,
  context?: BuddyActionContext,
  isCurrent: () => boolean = () => true,
): Promise<BuddyActionOutcome> {
  if (!isCurrent()) return ACTION_OK;
  if (
    (ctrl.action === "dismiss_suggestion" && ctrl.action_param?.trim()) ||
    ctrl.action === "create_worktrees_pulse" ||
    context?.suggestionId
  ) {
    const authority = await dispatch(async (send, getState) =>
      send(
        buddyOperatorApi.endpoints.getBuddyOperatorStatus.initiate(
          buddyOperatorIdentity(getState()),
          { subscribe: false, forceRefetch: true },
        ),
      ),
    );
    if (!isCurrent()) return ACTION_OK;
    if (authority.error || authority.data?.mode !== "disabled") {
      return actionUnavailable(
        dispatch,
        "This legacy work control is unavailable. Open Buddy Desk saved work for current revision-bound actions.",
      );
    }
  }
  switch (ctrl.action) {
    case "dismiss":
    case "dismiss_speech":
      dispatch(clearActiveSpeech());
      break;

    case "dismiss_suggestion": {
      const suggestionId = ctrl.action_param?.trim();
      if (!suggestionId) {
        return actionUnavailable(
          dispatch,
          "This dismiss control is missing its suggestion id.",
        );
      }
      await dispatch(
        buddyApi.endpoints.dismissBuddySuggestion.initiate(suggestionId),
      ).unwrap();
      if (!isCurrent()) return ACTION_OK;
      dispatch(dismissBuddySuggestion(suggestionId));
      break;
    }

    case "dismiss_runtime_event": {
      const eventId = ctrl.action_param?.trim();
      if (!eventId) {
        return actionUnavailable(
          dispatch,
          "This dismiss control is missing its event id.",
        );
      }
      dispatch(dismissRuntimeEvent(eventId));
      await dispatch(
        buddyApi.endpoints.dismissBuddyRuntimeEvent.initiate(eventId),
      )
        .unwrap()
        .catch(() => undefined);
      if (!isCurrent()) return ACTION_OK;
      dispatch(clearActiveSpeech());
      break;
    }

    case "open_setup":
      void dispatch(openChatInModeAndStart({ mode: "setup" }));
      dispatch(clearActiveSpeech());
      break;

    case "open_setup_mode": {
      const param = ctrl.action_param ?? "";
      const mode = isValidSetupMode(param) ? param : "setup";
      void dispatch(openChatInModeAndStart({ mode }));
      dispatch(clearActiveSpeech());
      break;
    }

    case "open_setup_mcp":
    case "open_setup_skills":
    case "open_setup_commands":
    case "open_setup_agents_md":
    case "open_setup_subagents": {
      const mode =
        LEGACY_SETUP_ACTION_TO_MODE[ctrl.action as LegacySetupAction];
      void dispatch(openChatInModeAndStart({ mode }));
      dispatch(clearActiveSpeech());
      break;
    }

    case "open_stats":
      navigateFromBuddyPage({ type: "stats" }, dispatch);
      dispatch(clearActiveSpeech());
      break;

    case "open_worktrees":
    case "review_worktree_cleanup":
    case "open_worktree_cleanup":
      navigateFromBuddyPage({ type: "worktrees" }, dispatch);
      dispatch(clearActiveSpeech());
      break;

    case "create_worktrees_pulse": {
      const outcome = await createWorktreesPulseReport(dispatch, isCurrent);
      if (outcome.status !== "ok") return outcome;
      break;
    }

    case "open_buddy":
      navigateFromBuddyPage({ type: "buddy" }, dispatch);
      dispatch(clearActiveSpeech());
      break;

    case "investigate_error": {
      dispatch(clearActiveSpeech());
      if (!context) break;
      await dispatch(
        startBuddyInvestigation({
          triggerText: context.triggerText,
          triggerSource: context.triggerSource,
          sourceChatId: context.sourceChatId,
          diagnostic: context.diagnostic,
        }),
      );
      break;
    }

    case "care_feed":
    case "care_play":
    case "care_pet":
    case "care_sleep":
    case "care_clean": {
      const action = ctrl.action.replace("care_", "") as
        | "feed"
        | "play"
        | "pet"
        | "sleep"
        | "clean";
      const result = await dispatch(
        buddyApi.endpoints.careBuddy.initiate({
          action,
          toy: ctrl.action_param,
        }),
      ).unwrap();
      if (!isCurrent()) return ACTION_OK;
      dispatch(setBuddySnapshot(result.snapshot));
      break;
    }

    case "accept_quest": {
      const suggestionId = ctrl.action_param?.trim();
      if (!suggestionId) {
        return actionUnavailable(
          dispatch,
          "This quest control is missing its quest id.",
        );
      }
      const result = await dispatch(
        buddyApi.endpoints.acceptBuddyQuest.initiate(suggestionId),
      ).unwrap();
      if (!isCurrent()) return ACTION_OK;
      dispatch(setBuddySnapshot(result.snapshot));
      break;
    }

    case "reroll_personality": {
      const result = await dispatch(
        buddyApi.endpoints.rerollBuddyPersonality.initiate(undefined),
      ).unwrap();
      if (!isCurrent()) return ACTION_OK;
      dispatch(setBuddySnapshot(result.snapshot));
      break;
    }

    default:
      return actionUnavailable(
        dispatch,
        "This Buddy action is not available in this build.",
      );
  }

  if (
    isCurrent() &&
    context?.suggestionId &&
    buddyActionResolvesSuggestion(ctrl.action)
  ) {
    await resolveBuddySuggestion(context.suggestionId, dispatch);
  }
  return ACTION_OK;
}

export function navigateFromBuddyPage(page: BuddyPage, dispatch: AppDispatch) {
  executeBuddyNavigation(page, dispatch);
}

export function routeDraftByKind(
  result: { draft_kind: DraftKind; draft_id: string },
  dispatch: AppDispatch,
) {
  switch (result.draft_kind) {
    case "skill":
      dispatch(
        push({ name: "extensions", tab: "skills", draftId: result.draft_id }),
      );
      break;
    case "command":
      dispatch(
        push({ name: "extensions", tab: "commands", draftId: result.draft_id }),
      );
      break;
    case "delegate":
      dispatch(
        push({
          name: "customization",
          kind: "subagents",
          draftId: result.draft_id,
        }),
      );
      break;
    case "mode":
      dispatch(
        push({
          name: "customization",
          kind: "modes",
          draftId: result.draft_id,
        }),
      );
      break;
    case "agents_md":
      dispatch(push({ name: "buddy", draftId: result.draft_id }));
      break;
    case "defaults_model":
      dispatch(push({ name: "default models", draftId: result.draft_id }));
      break;
    case "hook":
      dispatch(
        push({ name: "extensions", tab: "hooks", draftId: result.draft_id }),
      );
      break;
    case "pulse_report":
      dispatch(push({ name: "buddy", draftId: result.draft_id }));
      break;
  }
}

/**
 * Central executor for engine-driven NavigationRequest events.
 *
 * Maps BuddyPage variants to actual GUI page dispatches.
 * Every NavigationRequest from the sidebar SSE must route through here.
 */
export function executeBuddyNavigation(
  page: BuddyPage,
  dispatch: AppDispatch,
): void {
  switch (page.type) {
    case "buddy":
      dispatch(push({ name: "buddy" }));
      break;

    case "stats":
      dispatch(push({ name: "stats dashboard" }));
      break;

    case "settings":
      dispatch(push({ name: "general settings" }));
      break;

    case "customization":
      dispatch(push({ name: "customization" }));
      break;

    case "providers":
      dispatch(push({ name: "providers page" }));
      break;

    case "default_models":
      dispatch(push({ name: "default models" }));
      break;

    case "integrations":
      dispatch(push({ name: "integrations page" }));
      break;

    case "extensions":
      dispatch(push({ name: "extensions" }));
      break;

    case "marketplace_hub":
      dispatch(push({ name: "marketplace hub" }));
      break;

    case "marketplace":
      dispatch(push({ name: "mcp marketplace" }));
      break;

    case "skills_marketplace":
      dispatch(push({ name: "skills marketplace" }));
      break;

    case "commands_marketplace":
      dispatch(push({ name: "commands marketplace" }));
      break;

    case "delegates_marketplace":
      dispatch(push({ name: "subagents marketplace" }));
      break;

    case "tasks_list":
      dispatch(push({ name: "tasks list" }));
      break;

    case "task_workspace":
      dispatch(push({ name: "task workspace", taskId: page.task_id }));
      break;

    case "knowledge_graph":
      dispatch(push({ name: "knowledge graph" }));
      break;

    case "worktrees":
      dispatch(push({ name: "tasks list" }));
      break;

    case "setup_mode":
      if (isValidSetupMode(page.mode)) {
        void dispatch(openChatInModeAndStart({ mode: page.mode }));
      }
      break;

    default: {
      const _never: never = page;
      void _never;
    }
  }
}
