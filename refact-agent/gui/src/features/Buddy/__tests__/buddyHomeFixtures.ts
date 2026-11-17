import { http, HttpResponse } from "msw";
import { vi } from "vitest";
import { server } from "../../../utils/mockServer";
import type {
  BuddyActivityEntry,
  BuddyConversationEntry,
  BuddyOpportunity,
  BuddyPulse,
  BuddyRuntimeEvent,
  BuddySettings,
  BuddySnapshot,
  BuddySuggestion,
} from "../types";
import type { ChatCompanionGateInput } from "../buddyChatCompanionPolicy";

export const CONFIG_STATE = {
  config: {
    apiKey: "test",
    lspPort: 8001,
    themeProps: {},
    host: "vscode" as const,
  },
};

export const operatorStatus = (
  mode: "disabled" | "enabled" | "unavailable",
  unavailableReason?: {
    kind: string;
    detail: string;
    retry_in_seconds?: number;
  },
) => ({
  schema_version: 1,
  mode,
  features: {
    storage: mode !== "disabled",
    actions: false,
    supervision: false,
    improvement: false,
    editorial: false,
  },
  project:
    mode === "enabled"
      ? { project_id: "fixture", canonical_root: "/fixture" }
      : null,
  worker:
    mode === "enabled"
      ? { worker_id: "worker", engine_instance_id: "engine", epoch: 1 }
      : null,
  ...(mode === "unavailable" && unavailableReason
    ? { unavailable_reason: unavailableReason }
    : {}),
});

export async function mockBuddyCharacterModule() {
  const ReactModule = await vi.importActual<typeof import("react")>("react");

  return {
    BuddyCharacter: ({
      bubblePosition,
      randomizeBubblePosition,
      compactBubble,
      scenePose = "idle",
      sceneXPercent,
      sceneYPercent,
      sceneDepthScale,
      speechText,
      speechControls,
      onSpeechControl,
    }: {
      bubblePosition?: string;
      randomizeBubblePosition?: boolean;
      compactBubble?: boolean;
      scenePose?: string;
      sceneXPercent?: number;
      sceneYPercent?: number;
      sceneDepthScale?: number;
      speechText?: string | null;
      speechControls?: {
        id: string;
        label: string;
        action: string;
        action_param?: string;
        style: string;
      }[];
      onSpeechControl?: (control: {
        id: string;
        label: string;
        action: string;
        action_param?: string;
        style: string;
      }) => void;
    }) =>
      ReactModule.createElement(
        "div",
        {
          "data-bubble-position": bubblePosition,
          "data-depth-scale": sceneDepthScale,
          "data-compact-bubble": String(compactBubble),
          "data-pose": scenePose,
          "data-randomize-bubble-position": String(randomizeBubblePosition),
          "data-testid": "buddy-world-character",
          style:
            typeof sceneXPercent === "number" ||
            typeof sceneYPercent === "number"
              ? {
                  ...(typeof sceneXPercent === "number"
                    ? { left: `${sceneXPercent}%` }
                    : undefined),
                  ...(typeof sceneYPercent === "number"
                    ? { bottom: `${100 - sceneYPercent}%` }
                    : undefined),
                }
              : undefined,
        },
        speechText,
        ...(speechControls?.map((control) =>
          ReactModule.createElement(
            "button",
            {
              key: control.id,
              type: "button",
              onClick: () => onSpeechControl?.(control),
            },
            control.label,
          ),
        ) ?? []),
      ),
  };
}

export function makePulse(overrides?: Partial<BuddyPulse>): BuddyPulse {
  return {
    generated_at: "2024-01-01T00:00:00Z",
    tasks: { total: 3, stuck: 1, abandoned: 2, by_status: {} },
    trajectories: { total: 10, untitled: 2, oldest_age_days: 14 },
    memory: { total: 50, orphan: 5, stale_conflicts: 1 },
    providers: { defaults_ok: true, broken_refs: 0, quota_warnings: 0 },
    mcp: { total: 4, failing: 1, auth_expiring: 0 },
    customization: { modes: 3, skills: 2, commands: 1, subagents: 0, hooks: 0 },
    diagnostics: { last_hour: 7, top_error_types: ["model_not_found"] },
    git: { uncommitted_files: 5, diff_lines_4h: 120, branches: 3 },
    worktrees: {
      total_registered: 3,
      total_discovered: 1,
      total: 4,
      clean: 2,
      dirty: 1,
      unknown: 0,
      stale: 1,
      conflicted: 0,
      shared: 1,
      abandoned_clean: 2,
      changed_files: 3,
      additions: 10,
      deletions: 2,
      missing_registry_paths: 1,
      unregistered_cache_dirs: 1,
      merged_branches: 2,
    },
    ...overrides,
  };
}

