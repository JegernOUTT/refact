import type { AppDispatch } from "../../app/store";
import { push } from "../Pages/pagesSlice";
import {
  clearActiveSpeech,
  dismissBuddySuggestion,
  dismissRuntimeEvent,
  setBuddySnapshot,
} from "./buddySlice";
import {
  openChatInModeAndStart,
  startBuddyInvestigation,
} from "../Chat/Thread";
import { isValidSetupMode } from "../Setup/setupModes";
import type { BuddyControl, BuddyPage, DraftKind } from "./types";
import type { DiagnosticContext } from "./types";
import { buddyApi } from "../../services/refact/buddy";

const LEGACY_SETUP_ACTION_TO_MODE = {
  open_setup_mcp: "setup_mcp",
  open_setup_skills: "setup_skills",
  open_setup_commands: "setup_commands",
  open_setup_agents_md: "setup_agents_md",
  open_setup_subagents: "setup_subagents",
} as const;

type LegacySetupAction = keyof typeof LEGACY_SETUP_ACTION_TO_MODE;

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
  investigation?: {
    triggerText: string;
    triggerSource:
      | "thread"
      | "runtime"
      | "diagnostic"
      | "suggestion"
      | "frontend";
    sourceChatId?: string;
    diagnostic?: DiagnosticContext | null;
  },
): Promise<void> {
  switch (ctrl.action) {
    case "dismiss":
      dispatch(clearActiveSpeech());
      break;

    case "dismiss_suggestion": {
      const suggestionId = ctrl.action_param;
      if (!suggestionId) break;
      await dispatch(
        buddyApi.endpoints.dismissBuddySuggestion.initiate(suggestionId),
      ).unwrap();
      dispatch(dismissBuddySuggestion(suggestionId));
      break;
    }

    case "dismiss_runtime_event": {
      const eventId = ctrl.action_param?.trim();
      if (!eventId) break;
      dispatch(dismissRuntimeEvent(eventId));
      await dispatch(
        buddyApi.endpoints.dismissBuddyRuntimeEvent.initiate(eventId),
      )
        .unwrap()
        .catch(() => undefined);
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

    case "create_worktrees_pulse":
      navigateFromBuddyPage({ type: "buddy" }, dispatch);
      dispatch(clearActiveSpeech());
      break;

    case "open_buddy":
      navigateFromBuddyPage({ type: "buddy" }, dispatch);
      dispatch(clearActiveSpeech());
      break;

    case "investigate_error": {
      dispatch(clearActiveSpeech());
      if (!investigation) break;
      await dispatch(startBuddyInvestigation(investigation));
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
      dispatch(setBuddySnapshot(result.snapshot));
      break;
    }

    case "accept_quest": {
      const suggestionId = ctrl.action_param;
      if (!suggestionId) break;
      const result = await dispatch(
        buddyApi.endpoints.acceptBuddyQuest.initiate(suggestionId),
      ).unwrap();
      dispatch(setBuddySnapshot(result.snapshot));
      break;
    }

    case "reroll_personality": {
      const result = await dispatch(
        buddyApi.endpoints.rerollBuddyPersonality.initiate(undefined),
      ).unwrap();
      dispatch(setBuddySnapshot(result.snapshot));
      break;
    }

    default:
      dispatch(clearActiveSpeech());
  }
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