export function makeSnapshot(
  pulse?: BuddyPulse,
  overrides?: Partial<BuddySnapshot>,
): BuddySnapshot {
  return {
    state: {
      identity: { name: "Buddy", created_at: "", palette_index: 0 },
      progression: {
        stage: 0,
        stage_name: "Egg",
        level: 1,
        xp: 0,
        xp_next: 20,
      },
      skills: { unlocked: [], locked: [] },
      workflow_summaries: [],
      semantic: {
        mood: "idle",
        focus: "helping",
        headline: "",
        last_active: "",
      },
      recent_activities: [],
      suggestion_state: [],
      pet: {
        needs: {
          hunger: 80,
          energy: 85,
          hygiene: 80,
          boredom: 15,
          affection: 75,
        },
        condition: {
          sleeping: false,
          hungry: false,
          sleepy: false,
          dirty: false,
          bored: false,
          lonely: false,
        },
        evolution: {
          care_score: 0,
          neglect_score: 0,
          open_seconds: 0,
          last_evolved_at: null,
        },
      },
      personality: {
        archetype_id: "helper_sprite",
        archetype_label: "Helper Sprite",
        vibe: "Playful",
        summary: "An energetic helper.",
        prompt: "Playful",
        traits: {
          playfulness: 70,
          chaos: 35,
          sociability: 72,
          curiosity: 78,
          resilience: 66,
        },
      },
      active_quest: null,
      opportunities: [],
    },
    settings: {
      enabled: true,
      auto_diagnostics: true,
      auto_issue_creation: false,
      personality_prompt: null,
      autonomous_chats_enabled: true,
      proactive_enabled: true,
      message_observation_enabled: false,
      chat_reactions_enabled: false,
      housekeeping_enabled: true,
      humor_enabled: true,
      humor_level: "light",
      autonomy_level: "suggest",
      quiet_mode: false,
      daily_digest_hour: 18,
      observers: {
        task_health: true,
        trajectory_clutter: true,
        chat_pattern: false,
        customization_drift: true,
        memory_garden: true,
        mcp_auth: true,
        git_pressure: true,
        diagnostic_cluster: true,
        provider_health: true,
      },
    },
    enabled: true,
    pulse,
    ...overrides,
  };
}

export function makeOpportunity(
  overrides?: Partial<BuddyOpportunity>,
): BuddyOpportunity {
  return {
    id: "opp-1",
    kind: "diagnostic_investigation",
    summary: "Model config is broken",
    priority: "high",
    confidence: 0.9,
    fact_keys: [],
    cooldown_key: "opp-1",
    cooldown_secs: 1800,
    status: "new",
    proposed_actions: [],
    humor_allowed: false,
    related: { chat_ids: [], task_ids: [], memory_ids: [], config_paths: [] },
    created_at: "2024-01-01T00:00:00Z",
    expires_at: "2099-12-31T00:00:00Z",
    ...overrides,
  };
}

export function makeSettings(
  overrides?: Partial<BuddySettings>,
): BuddySettings {
  return {
    enabled: true,
    auto_diagnostics: true,
    auto_issue_creation: false,
    personality_prompt: null,
    autonomous_chats_enabled: true,
    proactive_enabled: true,
    message_observation_enabled: true,
    chat_reactions_enabled: true,
    housekeeping_enabled: true,
    humor_enabled: true,
    humor_level: "light",
    autonomy_level: "suggest",
    quiet_mode: false,
    daily_digest_hour: 18,
    observers: {
      task_health: true,
      trajectory_clutter: true,
      chat_pattern: true,
      customization_drift: true,
      memory_garden: true,
      mcp_auth: true,
      git_pressure: true,
      diagnostic_cluster: true,
      provider_health: true,
    },
    ...overrides,
  };
}

export function makeHomeOpportunity(
  overrides?: Partial<BuddyOpportunity>,
): BuddyOpportunity {
  return {
    id: "opp-1",
    kind: "diagnostic_investigation",
    summary: "Model config is broken",
    priority: "high",
    confidence: 0.9,
    fact_keys: [],
    cooldown_key: "opp-1",
    cooldown_secs: 1800,
    status: "new",
    proposed_actions: [{ kind: "open_page", page: { type: "providers" } }],
    humor_allowed: false,
    related: { chat_ids: [], task_ids: [], memory_ids: [], config_paths: [] },
    created_at: "2024-01-01T00:00:00Z",
    expires_at: "2099-12-31T00:00:00Z",
    ...overrides,
  };
}

export function makeHomeActivity(
  overrides?: Partial<BuddyActivityEntry>,
): BuddyActivityEntry {
  return {
    icon: "⚙️",
    title: "Task coach noticed pattern",
    description: "A tiny nudge appeared.",
    timestamp: "2024-01-01T00:00:00Z",
    activity_type: "buddy_task_health",
    chat_id: null,
    ...overrides,
  };
}

export function makeRuntimeEvent(
  overrides?: Partial<BuddyRuntimeEvent>,
): BuddyRuntimeEvent {
  return {
    id: "runtime-1",
    signal_type: "chat_error",
    title: "Model unavailable",
    description: "Default model is missing.",
    source: "provider",
    status: "failed",
    priority: "high",
    created_at: new Date().toISOString(),
    ...overrides,
  };
}

export function makeConversation(
  overrides?: Partial<BuddyConversationEntry>,
): BuddyConversationEntry {
  return {
    id: "buddy-chat-1",
    kind: "chat",
    title: "Recent Buddy chat",
    created_at: "2024-01-01T00:00:00Z",
    updated_at: "2024-01-01T01:00:00Z",
    status: "completed",
    message_count: 3,
    icon: "💬",
    badge: null,
    ...overrides,
  };
}

export function makeHomeSnapshot(
  overrides?: Partial<BuddySnapshot>,
): BuddySnapshot {
  const settings = overrides?.settings ?? makeSettings();
  const opportunity = makeHomeOpportunity();
  return {
    state: {
      identity: { name: "Buddy", created_at: "", palette_index: 0 },
      progression: {
        stage: 0,
        stage_name: "Egg",
        level: 1,
        xp: 8,
        xp_next: 20,
      },
      skills: { unlocked: [], locked: [] },
      workflow_summaries: [],
      semantic: {
        mood: "idle",
        focus: "helping",
        headline: "Keeping watch",
        last_active: "2024-01-01T00:00:00Z",
      },
      recent_activities: [makeHomeActivity()],
      suggestion_state: [
        {
          id: "suggestion-1",
          suggestion_type: "quest_start_setup",
          title: "Warm up this workspace",
          description: "Kick off setup so Buddy can help proactively.",
          created_at: "2024-01-01T00:00:00Z",
          dismissed: false,
          controls: [],
          quest: null,
        },
      ],
      pet: {
        needs: {
          hunger: 80,
          energy: 85,
          hygiene: 80,
          boredom: 15,
          affection: 75,
        },
        condition: {
          sleeping: false,
          hungry: false,
          sleepy: false,
          dirty: false,
          bored: false,
          lonely: false,
        },
        evolution: {
          care_score: 4,
          neglect_score: 0,
          open_seconds: 0,
          last_evolved_at: null,
        },
      },
      personality: {
        archetype_id: "helper_sprite",
        archetype_label: "Helper Sprite",
        vibe: "Playful",
        summary: "An energetic helper.",
        prompt: "Playful",
        traits: {
          playfulness: 70,
          chaos: 35,
          sociability: 72,
          curiosity: 78,
          resilience: 66,
        },
      },
      active_quest: null,
      opportunities: [opportunity],
    },
    settings,
    enabled: settings.enabled,
    pulse: makePulse(),
    opportunities: [opportunity],
    runtime_queue: [makeRuntimeEvent()],
    recent_diagnostics: [],
    active_speech: null,
    now_playing: null,
    active_drafts: [],
    ...overrides,
  };
}

export function installBuddyHomeHandlers(settings = makeSettings()) {
  const conversation = makeConversation();
  server.use(
    http.get("*/v1/buddy/opportunities", () =>
      HttpResponse.json({ opportunities: [makeHomeOpportunity()] }),
    ),
    http.get("*/v1/buddy/conversations", () =>
      HttpResponse.json([conversation]),
    ),
    http.get("*/v1/stats/llm/summary", () =>
      HttpResponse.json({
        totals: { total_calls: 12, successful_calls: 9, total_tokens: 3456 },
      }),
    ),
    http.get("*/v1/setup/status", () =>
      HttpResponse.json({
        configured: false,
        reasons: ["missing agents"],
        detail: {
          has_agents_md: false,
          has_knowledge: false,
          has_trajectories: false,
        },
      }),
    ),
    http.post("*/v1/buddy/settings", async ({ request }) => {
      const patch = (await request.json()) as Partial<BuddySettings>;
      return HttpResponse.json({ ...settings, ...patch });
    }),
  );
}

export function makeSuggestion(
  overrides?: Partial<BuddySuggestion>,
): BuddySuggestion {
  return {
    id: "suggestion-1",
    suggestion_type: "quest_start_setup",
    title: "Warm up this workspace",
    description: "Kick off setup so Buddy can help proactively.",
    created_at: "2024-01-01T00:00:00Z",
    dismissed: false,
    controls: [],
    quest: null,
    ...overrides,
  };
}

export function makePolishSnapshot(): BuddySnapshot {
  return {
    state: {
      identity: { name: "Buddy", created_at: "", palette_index: 0 },
      progression: {
        stage: 0,
        stage_name: "Egg",
        level: 1,
        xp: 0,
        xp_next: 20,
      },
      skills: { unlocked: [], locked: [] },
      workflow_summaries: [],
      semantic: {
        mood: "idle",
        focus: "helping",
        headline: "Ready to help",
        last_active: "",
      },
      recent_activities: [],
      suggestion_state: [],
      pet: {
        needs: {
          hunger: 80,
          energy: 85,
          hygiene: 80,
          boredom: 15,
          affection: 75,
        },
        condition: {
          sleeping: false,
          hungry: false,
          sleepy: false,
          dirty: false,
          bored: false,
          lonely: false,
        },
        evolution: {
          care_score: 0,
          neglect_score: 0,
          open_seconds: 0,
          last_evolved_at: null,
        },
      },
      personality: {
        archetype_id: "helper_sprite",
        archetype_label: "Helper Sprite",
        vibe: "Playful",
        summary: "An energetic helper.",
        prompt: "Playful",
        traits: {
          playfulness: 70,
          chaos: 35,
          sociability: 72,
          curiosity: 78,
          resilience: 66,
        },
      },
      active_quest: null,
      opportunities: [],
    },
    settings: {
      enabled: true,
      auto_diagnostics: true,
      auto_issue_creation: false,
      personality_prompt: null,
      autonomous_chats_enabled: true,
      proactive_enabled: true,
      message_observation_enabled: false,
      chat_reactions_enabled: false,
      housekeeping_enabled: true,
      humor_enabled: true,
      humor_level: "light",
      autonomy_level: "suggest",
      quiet_mode: false,
      daily_digest_hour: 18,
      observers: {
        task_health: true,
        trajectory_clutter: true,
        chat_pattern: false,
        customization_drift: true,
        memory_garden: true,
        mcp_auth: true,
        git_pressure: true,
        diagnostic_cluster: true,
        provider_health: true,
      },
    },
    enabled: true,
    pulse: makePulse(),
  };
}

export function makeSliceOpportunity(
  overrides?: Partial<BuddyOpportunity>,
): BuddyOpportunity {
  return {
    id: "opp-1",
    kind: "diagnostic_investigation",
    summary: "Investigate errors",
    priority: "high",
    confidence: 0.9,
    fact_keys: [],
    cooldown_key: "diagnostic:opp-1",
    cooldown_secs: 1800,
    status: "new",
    proposed_actions: [],
    humor: null,
    humor_allowed: false,
    related: { chat_ids: [], task_ids: [], memory_ids: [], config_paths: [] },
    created_at: "2024-01-01T00:00:00Z",
    expires_at: "2099-01-01T00:00:00Z",
    resolved_at: null,
    ...overrides,
  };
}

export function makeGateInput(
  overrides?: Partial<ChatCompanionGateInput>,
): ChatCompanionGateInput {
  return {
    nowMs: 1_000_000,
    quietUntilMs: null,
    queuedMessageCount: 0,
    lastAmbientImpressionAtMs: null,
    candidateIsAmbient: true,
    candidateAlreadyImpressed: false,
    bypassGates: false,
    ...overrides,
  };
}
