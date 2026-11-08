import type {
  Assignment,
  OrganizationItem,
  OrganizationOperation,
  Taxon,
} from "../../src/services/refact/organization";
import React from "react";
import { createRoot } from "react-dom/client";
import { Provider, useSelector } from "react-redux";
import { Badge, Button as UiButton } from "../../src/components/ui";
import { Button, Flex, Text } from "@radix-ui/themes";

import { setUpStore, type RootState } from "../../src/app/store";
import { Theme } from "../../src/components/Theme";
import { AbortControllerProvider } from "../../src/contexts/AbortControllers";
import { Dashboard } from "../../src/features/Dashboard";
import { Toolbar } from "../../src/components/Toolbar/Toolbar";
import { WorkspaceView } from "../../src/features/Workspace/WorkspaceView";
import { WorkspaceShell } from "../../src/features/Workspace/WorkspaceShell";
import { WorkPanel } from "../../src/features/Workspace/WorkPanel/WorkPanel";
import { ToolContent } from "../../src/components/ChatContent/ToolsContent";
import { resumeReceipt } from "../../src/components/Tools/AgenticTool/resumeFixtures";
import {
  launchPreviewCall,
  launchPreviewReceipt,
} from "../../src/components/Tools/AgenticTool/launchFixtures";
import { makeSurfaceKey } from "../../src/features/Workspace/surfaceKey";
import {
  openTab,
  setActiveTab,
  setDockOpen,
  setDockSection,
  setPanelsForced,
} from "../../src/features/Workspace/workspaceSlice";
import {
  openGitDiff,
  setGitDiffViewerPrefs,
} from "../../src/features/Workspace/GitPanel/gitPanelSlice";
import { TrajectoryButton } from "../../src/components/Trajectory";
import { ModeSelect } from "../../src/components/ChatForm/ModeSelect";
import {
  ModelSelector,
  type ModelOption,
} from "../../src/components/ui/ModelSelector";
import { BuddyHome } from "../../src/features/Buddy";
import { setBuddySnapshot } from "../../src/features/Buddy/buddySlice";
import {
  SETTINGS_SECTIONS,
  SettingsHub,
  settingsSectionToPage,
  type SettingsSectionId,
} from "../../src/features/Settings";
import {
  MarketplaceHub,
  isMarketplacePage,
  marketplaceTabToPage,
  type MarketplaceTabId,
} from "../../src/features/MarketplaceHub";
import { setBackendStatus } from "../../src/features/Connection";
import { applyChatEvent } from "../../src/features/Chat/Thread/actions";
import { selectMessagesById } from "../../src/features/Chat/Thread/selectors";
import type { EventEnvelope } from "../../src/services/refact/chatSubscription";
import { isPushMode } from "../../src/services/refact/chatSubscription";
import { sidebarSectionSnapshotReceived } from "../../src/features/Sidebar/sidebarSlice";
import { tasksApi, type TaskMeta } from "../../src/services/refact/tasks";
import type { OperatorStatus } from "../../src/services/refact/buddyOperator";
import type { ChatHistoryItem } from "../../src/features/History/historySlice";
import type { ChatThreadRuntime } from "../../src/features/Chat/Thread";
import type { ChatMessages } from "../../src/services/refact";
import type {
  BuddyActivityEntry,
  BuddyConversationEntry,
  BuddyOpportunity,
  BuddyPulse,
  BuddyRuntimeEvent,
  BuddySettings,
  BuddySnapshot,
  DiagnosticContext,
} from "../../src/features/Buddy/types";
import type {
  CapsResponse,
  ConfiguredProvidersResponse,
  IntegrationWithIconResponse,
  ProviderDefaults,
} from "../../src/services/refact";
import type { ConfigItem } from "../../src/services/refact/customization";
import type { ExtRegistryResponse } from "../../src/services/refact/extensions";
import type {
  PrivacyFileRecord,
  PrivacyInspectResponse,
  PrivacyPolicyResponse,
  PrivacyStatusResponse,
} from "../../src/services/refact/privacy";
import { BlockCard } from "../../src/features/Privacy/BlockCard";
import { WithheldOutputCard } from "../../src/features/Privacy/WithheldOutputCard";
import "../../src/lib/render/web.css";

const now = "2026-06-07T10:00:00Z";

const showcaseGitChange = (
  relativePath: string,
  status: "ADDED" | "MODIFIED" | "DELETED" | "RENAMED",
  oldPath: string | null = null,
) => ({
  relative_path: relativePath,
  absolute_path: `/workspace/refact/${relativePath}`,
  old_path: oldPath,
  status,
  staged: false,
  unstaged: true,
});

const showcaseGitStatus = {
  root: "/workspace/refact",
  branch: "feature/commit-window",
  head_detached: false,
  head_oid: "9f1c2ab3d4e5f60718293a4b5c6d7e8f90a1b2c3",
  upstream: "origin/feature/commit-window",
  ahead: 2,
  behind: 1,
  repo_state: "merge",
  truncated: false,
  staged: [],
  unstaged: [
    {
      relative_path: "README.md",
      absolute_path: "/workspace/refact/README.md",
      status: "MODIFIED",
    },
  ],
  untracked_included: true,
  changes: [
    showcaseGitChange("README.md", "MODIFIED"),
    showcaseGitChange(
      "refact-agent/gui/src/features/Workspace/GitPanel/GitDock.tsx",
      "MODIFIED",
    ),
    showcaseGitChange(
      "refact-agent/gui/src/features/Workspace/GitPanel/ChangesTree.tsx",
      "ADDED",
    ),
    showcaseGitChange(
      "refact-agent/gui/src/features/Workspace/GitPanel/StatusList.tsx",
      "DELETED",
    ),
    showcaseGitChange(
      "docs/commit-window.md",
      "RENAMED",
      "docs/changes-panel.md",
    ),
  ],
  untracked: [
    {
      relative_path: "notes/todo.txt",
      absolute_path: "/workspace/refact/notes/todo.txt",
    },
  ],
  conflicted: [
    {
      relative_path: "refact-agent/engine/src/git/mod.rs",
      absolute_path: "/workspace/refact/refact-agent/engine/src/git/mod.rs",
      ours: "a1",
      theirs: "b2",
      ancestor: "c3",
    },
  ],
};
const showcaseGitActionsStatus = {
  ...showcaseGitStatus,
  branch: "main",
  upstream: "origin/main",
  ahead: 3,
  behind: 1,
  repo_state: "clean",
};
const showcaseDiffHead = `import { createSlice, type PayloadAction } from "@reduxjs/toolkit";

import { makeSurfaceKey } from "../surfaceKey";
import { openTab, setDockOpen } from "../workspaceSlice";

export type GitDiffTarget = {
  root: string;
  path: string;
  kind: "change" | "untracked";
};

type GitPanelState = {
  selected: GitDiffTarget | null;
  history: string[];
};

const initialState: GitPanelState = {
  selected: null,
  history: [],
};

export const gitPanelSlice = createSlice({
  name: "gitPanel",
  initialState,
  reducers: {
    selectTarget: (state, action: PayloadAction<GitDiffTarget>) => {
      state.selected = action.payload;
      state.history.push(action.payload.path);
    },
    clearTarget: (state) => {
      state.selected = null;
    },
    trimHistory: (state) => {
      state.history = state.history.slice(-20);
    },
  },
});

export const { selectTarget, clearTarget, trimHistory } =
  gitPanelSlice.actions;

function isMobile(): boolean {
  return window.matchMedia("(max-width: 767px)").matches;
}

export const openGitDiff = (target: GitDiffTarget) => (dispatch) => {
  dispatch(selectTarget(target));
  dispatch(openTab(makeSurfaceKey("git", "main")));
  if (isMobile()) dispatch(setDockOpen(false));
};

export default gitPanelSlice.reducer;
`;
const showcaseDiffWorktree = `import { createSlice, type PayloadAction } from "@reduxjs/toolkit";

import { makeGitDiffSurfaceKey } from "../surfaceKey";
import { bindSurfaceToChat, openTab, setDockOpen } from "../workspaceSlice";
import type { GitDiffDescriptor } from "./diff/gitDiffDescriptor";

export type GitDiffTarget = {
  root: string;
  path: string;
  old_path: string | null;
  kind: "change" | "untracked" | "conflict";
};

type GitPanelState = {
  selected: GitDiffTarget | null;
  history: string[];
};

const initialState: GitPanelState = {
  selected: null,
  history: [],
};

export const gitPanelSlice = createSlice({
  name: "gitPanel",
  initialState,
  reducers: {
    selectTarget: (state, action: PayloadAction<GitDiffTarget>) => {
      state.selected = action.payload;
      state.history.push(action.payload.path);
    },
    trimHistory: (state) => {
      state.history = state.history.slice(-50);
    },
  },
});

export const { selectTarget, trimHistory } = gitPanelSlice.actions;

function isMobile(): boolean {
  return window.matchMedia("(max-width: 767px)").matches;
}

export function workingDiffDescriptor(
  target: GitDiffTarget,
): GitDiffDescriptor {
  return {
    root: target.root,
    path: target.path,
    old_path: target.old_path,
    left: { rev: "HEAD", path: target.old_path ?? target.path },
    right: { rev: "WORKTREE", path: target.path },
    kind: "working",
  };
}

export const openGitDiff = (target: GitDiffTarget) => (dispatch) => {
  const surfaceKey = makeGitDiffSurfaceKey(workingDiffDescriptor(target));
  dispatch(selectTarget(target));
  dispatch(openTab(surfaceKey));
  dispatch(bindSurfaceToChat({ surfaceKey, chatId: "showcase" }));
  if (isMobile()) dispatch(setDockOpen(false));
};

export default gitPanelSlice.reducer;
`;
type ShowcaseLogSeed = {
  id: string;
  parents: string[];
  hours: number;
  subject: string;
  author: number;
};
const showcaseAuthors = [
  ["Ada Lovelace", "ada@example.com"],
  ["Grace Hopper", "grace@example.com"],
  ["Linus Torvalds", "linus@example.com"],
  ["Margaret Hamilton", "margaret@example.com"],
] as const;
const showcaseMainSubjects = [
  "Initial project layout",
  "Add engine HTTP router",
  "Wire chat runtime to the GUI",
  "Introduce workspace dock",
  "Add files panel tree",
  "Persist dock width",
  "Add git status endpoint",
  "Render status in the dock",
  "Fix CRLF handling in diffs",
  "Add commit tool window",
  "Group changes by directory",
  "Support amend in commit panel",
  "Stage and unstage endpoints",
  "Rollback dialog",
  "Merge conflicts group",
  "Polish keyboard navigation",
  "Diff viewer as editor tab",
  "Merge branch 'feature/log-graph'",
  "Bump dependencies",
  "Track ahead/behind counts",
  "Add commit details endpoint",
  "Show changed files in details",
  "Remote branches in branches endpoint",
  "Tags in branches endpoint",
  "Refactor git read service",
  "Merge branch 'feature/branch-pane'",
  "Debounce log filters",
  "Fix flaky log test",
  "Document git tool window",
  "Release 1.2",
];
function showcaseOid(id: string): string {
  let hash = 2166136261;
  let out = "";
  for (let round = 0; out.length < 40; round += 1) {
    for (const char of `${id}:${round}`) {
      hash = Math.imul(hash ^ char.charCodeAt(0), 16777619) >>> 0;
    }
    out += hash.toString(16).padStart(8, "0");
  }
  return out.slice(0, 40);
}
function showcaseLogSeeds(): ShowcaseLogSeed[] {
  const seeds: ShowcaseLogSeed[] = showcaseMainSubjects.map(
    (subject, index) => ({
      id: `M${index + 1}`,
      parents: index === 0 ? [] : [`M${index}`],
      hours: (index + 1) * 10,
      subject,
      author: index % showcaseAuthors.length,
    }),
  );
  seeds[17].parents = ["M17", "A6"];
  seeds[25].parents = ["M25", "B5"];
  const branch = (
    prefix: string,
    from: string,
    start: number,
    step: number,
    subjects: string[],
    author: number,
  ) =>
    subjects.map((subject, index) => ({
      id: `${prefix}${index + 1}`,
      parents: [index === 0 ? from : `${prefix}${index}`],
      hours: start + (index + 1) * step,
      subject,
      author,
    }));
  return [
    ...seeds,
    ...branch(
      "A",
      "M10",
      100,
      12,
      [
        "Commit graph layout module",
        "Lane reuse and merge edges",
        "Render graph cells as SVG",
        "Colour lanes with chart tokens",
        "Paging continuation for lanes",
        "Graph unit tests",
      ],
      1,
    ),
    ...branch(
      "B",
      "M20",
      200,
      11,
      [
        "Branches pane skeleton",
        "Group remotes by name",
        "Collapse tags by default",
        "Navigate log by ref",
        "Branch search box",
      ],
      3,
    ),
    ...branch(
      "C",
      "M28",
      280,
      9,
      [
        "Spike: branch context actions",
        "Checkout from branches pane",
        "WIP rename branch dialog",
      ],
      2,
    ),
  ].sort((left, right) => right.hours - left.hours);
}
const showcaseLogRefs: Record<string, { name: string; kind: string }[]> = {
  M30: [
    { name: "HEAD", kind: "head" },
    { name: "main", kind: "local" },
    { name: "origin/main", kind: "remote" },
    { name: "v1.2", kind: "tag" },
  ],
  C3: [
    { name: "feature/branch-actions", kind: "local" },
    { name: "origin/feature/branch-actions", kind: "remote" },
  ],
  A6: [{ name: "origin/feature/log-graph", kind: "remote" }],
  M26: [{ name: "v1.1", kind: "tag" }],
  M18: [{ name: "v1.0", kind: "tag" }],
};
const showcaseLogNow = Date.parse(now);
const showcaseLogCommits = showcaseLogSeeds().map((seed) => {
  const [authorName, authorEmail] = showcaseAuthors[seed.author];
  const oid = showcaseOid(seed.id);
  return {
    oid,
    short_oid: oid.slice(0, 8),
    time_ms: showcaseLogNow - (320 - seed.hours) * 3_600_000,
    author_name: authorName,
    author_email: authorEmail,
    message_first_line: seed.subject,
    message: `${seed.subject}\n\nKeeps the JetBrains-style Git tool window in sync with the engine.\nCo-authored-by: Refact Agent <agent@refact.ai>`,
    parents: seed.parents.map(showcaseOid),
    refs: showcaseLogRefs[seed.id] ?? [],
  };
});
const showcaseBranches = {
  root: "/workspace/refact",
  current: "main",
  head_detached: false,
  head_oid: showcaseOid("M30"),
  branches: [
    {
      name: "main",
      is_head: true,
      upstream: "origin/main",
      tip_oid: showcaseOid("M30"),
      ahead: 0,
      behind: 0,
      upstream_gone: false,
    },
    {
      name: "feature/branch-actions",
      is_head: false,
      upstream: "origin/feature/branch-actions",
      tip_oid: showcaseOid("C3"),
      ahead: 2,
      behind: 1,
      upstream_gone: false,
    },
    {
      name: "feature/log-graph",
      is_head: false,
      upstream: "origin/feature/log-graph",
      tip_oid: showcaseOid("A6"),
      ahead: 0,
      behind: 0,
      upstream_gone: true,
    },
  ],
  remotes: ["origin", "upstream"],
  remote_branches: [
    ["origin", "main", "M30"],
    ["origin", "feature/branch-actions", "C3"],
    ["origin", "feature/log-graph", "A6"],
    ["upstream", "main", "M26"],
  ].map(([remote, shortName, id]) => ({
    name: `${remote}/${shortName}`,
    remote,
    short_name: shortName,
    tip_oid: showcaseOid(id),
  })),
  tags: [
    ["v1.2", "M30"],
    ["v1.1", "M26"],
    ["v1.0", "M18"],
  ].map(([name, id]) => ({ name, target_oid: showcaseOid(id) })),
};
function showcaseCommitDetails(oid: string) {
  const commit =
    showcaseLogCommits.find((item) => item.oid === oid) ??
    showcaseLogCommits[0];
  const gitPanel = "refact-agent/gui/src/features/Workspace/GitPanel";
  return {
    root: "/workspace/refact",
    oid: commit.oid,
    short_oid: commit.short_oid,
    parents: commit.parents,
    author_name: commit.author_name,
    author_email: commit.author_email,
    author_time_ms: commit.time_ms,
    committer_name: "GitHub",
    committer_email: "noreply@github.com",
    committer_time_ms: commit.time_ms + 600_000,
    message: commit.message,
    refs: commit.refs,
    branches: ["main", "origin/main", "upstream/main"],
    files: [
      [`${gitPanel}/log/GitLog.tsx`, null, "ADDED", 148, 0],
      [`${gitPanel}/log/commitGraphLayout.ts`, null, "ADDED", 162, 0],
      [`${gitPanel}/GitPanel.tsx`, null, "MODIFIED", 64, 31],
      [`${gitPanel}/GitPanel.module.css`, null, "MODIFIED", 58, 74],
      [`${gitPanel}/BranchesLog.tsx`, null, "DELETED", 0, 108],
      [
        "refact-agent/gui/src/services/refact/gitRead.ts",
        null,
        "MODIFIED",
        131,
        12,
      ],
      ["docs/git-tool-window.md", "docs/git-panel.md", "RENAMED", 12, 4],
    ].map(([path, oldPath, status, additions, deletions]) => ({
      path,
      old_path: oldPath,
      status,
      additions,
      deletions,
      binary: false,
    })),
    files_truncated: false,
  };
}
const chatId = "showcase-chat";
const splitChatId = "showcase-chat-b";
const chatSurface = (id: string) => makeSurfaceKey("chat", id);
const route =
  new URLSearchParams(window.location.search).get("route") ?? "dashboard";
const terminalDisabledRoute = route === "workspace-chrome-disabled";
const workspaceGitDiffRoute = route === "workspace-git-diff";
const workspaceGitLogRoute = route === "workspace-git-log";
const workspaceGitBranchesRoute = route === "workspace-git-branches";
const workspaceGitPushRoute = route === "workspace-git-push";
const workspaceGitActionsRoute =
  workspaceGitBranchesRoute || workspaceGitPushRoute;
const workspaceGitRoute =
  route === "workspace-git" ||
  workspaceGitDiffRoute ||
  workspaceGitLogRoute ||
  workspaceGitActionsRoute;
const workspaceChromeRoute =
  route === "workspace-chrome" || terminalDisabledRoute || workspaceGitRoute;
const chatRoute =
  route === "chat" ||
  route === "chat-split" ||
  route === "queue" ||
  workspaceChromeRoute;
const chatDndRoute = route === "chat-dnd";
const queueRoute = route === "queue";
const multiChatRoute = route === "chat-split" || chatDndRoute;
const homeActivityRoute = route === "home-activity";
const homeActivityChats = [
  {
    id: "activity-streaming",
    title: "Streaming refactor of the dock",
    updated_at_ms: Date.parse("2026-06-07T09:59:00Z"),
    session_state: "generating",
    recovery_state: null,
    schedule_count: 0,
    paused_schedule_count: 0,
  },
  {
    id: "activity-recovering",
    title: "Recovering provider timeout",
    updated_at_ms: Date.parse("2026-06-07T09:58:00Z"),
    session_state: null,
    recovery_state: "waiting",
    schedule_count: 0,
    paused_schedule_count: 0,
  },
  {
    id: "activity-attention",
    title: "Recovery needs your decision",
    updated_at_ms: Date.parse("2026-06-07T09:50:00Z"),
    session_state: null,
    recovery_state: "needs_action",
    schedule_count: 0,
    paused_schedule_count: 0,
  },
  {
    id: "activity-scheduled",
    title: "Nightly dependency audit",
    updated_at_ms: Date.parse("2026-06-07T08:00:00Z"),
    session_state: "idle",
    recovery_state: null,
    schedule_count: 2,
    paused_schedule_count: 1,
  },
  {
    id: "activity-paused",
    title: "Paused weekly report",
    updated_at_ms: Date.parse("2026-06-06T08:00:00Z"),
    session_state: null,
    recovery_state: null,
    schedule_count: 1,
    paused_schedule_count: 1,
  },
];

// Legacy layout is the default so an absent/failed status never implies
// compatibility. `?operator=enabled|unavailable` opts into the operator surface
// so browser specs can exercise the Desk and the degraded collapse.
const operatorParam = new URLSearchParams(window.location.search).get(
  "operator",
);
const sceneSpeechParam = new URLSearchParams(window.location.search).has(
  "scene_speech",
);
const recentErrorsFixtureParam = new URLSearchParams(
  window.location.search,
).has("recent_errors_fixture");
const showcaseProject = {
  project_id: "showcase",
  canonical_root: "/workspace/refact",
};
const buddyOperatorStatus: OperatorStatus =
  operatorParam === "enabled"
    ? {
        schema_version: 1,
        mode: "enabled",
        features: {
          storage: true,
          actions: true,
          supervision: false,
          improvement: false,
          editorial: false,
        },
        project: showcaseProject,
        worker: {
          worker_id: "showcase-worker",
          engine_instance_id: "showcase-engine",
          epoch: 1,
        },
      }
    : {
        schema_version: 1,
        mode: operatorParam === "unavailable" ? "unavailable" : "disabled",
        features: {
          storage: false,
          actions: false,
          supervision: false,
          improvement: false,
          editorial: false,
        },
        project: null,
        worker: null,
        ...(operatorParam === "unavailable"
          ? {
              unavailable_reason: {
                kind: "already_owned",
                detail:
                  "operator store is already owned by another writer (pid 2810136 (refact-lsp 0.1.0) on svakhreev-work, started 2026-09-15 09:36:52 UTC, cmd: refact-lsp --http-port 8001)",
                retry_in_seconds: 900,
              },
            }
          : {}),
      };

const buddySettings: BuddySettings = {
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
};

const buddyPulse: BuddyPulse = {
  generated_at: now,
  tasks: { total: 48, stuck: 1, abandoned: 9, by_status: { planning: 12 } },
  trajectories: { total: 124, untitled: 6, oldest_age_days: 18 },
  memory: { total: 28156, orphan: 3, stale_conflicts: 1 },
  providers: { defaults_ok: true, broken_refs: 0, quota_warnings: 1 },
  mcp: { total: 7, failing: 1, auth_expiring: 1 },
  customization: { modes: 8, skills: 14, commands: 9, subagents: 6, hooks: 3 },
  diagnostics: { last_hour: 4, top_error_types: ["frontend", "llm_error"] },
  git: { uncommitted_files: 5, diff_lines_4h: 180, branches: 4 },
  worktrees: {
    total_registered: 5,
    total_discovered: 2,
    total: 7,
    clean: 3,
    dirty: 2,
    unknown: 0,
    stale: 1,
    conflicted: 0,
    shared: 1,
    abandoned_clean: 2,
    changed_files: 9,
    additions: 420,
    deletions: 73,
    missing_registry_paths: 1,
    unregistered_cache_dirs: 1,
    merged_branches: 2,
  },
  humor: "Tiny dashboard goblin found one suspicious scrollbar crumb.",
};

const buddyOpportunity: BuddyOpportunity = {
  id: "showcase-opportunity",
  kind: "diagnostic_investigation",
  summary:
    "Investigate a long-running diagnostic cluster before it becomes gremlin soup",
  priority: "high",
  confidence: 0.92,
  fact_keys: ["diagnostic:frontend:overflow"],
  cooldown_key: "showcase-opportunity",
  cooldown_secs: 1800,
  status: "new",
  proposed_actions: [
    { kind: "open_page", page: { type: "providers" } },
    { kind: "dismiss" },
  ],
  humor_allowed: true,
  humor: "Scrollbar? I hardly knowbar.",
  related: {
    chat_ids: ["showcase-chat"],
    task_ids: ["task-active"],
    memory_ids: [],
    config_paths: ["/workspace/refact/refact-agent/gui/src/features/Buddy"],
  },
  created_at: now,
  expires_at: "2099-12-31T00:00:00Z",
};

const buddyActivities: BuddyActivityEntry[] = [
  {
    icon: "🧭",
    title: "Buddy reviewed responsive surfaces",
    description:
      "Checked hero, grids, activity, settings, and workshop panels.",
    timestamp: now,
    activity_type: "buddy_layout_review",
    chat_id: "showcase-chat",
  },
  {
    icon: "⚙️",
    title: "Refact e2e gate queued",
    description: "No horizontal scroll matrix is ready for narrow widths.",
    timestamp: "2026-06-07T09:40:00Z",
    activity_type: "refact_e2e_gate",
    chat_id: null,
  },
];

const buddyRuntimeEvent: BuddyRuntimeEvent = {
  id: "showcase-runtime-error",
  signal_type: "frontend_error_burst",
  title: "Possible layout overflow",
  description: "A narrow viewport almost clipped a Buddy utility panel.",
  source: "route-showcase",
  status: "failed",
  failure_category: "frontend",
  failure_summary: "responsive layout guard",
  priority: "high",
  created_at: now,
  chat_id: "showcase-chat",
};

const buddyRecentErrorsFixture: BuddyRuntimeEvent[] = [
  {
    ...buddyRuntimeEvent,
    id: "recent-error-title-summary",
    title: "Model unavailable",
    failure_summary: "Model unavailable",
    created_at: new Date(Date.now() - 1000).toISOString(),
  },
  ...Array.from({ length: 3 }, (_, index) => ({
    ...buddyRuntimeEvent,
    id: `recent-error-repeated-${index}`,
    title: "Provider unavailable",
    failure_summary: "Provider unavailable — retry later",
    created_at: new Date(Date.now() - 2000).toISOString(),
  })),
  ...Array.from({ length: 25 }, (_, index) => ({
    ...buddyRuntimeEvent,
    id: `recent-error-distinct-${index}`,
    title: `Distinct warning ${index}`,
    failure_summary: `Additional detail ${index}`,
    created_at: new Date(Date.now() - 3000 - index * 1000).toISOString(),
  })),
];

const buddyRecentErrorDiagnostics: DiagnosticContext[] =
  recentErrorsFixtureParam
    ? [
        {
          error_type: "model_unavailable",
          error_message: "Model unavailable",
          source_file: null,
          tool_name: null,
          chat_id: "showcase-chat",
          diagnostic_id: "recent-error-diagnostic-model",
          collected_at: new Date(Date.now() - 1000).toISOString(),
          severity: "high",
        },
        ...Array.from({ length: 3 }, (_, index) => ({
          error_type: "provider_unavailable",
          error_message: "Provider unavailable — retry later",
          source_file: null,
          tool_name: null,
          chat_id: "showcase-chat",
          diagnostic_id: `recent-error-diagnostic-repeat-${index}`,
          collected_at: new Date(Date.now() - 2000).toISOString(),
          severity: "high" as const,
        })),
        ...Array.from({ length: 25 }, (_, index) => ({
          error_type: `distinct_warning_${index}`,
          error_message: `Additional detail ${index}`,
          source_file: null,
          tool_name: null,
          chat_id: "showcase-chat",
          diagnostic_id: `recent-error-diagnostic-distinct-${index}`,
          collected_at: new Date(
            Date.now() - 3000 - index * 1000,
          ).toISOString(),
          severity: "high" as const,
        })),
      ]
    : [];

const buddyConversations: BuddyConversationEntry[] = [
  {
    id: "showcase-buddy-chat",
    kind: "chat",
    title: "Buddy route responsive smoke with a very long title",
    created_at: now,
    updated_at: now,
    status: "completed",
    message_count: 7,
    icon: "💬",
    badge: "Review",
  },
  {
    id: "showcase-buddy-workflow",
    kind: "workflow",
    title: "Autonomous layout watcher",
    created_at: "2026-06-07T08:00:00Z",
    updated_at: "2026-06-07T08:30:00Z",
    status: "completed",
    message_count: 3,
    icon: "🛠️",
    badge: "Workflow",
    workflow_id: "buddy_layout_watcher",
  },
];

const activityFixtureParam = new URLSearchParams(window.location.search).has(
  "activity_fixture",
);
const realConversationsFixtureParam = new URLSearchParams(
  window.location.search,
).has("conversations_fixture");
const realConversationsFixture: BuddyConversationEntry[] = [
  {
    id: "memo_extraction",
    kind: "system",
    title: "Memo extraction",
    created_at: "2026-09-24T14:14:29.963146380+00:00",
    updated_at: "2026-09-24T14:14:29.963146380+00:00",
    status: "failed",
    message_count: 100,
    icon: "🧠",
    badge: "Memo",
  },
  {
    id: "kg_enrich",
    kind: "system",
    title: "Knowledge graph enrichment",
    created_at: "2026-09-24T06:54:38.312631790+00:00",
    updated_at: "2026-09-24T06:54:38.312631790+00:00",
    status: "failed",
    message_count: 100,
    icon: "📚",
    badge: "Knowledge",
  },
  {
    id: "kg_deprecate",
    kind: "system",
    title: "Knowledge cleanup",
    created_at: "2026-09-24T05:54:38.312631790+00:00",
    updated_at: "2026-09-24T05:54:38.312631790+00:00",
    status: "failed",
    message_count: 100,
    icon: "📚",
    badge: "Knowledge",
  },
  {
    id: "commit_message",
    kind: "workflow",
    title: "Commit message generation",
    created_at: "2026-09-22T18:50:59.124159204+00:00",
    updated_at: "2026-09-22T18:50:59.124159204+00:00",
    status: "failed",
    message_count: 100,
    icon: "🔄",
    badge: "Commit Msg",
  },
  {
    id: "55b749c2-540e-45f2-99ce-d852698a4f2a",
    kind: "system",
    title: "Privacy observation degraded",
    created_at: "2026-09-21T12:51:39.536239200+00:00",
    updated_at: "2026-09-21T12:51:39.536239200+00:00",
    status: "active",
    message_count: 1,
    icon: "🔄",
    badge: null,
  },
  {
    id: "title_generation",
    kind: "workflow",
    title: "Title generation",
    created_at: "2026-06-03T05:55:03.452956309+00:00",
    updated_at: "2026-06-03T05:55:03.452956309+00:00",
    status: "completed",
    message_count: 100,
    icon: "🔄",
    badge: null,
  },
];
const buddyActivityConversations: BuddyConversationEntry[] =
  realConversationsFixtureParam
    ? realConversationsFixture
    : activityFixtureParam
      ? Array.from({ length: 60 }, (_, index) => {
          const workflowIndex = Math.floor(index / 12);
          const ids = [
            "refact_error_detective",
            "buddy_pr_issue_matchmaker",
            "kg_deprecate",
            "privacy_observation_degraded",
            "commit_message",
          ];
          const badges = [
            "Error Detective",
            "PR Matchmaker",
            "Knowledge",
            "Privacy observation degraded",
            "Commit message",
          ];
          const id = ids[workflowIndex];
          const badge = badges[workflowIndex];
          const title =
            index % 12 < 2
              ? `${id.replace(/_/g, " ")} (${badge})`
              : `${badge} report ${index}`;
          return {
            id: index % 12 < 2 ? id : `${id}-${index}`,
            kind: "workflow",
            title,
            created_at: now,
            updated_at: now,
            status: "completed",
            message_count: index === 0 ? 1 : 2,
            icon: "⚙️",
            badge,
          };
        })
      : buddyConversations;

const buddySnapshot: BuddySnapshot = {
  state: {
    identity: {
      name: "Pixel",
      created_at: "2026-06-01T00:00:00Z",
      palette_index: 0,
    },
    progression: {
      stage: 1,
      stage_name: "Sprout",
      level: 3,
      xp: 42,
      xp_next: 80,
    },
    skills: { unlocked: ["review", "nudge"], locked: ["teleport"] },
    workflow_summaries: [],
    semantic: {
      mood: "curious",
      focus: "responsive polish",
      headline: "Guarding narrow layouts with tiny claws",
      last_active: now,
    },
    recent_activities: buddyActivities,
    suggestion_state: [
      {
        id: "showcase-suggestion",
        suggestion_type: "layout_check",
        title: "Run a narrow Buddy smoke pass",
        description: "Open settings and filter activity after the route loads.",
        created_at: now,
        dismissed: false,
        controls: [],
        quest: null,
      },
    ],
    pet: {
      needs: {
        hunger: 70,
        energy: 82,
        hygiene: 88,
        boredom: 18,
        affection: 91,
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
        care_score: 12,
        neglect_score: 1,
        open_seconds: 320,
        last_evolved_at: null,
      },
    },
    personality: {
      archetype_id: "chaos_sprite",
      archetype_label: "Chaos Sprite",
      vibe: "Mildly chaotic, cute, and helpful",
      summary: "A tiny gremlin assistant who celebrates completed checks.",
      prompt: "Be warm, curious, and gently mischievous.",
      traits: {
        playfulness: 82,
        chaos: 48,
        sociability: 74,
        curiosity: 88,
        resilience: 69,
      },
    },
    active_quest: null,
    opportunities: [buddyOpportunity],
  },
  settings: buddySettings,
  enabled: true,
  storage: {
    project_root: "/workspace/refact",
    buddy_dir: "/workspace/refact/.refact/buddy",
    settings_path: "/workspace/refact/.refact/buddy/settings.json",
  },
  recent_diagnostics: recentErrorsFixtureParam
    ? buddyRecentErrorDiagnostics
    : [],
  active_speech: sceneSpeechParam
    ? {
        id: "showcase-scene-speech",
        text: "Snack beacon detected.",
        mood: "curious",
        scope: "project",
        persistent: true,
        ttl_seconds: 0,
        created_at: now,
        controls: [],
        speech_intent: "speech_insight",
      }
    : null,
  runtime_queue: recentErrorsFixtureParam
    ? buddyRecentErrorsFixture
    : [buddyRuntimeEvent],
  now_playing: null,
  pulse: buddyPulse,
  opportunities: [buddyOpportunity],
  active_drafts: [],
  chat_reaction_debug: {
    recent_attempts: [],
    counts_by_result: { emitted: 2, skipped: 1 },
    last_skip_reason: null,
    last_emitted_at: now,
  },
};

const showcaseMessages: ChatMessages = [
  {
    role: "user",
    message_id: "showcase-user-1",
    content:
      "Check that expandable tool cards and message footer actions stay responsive.",
  },
  {
    role: "assistant",
    message_id: "showcase-assistant-tools",
    content:
      "I checked two representative tool families so this route can verify expand and collapse interactions.",
    tool_calls: [
      {
        id: "showcase-read-tool",
        index: 0,
        type: "function",
        function: {
          name: "cat",
          arguments: JSON.stringify({
            paths: "src/components/ui/ToolCard.tsx",
          }),
        },
      },
      {
        id: "showcase-exec-tool",
        index: 1,
        type: "function",
        function: {
          name: "process_start",
          arguments: JSON.stringify({
            command: "npm run build",
            description: "Build route showcase",
            mode: "background",
          }),
        },
      },
    ],
    usage: {
      prompt_tokens: 1200,
      completion_tokens: 140,
      total_tokens: 1340,
    },
  },
  {
    role: "tool",
    tool_call_id: "showcase-read-tool",
    content:
      "File src/components/ui/ToolCard.tsx:1-8\n```tsx\nexport function ToolCard() {\n  return <section />;\n}\n```",
    tool_failed: false,
  },
  {
    role: "tool",
    tool_call_id: "showcase-exec-tool",
    content:
      "Process started\nstdout:\nroute showcase ready\nstderr:\n<empty>\n",
    tool_failed: false,
    extra: {
      process_id: "exec_route_showcase",
      status: "running",
      short_description: "Build route showcase",
      command: "npm run build",
      mode: "background",
      cwd: "/workspace/refact-agent/gui",
      started_at_ms: Date.now() - 12_000,
    },
  },
];

const queueEventMessages: ChatMessages = [
  ...showcaseMessages,
  ...(
    [
      ["mode_switch", "chat.mode", "Switched to Agent mode", {}],
      ["tool_decision", "chat.tools", "Tool call approved", { accepted: true }],
      ["ide_callback", "ide.vscode", "IDE returned the open file", {}],
      [
        "process_completed",
        "exec.process",
        "npm run build finished",
        { process_id: "exec_route_showcase", status: "exited", exit_code: 0 },
      ],
      [
        "process_completed",
        "exec.process",
        "npm run lint failed",
        { process_id: "exec_route_lint", status: "failed", exit_code: 1 },
      ],
      [
        "cron_fire",
        "scheduler.cron",
        "Scheduled job fired",
        {
          task_id: "responsive-cron-task",
          status: "fired",
        },
      ],
      ["tick", "chat.ticker", "Tick 42", {}],
      [
        "summarization_marker",
        "chat.summarizer",
        "Older context compacted",
        { status: "applied" },
      ],
      [
        "cancellation_note",
        "chat.session",
        "Generation cancelled by the user",
        { status: "cancelled" },
      ],
      [
        "verifier_report",
        "goal.verifier",
        "Verifier reported remaining gaps",
        { verdict: "needs_work" },
      ],
      ["plan_delta", "chat.plan", "Plan updated with a queue section", {}],
      ["goal_delta", "chat.goal", "Goal narrowed to the queue redesign", {}],
      [
        "goal_pursuit",
        "chat.goal",
        "Pursuit step 3 of 8",
        { status: "active" },
      ],
      [
        "system_notice",
        "agents.spawn",
        "Background agent spawned",
        { status: "ok" },
      ],
      [
        "unknown_future_subkind",
        "engine.future",
        "An event kind this GUI build does not know yet",
        {},
      ],
    ] as const
  )
    .map(([subkind, source, content, payload], index) => ({
      role: "event" as const,
      message_id: `showcase-event-${index}`,
      content,
      event: {
        subkind,
        source,
        payload: { ...payload, created_at_ms: Date.parse(now) + index * 1000 },
      },
    }))
    .flatMap((event) =>
      event.event.subkind === "unknown_future_subkind"
        ? [
            {
              role: "assistant" as const,
              message_id: "queue-event-separator",
              content: "Continuing the active step.",
            },
            event,
          ]
        : [event],
    ),
] as ChatMessages;

const queueItems: ChatThreadRuntime["queued_items"] = [
  {
    client_request_id: "queued-user-1",
    priority: false,
    command_type: "user_message",
    preview: "Also check the dark theme once you are done",
    content: "Also check the dark theme once you are done",
    enqueued_at_ms: Date.parse(now),
  },
  {
    client_request_id: "delivery-agent-1",
    priority: false,
    command_type: "delivery",
    preview: "Child agent finished the engine subset",
    push: "append" as const,
    source: "agents.push",
    event: {
      subkind: "system_notice",
      source: "agents.push",
      payload: { status: "ok", agent_id: "bgagent-showcase" },
    },
    enqueued_at_ms: Date.parse(now) + 1000,
  },
  {
    client_request_id: "delivery-process-1",
    priority: true,
    command_type: "delivery",
    preview: "npm run lint failed with exit code 1",
    push: "preempt" as const,
    source: "exec.process",
    event: {
      subkind: "process_completed",
      source: "exec.process",
      payload: {
        process_id: "exec_route_showcase",
        status: "failed",
        exit_code: 1,
      },
    },
    enqueued_at_ms: Date.parse(now) + 2000,
  },
  {
    client_request_id: "delivery-cron-1",
    priority: false,
    command_type: "delivery",
    preview: "Nightly maintenance sweep is due",
    push: "when_idle" as const,
    source: "scheduler.cron",
    event: {
      subkind: "cron_fire",
      source: "scheduler.cron",
      payload: { task_id: "responsive-cron-task", status: "fired" },
    },
    enqueued_at_ms: Date.parse(now) + 3000,
  },
  {
    client_request_id: "delivery-unlabeled-1",
    priority: false,
    command_type: "delivery",
    preview: "A long single-line continuation: " + "unbroken".repeat(80),
    enqueued_at_ms: Date.parse(now) + 4000,
  },
] satisfies ChatThreadRuntime["queued_items"];

// Opt-in stress data stays in this browser fixture, never production props.
const queueParams = new URLSearchParams(window.location.search);
const queueCommandsFixture = queueParams.get("queue_commands") === "1";
if (queueCommandsFixture) {
  queueItems.push(
    {
      client_request_id: "queued-question-1",
      priority: false,
      command_type: "submit_question_response",
      preview: "Answer: use the dark palette",
      content: "use the dark palette",
      enqueued_at_ms: Date.parse(now) + 5000,
    },
    {
      client_request_id: "queued-regenerate-1",
      priority: true,
      command_type: "regenerate",
      preview: "Regenerate the last answer",
      enqueued_at_ms: Date.parse(now) + 6000,
    },
    {
      client_request_id: "queued-tool-1",
      priority: false,
      command_type: "tool_decision",
      preview: "Approve shell tool call",
      content: "approve",
      enqueued_at_ms: Date.parse(now) + 7000,
    },
  );
}
const queueCount = Math.min(
  100,
  Number(queueParams.get("queue_count")) || queueItems.length,
);
for (let index = queueItems.length; index < queueCount; index++) {
  queueItems.push({
    client_request_id: `delivery-stress-${index + 1}`,
    priority: false,
    command_type: "delivery",
    push: "append",
    source: `agents.${"long_source_identifier_".repeat(12)}`,
    preview: `Stress delivery ${
      index + 1
    }: ${"Long queued content with meaningful details. ".repeat(
      80,
    )}${"unbroken".repeat(80)}`,
    enqueued_at_ms: Date.parse(now) + index * 1000,
  });
}

const chatRuntime: ChatThreadRuntime = {
  thread: {
    id: chatId,
    title: "Responsive route showcase chat",
    createdAt: now,
    updatedAt: now,
    model: "gpt-4o",
    tool_use: "agent",
    messages: showcaseMessages,
    new_chat_suggested: { wasSuggested: false },
  },
  streaming: false,
  waiting_for_response: false,
  prevent_send: false,
  error: null,
  queued_items: [],
  send_immediately: false,
  attached_images: [],
  attached_text_files: [],
  background_agents: {},
  confirmation: {
    pause: false,
    pause_reasons: [],
    status: { wasInteracted: false, confirmationStatus: true },
  },
  snapshot_received: true,
  task_widget_expanded: false,
  memory_enrichment_user_touched: false,
  manual_preview_items: [],
  manual_preview_ran: false,
};

const queueChatRuntime: ChatThreadRuntime = {
  ...chatRuntime,
  thread: {
    ...chatRuntime.thread,
    title: "Delivery queue and event history",
    messages: [
      ...queueEventMessages,
      {
        role: "assistant",
        message_id: "queue-partial",
        content: "Partial response from the active step…",
      },
      {
        role: "event",
        message_id: "queue-single-event",
        subkind: "system_notice",
        source: "chat.runtime",
        content: "Waiting for the next tool result",
      },
    ],
  },
  streaming: true,
  session_state: "generating",
  queued_items: queueItems,
  waiting_interruptible: queueParams.get("waiting") === "1",
};

const rebuildRoute = new URLSearchParams(window.location.search).has("rebuild");
const rebuildReport = (id: string, content: string): ChatMessages[number] => ({
  role: "compression_report",
  message_id: id,
  content: "Context rebuilt",
  compression_report: {
    kind: "reconstructed_history",
    schema_version: 1,
    model: "fixture-model",
    trigger: "manual",
    payload: {
      messages: [{ role: "user", message_id: `${id}-payload`, content }],
    },
  },
});
if (rebuildRoute) {
  chatRuntime.thread.title = "Rebuilt archive fixture";
  chatRuntime.thread.model = "openai_codex_personal/gpt-5.5";
  chatRuntime.thread.messages = [
    {
      role: "user",
      message_id: "original",
      content: "Original archive sentinel",
    },
    rebuildReport("report-first", "Earlier rebuilt sentinel"),
    { role: "user", message_id: "middle", content: "Between reports sentinel" },
    rebuildReport("report-latest", "Latest rebuilt sentinel"),
    { role: "user", message_id: "suffix", content: "After rebuild sentinel" },
  ];
}

const splitChatRuntime: ChatThreadRuntime = {
  ...chatRuntime,
  thread: {
    ...chatRuntime.thread,
    id: splitChatId,
    title: "Responsive split pane companion",
    messages: showcaseMessages,
  },
};

if (rebuildRoute) {
  splitChatRuntime.thread.title = "Legacy archive fixture";
  splitChatRuntime.thread.messages = [
    {
      role: "user",
      message_id: "legacy-original",
      content: "Legacy original sentinel",
    },
    {
      role: "assistant",
      message_id: "legacy-summary",
      content: "Legacy summary sentinel",
      extra: { compression: { kind: "llm_segment_summary" } },
    },
  ];
}

const historyItem = (
  id: string,
  title: string,
  updatedAt: string,
): ChatHistoryItem => ({
  id,
  title,
  createdAt: updatedAt,
  updatedAt,
  model: "gpt-4o",
  messages: [],
  new_chat_suggested: { wasSuggested: false },
  message_count: 3,
  mode: "agent",
});

const tasks: TaskMeta[] = [
  {
    id: "task-planning",
    name: "Responsive design-system migration",
    status: "planning",
    created_at: now,
    updated_at: now,
    cards_total: 37,
    cards_done: 5,
    cards_failed: 0,
    agents_active: 1,
  },
  {
    id: "task-active",
    name: "Token layer audit",
    status: "active",
    created_at: now,
    updated_at: now,
    cards_total: 8,
    cards_done: 3,
    cards_failed: 1,
    agents_active: 2,
  },
];

const settingsSectionParam = new URLSearchParams(window.location.search).get(
  "settings",
);
const settingsSectionIds = new Set<string>(
  SETTINGS_SECTIONS.map((section) => section.id),
);
const settingsSection: SettingsSectionId = settingsSectionIds.has(
  settingsSectionParam ?? "",
)
  ? (settingsSectionParam as SettingsSectionId)
  : "general";
const marketplaceTabParam = new URLSearchParams(window.location.search).get(
  "marketplace",
);
const marketplaceTabs = new Set<string>([
  "skills",
  "commands",
  "subagents",
  "mcp",
  "extensions",
]);
const marketplaceTab: MarketplaceTabId = marketplaceTabs.has(
  marketplaceTabParam ?? "",
)
  ? (marketplaceTabParam as MarketplaceTabId)
  : "skills";

const providerList: ConfiguredProvidersResponse = {
  providers: [
    {
      name: "openai_codex_personal",
      base_provider: "openai",
      display_name: "OpenAI Codex Personal With A Very Long Display Name",
      enabled: true,
      readonly: false,
      has_credentials: true,
      status: "active",
      model_count: 12,
    },
    {
      name: "local_experimental_provider_with_long_name",
      base_provider: "openai_compatible",
      display_name: "Local Experimental Provider With Long Name",
      enabled: false,
      readonly: false,
      has_credentials: false,
      status: "configured",
      model_count: 3,
    },
  ],
  error_log: [],
};

const chatModel = (id: string) => ({
  n_ctx: 128000,
  name: id,
  tokenizer: "cl100k_base",
  id,
  supports_tools: true,
  supports_multimodality: true,
  supports_clicks: false,
  supports_agent: true,
  reasoning_effort_options: ["low", "medium", "high"],
  supports_thinking_budget: true,
  supports_adaptive_thinking_budget: false,
  default_temperature: 0.2,
  enabled: true,
  type: "chat" as const,
});

const caps: CapsResponse = {
  caps_version: 1,
  chat_default_model: "openai_codex_personal/gpt-5.5",
  chat_model_2: "openai_codex_personal/gpt-5.5-mini",
  task_planner_agent_model: "openai_codex_personal/gpt-5.5-planner",
  chat_thinking_model: "openai_codex_personal/gpt-5.5-thinking",
  chat_light_model:
    "local_experimental_provider_with_long_name/tiny-but-useful-model",
  chat_buddy_model: "openai_codex_personal/gpt-5.5-buddy",
  chat_models: {
    "openai_codex_personal/gpt-5.5": chatModel("openai_codex_personal/gpt-5.5"),
    "openai_codex_personal/gpt-5.5-mini": chatModel(
      "openai_codex_personal/gpt-5.5-mini",
    ),
    "openai_codex_personal/gpt-5.5-planner": chatModel(
      "openai_codex_personal/gpt-5.5-planner",
    ),
    "openai_codex_personal/gpt-5.5-thinking": chatModel(
      "openai_codex_personal/gpt-5.5-thinking",
    ),
    "openai_codex_personal/gpt-5.5-buddy": chatModel(
      "openai_codex_personal/gpt-5.5-buddy",
    ),
    "local_experimental_provider_with_long_name/tiny-but-useful-model":
      chatModel(
        "local_experimental_provider_with_long_name/tiny-but-useful-model",
      ),
  },
  code_chat_default_system_prompt: "",
  completion_models: {},
  completion_default_model: "",
  code_completion_n_ctx: 8192,
  endpoint_chat_passthrough: "",
  endpoint_style: "openai",
  endpoint_template: "",
  running_models: [],
  tokenizer_path_template: "",
  tokenizer_rewrite_path: {},
  metadata: { pricing: {} },
  customization: "",
};

const defaults: ProviderDefaults = {
  chat: { model: "openai_codex_personal/gpt-5.5", max_new_tokens: 4096 },
  chat_model_2: { model: "openai_codex_personal/gpt-5.5-mini" },
  task_planner_agent_model: {
    model: "openai_codex_personal/gpt-5.5-planner",
  },
  chat_light: {
    model: "local_experimental_provider_with_long_name/tiny-but-useful-model",
  },
  chat_thinking: { model: "openai_codex_personal/gpt-5.5-thinking" },
  chat_buddy: { model: "openai_codex_personal/gpt-5.5-buddy" },
};

const overlayRegressionModels: ModelOption[] = [
  {
    value: "openai_codex_personal/gpt-5.5",
    displayName: "OpenAI Codex Personal GPT 5.5",
    group: "openai_codex_personal",
    badges: ["default"],
    contextWindow: "128K ctx",
  },
  {
    value: "openai_codex_personal/gpt-5.5-planner",
    displayName: "Task Planner GPT 5.5",
    group: "openai_codex_personal",
    badges: ["task-agent"],
    contextWindow: "128K ctx",
  },
  {
    value: "local_experimental_provider_with_long_name/tiny-but-useful-model",
    displayName: "Tiny Useful Local Model",
    group: "local_experimental_provider_with_long_name",
    badges: ["light"],
    contextWindow: "32K ctx",
  },
];

const overlayRegressionModelGroups = [
  { id: "openai_codex_personal", label: "OpenAI Codex Personal" },
  {
    id: "local_experimental_provider_with_long_name",
    label: "Local Experimental Provider",
  },
];

const chatModesResponse = {
  modes: [
    {
      id: "agent",
      title: "Agent",
      description: "Autonomous coding mode",
      tools_count: 12,
      thread_defaults: {
        include_project_info: true,
        checkpoints_enabled: true,
        auto_approve_editing_tools: false,
        auto_approve_dangerous_commands: false,
      },
      ui: { order: 1, tags: ["editing", "tools"] },
    },
    {
      id: "ask",
      title: "Ask",
      description: "Quick answers without edits",
      tools_count: 0,
      thread_defaults: {
        include_project_info: true,
        checkpoints_enabled: false,
        auto_approve_editing_tools: false,
        auto_approve_dangerous_commands: false,
      },
      ui: { order: 2, tags: ["chat"] },
    },
  ],
  errors: [],
};

const configItem = (
  kind: ConfigItem["kind"],
  id: string,
  title: string,
): ConfigItem => ({
  id,
  kind,
  title,
  file_path: `/tmp/${id}.yaml`,
  specific: false,
  scope: "global",
  global_path: `/tmp/${id}.yaml`,
  local_path: "",
  global_exists: true,
  local_exists: false,
});

const registry = {
  modes: [
    configItem(
      "modes",
      "very_long_mode_name_for_responsive_testing",
      "Very Long Mode Name For Responsive Testing",
    ),
  ],
  subagents: [
    configItem(
      "subagents",
      "responsive_subagent_with_long_identifier",
      "Responsive Subagent With Long Identifier",
    ),
  ],
  toolbox_commands: [
    configItem(
      "toolbox_commands",
      "long_toolbox_command_name_that_should_truncate",
      "Long Toolbox Command Name That Should Truncate",
    ),
  ],
  code_lens: [
    configItem(
      "code_lens",
      "long_code_lens_action_name",
      "Long Code Lens Action",
    ),
  ],
  errors: [],
  has_project_root: true,
};

const integrations: IntegrationWithIconResponse = {
  integrations: [
    {
      project_path: "",
      integr_name: "github_enterprise_with_a_very_long_name",
      icon_path: "/integrations/github.svg",
      integr_config_path: ".config/refact/integrations/github_enterprise.yaml",
      integr_config_exists: true,
      on_your_laptop: true,
      when_isolated: true,
    },
    {
      project_path: "/workspace/very/long/project/path/for/responsive/testing",
      integr_name: "mcp_TEMPLATE",
      icon_path: "/integrations/mcp.svg",
      integr_config_path: "/workspace/.refact/integrations/mcp_TEMPLATE.yaml",
      integr_config_exists: false,
      on_your_laptop: true,
      when_isolated: true,
    },
  ],
  error_log: [],
};

const extensions: ExtRegistryResponse = {
  skills: [
    {
      name: "responsive-skill-with-a-very-long-name",
      description:
        "A long skill description that should wrap inside the settings hub.",
      source: "/tmp/skills/responsive.md",
      source_label: "global",
      scope: "global",
      read_only: false,
      file_path: "/tmp/skills/responsive.md",
    },
  ],
  slash_commands: [
    {
      name: "responsive-command-with-a-very-long-name",
      description:
        "A long command description that should wrap inside the settings hub.",
      source: "/tmp/commands/responsive.md",
      source_label: "global",
      scope: "global",
      read_only: false,
      file_path: "/tmp/commands/responsive.md",
    },
  ],
  hooks: [
    {
      event: "on_session_start_with_long_event_name",
      command: "echo responsive settings hook",
      source: "/tmp/hooks.yaml",
      source_label: "global",
      scope: "global",
      read_only: false,
    },
  ],
  has_project_root: true,
};

const marketplaceSources = [
  {
    id: "refact-starter",
    label: "Refact Starter Source With Long Name",
    description: "Bundled marketplace entries for responsive testing.",
    enabled: true,
    builtin: true,
    removable: false,
    source_kind: "builtin_embedded",
    supported_kinds: ["skill", "command", "subagent"],
    parser_mode: "scan",
    item_count: 3,
  },
];

const extensionMarketplaceItem = (kind: "skill" | "command" | "subagent") => ({
  id: `responsive-${kind}`,
  name: `Responsive ${kind} marketplace item with long name`,
  description:
    "A marketplace item with enough descriptive text to prove the toolbar and cards wrap cleanly on narrow screens.",
  tags: ["responsive", "layout", "marketplace"],
  publisher: "Refact",
  kind,
  source_id: "refact-starter",
  source_label: "Refact Starter Source With Long Name",
  path: `${kind}s/responsive`,
  installed_scopes: [],
});

const mcpMarketplace = {
  servers: [
    {
      id: "responsive-mcp-server",
      source_id: "refact-bundled",
      name: "Responsive MCP Server With Long Name",
      description:
        "A representative MCP server card used by the route showcase responsiveness check.",
      publisher: "Refact",
      tags: ["responsive", "tools", "marketplace"],
      transport: "stdio",
      install_recipe: { command: "npx responsive-mcp" },
      confirmation_default: [],
    },
  ],
  sources: [
    {
      id: "refact-bundled",
      label: "Refact Bundled MCP Registry With Long Name",
      type: "refact_index",
      enabled: true,
      removable: false,
      server_count: 1,
      status: "ok",
    },
  ],
  pagination: { page: 1, page_size: 20, total: 1 },
};

const pluginMarketplaces = {
  marketplaces: [
    {
      name: "responsive-plugins",
      source: "JegernOUTT/refact-plugins-responsive-fixture",
      added_at: now,
    },
  ],
};

const pluginList = {
  plugins: [
    {
      name: "responsive-plugin-with-a-very-long-name",
      description:
        "A plugin marketplace item with long text for the marketplace route showcase.",
      version: "1.0.0",
      tags: ["responsive", "plugin"],
      marketplace: "responsive-plugins",
    },
  ],
};

const cronTasks = [
  {
    id: "responsive-cron-task",
    cron: "*/15 * * * *",
    human_schedule: "Every fifteen minutes with a long schedule description",
    description:
      "Responsive scheduler task with a deliberately long description",
    prompt: "Check responsive settings state",
    recurring: true,
    durable: true,
    next_fire_at_ms: Date.now() + 900000,
    fire_count: 42,
    created_at_ms: Date.now() - 3600000,
    enabled: true,
    paused: false,
    trigger_kind: "cron",
    tz: "UTC",
    every_ms: null,
    at_ms: null,
    last_status: "fired",
    last_error: null,
    recent_runs: [
      {
        at_ms: Date.now() - 1800000,
        status: "fired",
        error: null,
      },
    ],
    action_kind: "agent_turn",
    delivery: { kind: "chat" },
    chat_id: chatId,
    target: "existing_chat",
    isolated: false,
  },
];

const privacyFile: PrivacyFileRecord = {
  path: ".env",
  zone: "secrets",
  attribution: "observed",
};

const privacyPolicyResponse: PrivacyPolicyResponse = {
  policy: {
    blocked: ["*.blocked"],
    zones: [
      {
        name: "secrets",
        patterns: [".env*"],
        send_to: ["trusted"],
        on_shell_read: "withhold",
      },
      {
        name: "normal",
        patterns: ["**"],
        send_to: ["*"],
        on_shell_read: "ask",
      },
    ],
    subagents: { report_declassifies: true },
    tool_access: { providers: {} },
  },
  destinations: [
    {
      id: "trusted",
      kind: "provider",
      display_name: "Trusted provider",
    },
    {
      id: "build-mcp",
      kind: "mcp",
      display_name: "Build MCP",
    },
  ],
  match_counts: { secrets: 2, normal: 7 },
  error: null,
  source_paths: ["/workspace/refact/.refact/privacy.yaml"],
};

const privacyStatusResponse: PrivacyStatusResponse = {
  platform: "linux",
  observation: {
    platform_supported: true,
    runtime_available: false,
    last_error: "PTRACE_TRACEME is unavailable",
  },
  config_error: null,
};

const privacyInspectResponse: PrivacyInspectResponse = {
  chat_id: chatId,
  destination: {
    id: "untrusted",
    kind: "provider",
    display_name: "untrusted/model",
  },
  sendable: false,
  records: [privacyFile],
  blocked: [{ record_index: 0, record: privacyFile }],
  refusal: "destination untrusted cannot receive message 0 path .env",
};

function jsonResponse(data: unknown) {
  return new Response(JSON.stringify(data), {
    headers: { "Content-Type": "application/json" },
  });
}

let organizationRevision = 1;
const organizationTopics: Taxon[] = [
  { id: "topic-gui", name: "Interface", archived: false },
  { id: "topic-engine", name: "Engine", archived: false },
];
const organizationTags: Taxon[] = [
  { id: "tag-review", name: "Review", archived: false },
  { id: "tag-tests", name: "Tests", archived: false },
  { id: "tag-polish", name: "Polish", archived: false },
];
const organizationAssignments = new Map<string, Assignment>();
const nativeFetch = window.fetch.bind(window);
const showcaseCommandLog: unknown[] = [];
const showcaseClipboardWrites: string[] = [];
const showcasePrivacyActions: string[] = [];
type ShowcaseRequest = {
  path: string;
  method: string;
  body: unknown;
};
const showcaseRequests: ShowcaseRequest[] = [];
let privacyPolicy = privacyPolicyResponse.policy;
(
  window as unknown as { __routeShowcaseCommands: unknown[] }
).__routeShowcaseCommands = showcaseCommandLog;
(
  window as unknown as { __routeShowcaseClipboardWrites: string[] }
).__routeShowcaseClipboardWrites = showcaseClipboardWrites;
(
  window as unknown as { __routeShowcaseRequests: ShowcaseRequest[] }
).__routeShowcaseRequests = showcaseRequests;
(
  window as unknown as { __routeShowcasePrivacyActions: string[] }
).__routeShowcasePrivacyActions = showcasePrivacyActions;
Object.defineProperty(window.navigator, "clipboard", {
  configurable: true,
  value: {
    writeText: (text: string) => {
      showcaseClipboardWrites.push(text);
      return Promise.resolve();
    },
  },
});
window.fetch = async (input: RequestInfo | URL, init?: RequestInit) => {
  const url = new URL(
    typeof input === "string"
      ? input
      : input instanceof URL
        ? input.toString()
        : input.url,
    window.location.origin,
  );
  const path = url.pathname;
  const method =
    init?.method ?? (input instanceof Request ? input.method : "GET");
  let body: unknown = null;
  const requestBody = init?.body
    ? String(init.body)
    : input instanceof Request
      ? await input.clone().text()
      : "";
  if (requestBody) {
    try {
      body = JSON.parse(requestBody);
    } catch {
      body = requestBody;
    }
  }
  showcaseRequests.push({ path, method, body });
  if (path === "/v1/organization/taxonomy")
    return jsonResponse({
      revision: organizationRevision,
      items:
        url.searchParams.get("kind") === "topics"
          ? organizationTopics
          : organizationTags,
      next_cursor: null,
    });
  if (path === "/v1/organization/items") {
    const { items } = body as { items: OrganizationItem[] };
    return jsonResponse({
      revision: organizationRevision,
      assignments: items.map((item, index) => {
        const key = `${item.kind}:${item.id}`;
        const assignment = organizationAssignments.get(key) ?? {
          item,
          topic_id: index % 3 === 0 ? null : organizationTopics[index % 2].id,
          tag_ids: index % 2 ? organizationTags.map((tag) => tag.id) : [],
          topic_locked: false,
          tags_locked: false,
        };
        organizationAssignments.set(key, assignment);
        return assignment;
      }),
    });
  }
  if (path === "/v1/organization/apply") {
    const payload = body as {
      expected_revision: number;
      operations: OrganizationOperation[];
    };
    if (payload.expected_revision !== organizationRevision)
      return new Response("Organization changed", { status: 409 });
    const results = payload.operations.map((operation, op_index) => {
      if (operation.op === "assign_item") {
        organizationAssignments.set(
          `${operation.item.kind}:${operation.item.id}`,
          operation,
        );
        return { op_index };
      }
      const id = `created-${organizationRevision}-${op_index}`;
      (operation.op === "create_topic"
        ? organizationTopics
        : organizationTags
      ).push({ id, name: operation.name, archived: false });
      return { op_index, id };
    });
    organizationRevision++;
    return jsonResponse({ revision: organizationRevision, results });
  }

  if (path === "/v1/chats/activity") {
    return jsonResponse({ chats: homeActivityRoute ? homeActivityChats : [] });
  }
  if (path.startsWith("/v1/chats/")) {
    if (init?.body) {
      try {
        showcaseCommandLog.push(JSON.parse(String(init.body)));
      } catch {
        showcaseCommandLog.push(init.body);
      }
    }
    if (queueRoute && path.endsWith("/commands")) {
      if (queueParams.get("queue_pending") === "1") {
        await new Promise<void>((resolve) => {
          Object.assign(window, { __releaseQueueRequest: resolve });
        });
      }
      if (queueParams.get("queue_error") === "1") {
        return new Response("Queue update failed", { status: 500 });
      }
    }
    const routeRejection = queueCommandsFixture
      ? showcaseQueueRouteRejection(path, method, body)
      : null;
    if (routeRejection) {
      showcaseQueueRouteRejections.push(routeRejection);
      return new Response(routeRejection, { status: 409 });
    }
    applyShowcaseQueueMutation(path, method, body);
    return Promise.resolve(jsonResponse({ ok: true }));
  }
  if (path === "/v1/files/tree") {
    return Promise.resolve(
      jsonResponse({
        path: url.searchParams.get("path") ?? "",
        entries: [
          {
            name: "src",
            path: "/workspace/refact/src",
            kind: "dir",
            size: null,
          },
          {
            name: "README.md",
            path: "/workspace/refact/README.md",
            kind: "file",
            size: 1024,
          },
        ],
        truncated: false,
      }),
    );
  }
  if (path === "/v1/git/file") {
    const rev = url.searchParams.get("rev") ?? "HEAD";
    const filePath = url.searchParams.get("path") ?? "";
    const content =
      rev === "WORKTREE" ? showcaseDiffWorktree : showcaseDiffHead;
    return Promise.resolve(
      jsonResponse({
        root: "/workspace/refact",
        path: filePath,
        rev,
        exists: true,
        binary: false,
        truncated: false,
        size: content.length,
        content,
      }),
    );
  }
  if (path === "/v1/git/status") {
    return Promise.resolve(
      jsonResponse({
        roots: [
          workspaceGitActionsRoute
            ? showcaseGitActionsStatus
            : showcaseGitStatus,
        ],
      }),
    );
  }
  if (path === "/v1/git/log" && workspaceGitActionsRoute) {
    const limit = Number(url.searchParams.get("limit") ?? "1");
    return Promise.resolve(
      jsonResponse({
        roots: [
          {
            root: "/workspace/refact",
            commits: showcaseLogCommits.slice(0, limit),
            head_oid: showcaseOid("M30"),
          },
        ],
      }),
    );
  }
  if (
    path.startsWith("/v1/git/branch/") ||
    /^\/v1\/git\/(fetch|pull|push)$/.test(path)
  ) {
    return Promise.resolve(
      jsonResponse({
        ok: true,
        summary: "Already up to date",
        output: "",
        branch: "main",
        head_oid: showcaseOid("M30"),
        detached: false,
        name: "main",
        tip_oid: showcaseOid("M30"),
        checked_out: true,
        deleted: true,
      }),
    );
  }
  if (path === "/v1/git/log" && workspaceGitLogRoute) {
    const skip = Number(url.searchParams.get("skip") ?? "0");
    const text = (url.searchParams.get("text") ?? "").toLowerCase();
    const commits = showcaseLogCommits.filter(
      (commit) =>
        !text ||
        commit.message.toLowerCase().includes(text) ||
        commit.oid.startsWith(text),
    );
    return Promise.resolve(
      jsonResponse({
        roots: [
          {
            root: "/workspace/refact",
            commits: skip === 0 ? commits : [],
            head_oid: showcaseOid("M30"),
            scan_truncated: false,
          },
        ],
      }),
    );
  }
  if (path === "/v1/git/branches") {
    return Promise.resolve(jsonResponse({ roots: [showcaseBranches] }));
  }
  if (path === "/v1/git/commit" && method === "GET") {
    return Promise.resolve(
      jsonResponse(showcaseCommitDetails(url.searchParams.get("oid") ?? "")),
    );
  }
  if (path === "/v1/git/log") {
    return Promise.resolve(
      jsonResponse({
        roots: [
          {
            root: "/workspace/refact",
            commits: [
              {
                oid: "9f1c2ab3d4e5f60718293a4b5c6d7e8f90a1b2c3",
                short_oid: "9f1c2ab",
                time_ms: Date.parse(now),
                author_name: "Ada",
                author_email: "ada@example.com",
                message_first_line: "Add commit tool window",
                message: "Add commit tool window",
              },
            ],
          },
        ],
      }),
    );
  }
  if (path === "/v1/exec/list") {
    if (terminalDisabledRoute) {
      return Promise.resolve(
        new Response("exec HTTP is disabled", { status: 403 }),
      );
    }
    return Promise.resolve(jsonResponse({ processes: [] }));
  }
  if (path === "/v1/worktrees") {
    return Promise.resolve(
      jsonResponse({
        project_hash: "route-showcase",
        source_workspace_root: "/workspace/refact",
        source_current_branch: "main",
        source_branches: ["main"],
        worktrees: [],
      }),
    );
  }
  if (path === "/v1/voice/status") {
    return Promise.resolve(
      jsonResponse({
        enabled: false,
        model_loaded: false,
        model_name: "",
        is_downloading: false,
        download_progress: 0,
      }),
    );
  }
  if (path === "/v1/at-command-completion") {
    return Promise.resolve(
      jsonResponse({
        completions: [],
        replace: [0, 0],
        is_cmd_executable: false,
      }),
    );
  }
  if (path === "/v1/buddy/operator/status") {
    return Promise.resolve(jsonResponse(buddyOperatorStatus));
  }
  if (path === "/v1/buddy/operator/reopen") {
    return Promise.resolve(jsonResponse(buddyOperatorStatus));
  }
  if (path === "/v1/buddy") return Promise.resolve(jsonResponse(buddySnapshot));
  if (path === "/v1/buddy/settings") {
    if (init?.method === "POST" && init.body) {
      const patch = JSON.parse(String(init.body)) as Partial<BuddySettings>;
      return Promise.resolve(jsonResponse({ ...buddySettings, ...patch }));
    }
    return Promise.resolve(jsonResponse(buddySettings));
  }
  if (path === "/v1/buddy/opportunities") {
    return Promise.resolve(jsonResponse({ opportunities: [buddyOpportunity] }));
  }
  if (path === "/v1/buddy/conversations") {
    if (init?.method === "POST") {
      return Promise.resolve(
        jsonResponse({
          chat_id: "showcase-new-buddy-chat",
          title: "New Buddy showcase chat",
          created_at: now,
        }),
      );
    }
    return Promise.resolve(jsonResponse(buddyActivityConversations));
  }
  if (path === "/v1/buddy/pulse")
    return Promise.resolve(jsonResponse(buddyPulse));
  if (path.startsWith("/v1/buddy/runtime/")) {
    return Promise.resolve(jsonResponse({ dismissed: true }));
  }
  if (path.startsWith("/v1/buddy/opportunities/")) {
    return Promise.resolve(
      jsonResponse({
        snapshot: buddySnapshot,
        action_result: { kind: "dismiss" },
      }),
    );
  }
  if (path.startsWith("/v1/buddy/care")) {
    return Promise.resolve(
      jsonResponse({ message: "ok", snapshot: buddySnapshot }),
    );
  }
  if (path === "/v1/ping") return Promise.resolve(new Response("pong"));
  if (path === "/v1/setup/status") {
    return Promise.resolve(
      jsonResponse({
        configured: true,
        reasons: [],
        detail: {
          project_root: "/workspace/refact",
          has_agents_md: true,
          has_knowledge: true,
          has_trajectories: true,
        },
      }),
    );
  }
  if (path === "/v1/stats/llm/summary") {
    return Promise.resolve(
      jsonResponse({
        date_range: { from: "2026-06-01", to: "2026-06-07" },
        totals: {
          total_calls: 42,
          successful_calls: 39,
          failed_calls: 3,
          total_prompt_tokens: 12000,
          total_completion_tokens: 4000,
          total_tokens: 16000,
          total_cache_read_tokens: 0,
          total_cache_creation_tokens: 0,
          total_cost_usd: 0.12,
          total_duration_ms: 42000,
          avg_duration_ms: 1000,
          total_conversations: 8,
          total_messages_sent: 55,
        },
        by_model: [],
        by_provider: [],
        by_day: [],
        by_mode: [],
        top_conversations: [],
      }),
    );
  }
  if (path === "/v1/providers")
    return Promise.resolve(jsonResponse(providerList));
  if (path === "/v1/chat-modes")
    return Promise.resolve(jsonResponse(chatModesResponse));
  if (path === "/v1/caps") return Promise.resolve(jsonResponse(caps));
  if (path === "/v1/defaults") return Promise.resolve(jsonResponse(defaults));
  if (path === "/v1/customization/registry") {
    return Promise.resolve(jsonResponse(registry));
  }
  if (path === "/v1/integrations")
    return Promise.resolve(jsonResponse(integrations));
  if (path === "/v1/scheduler/cron")
    return Promise.resolve(jsonResponse(cronTasks));
  if (path === "/v1/docs-list") {
    return Promise.resolve(
      jsonResponse([
        {
          url: "https://docs.example.com/a/very/long/documentation/source/url",
          max_depth: 3,
          max_pages: 25,
          pages: { index: "Index", quickstart: "Quickstart" },
        },
      ]),
    );
  }
  if (path === "/v1/ext/registry")
    return Promise.resolve(jsonResponse(extensions));
  if (path === "/v1/skills/marketplace") {
    return Promise.resolve(
      jsonResponse({
        items: [extensionMarketplaceItem("skill")],
        sources: marketplaceSources,
      }),
    );
  }
  if (path === "/v1/commands/marketplace") {
    return Promise.resolve(
      jsonResponse({
        items: [extensionMarketplaceItem("command")],
        sources: marketplaceSources,
      }),
    );
  }
  if (path === "/v1/subagents/marketplace") {
    return Promise.resolve(
      jsonResponse({
        items: [extensionMarketplaceItem("subagent")],
        sources: marketplaceSources,
      }),
    );
  }
  if (path === "/v1/mcp/marketplace")
    return Promise.resolve(jsonResponse(mcpMarketplace));
  if (path === "/v1/mcp/marketplace/installed")
    return Promise.resolve(jsonResponse({ installed: [] }));
  if (path === "/v1/plugins/marketplaces")
    return Promise.resolve(jsonResponse(pluginMarketplaces));
  if (path === "/v1/plugins/installed")
    return Promise.resolve(jsonResponse({ installed: [] }));
  if (path === "/v1/plugins/marketplace/responsive-plugins/plugins")
    return Promise.resolve(jsonResponse(pluginList));
  if (path === "/v1/privacy/policy") {
    if (method === "POST" && body && typeof body === "object") {
      privacyPolicy = body as PrivacyPolicyResponse["policy"];
    }
    return Promise.resolve(
      jsonResponse({ ...privacyPolicyResponse, policy: privacyPolicy }),
    );
  }
  if (path === "/v1/privacy/status")
    return Promise.resolve(jsonResponse(privacyStatusResponse));
  if (path === "/v1/privacy/inspect")
    return Promise.resolve(jsonResponse(privacyInspectResponse));
  if (rebuildRoute) return jsonResponse({});
  return nativeFetch(input, init);
};

const preloadedState: Partial<RootState> = {
  config: {
    host: "web",
    lspPort: 8001,
    lspUrl: "",
    dev: true,
    engineServed: false,
    apiKey: null,
    features: { statistics: true, vecdb: true, ast: true, images: true },
    themeProps: {
      appearance:
        new URLSearchParams(window.location.search).get("theme") === "light"
          ? "light"
          : "dark",
    },
    shiftEnterToSubmit: false,
  },
  chat: {
    current_thread_id: route === "chat-split" ? splitChatId : chatId,
    open_thread_ids: multiChatRoute ? [chatId, splitChatId] : [chatId],
    threads: multiChatRoute
      ? { [chatId]: chatRuntime, [splitChatId]: splitChatRuntime }
      : { [chatId]: queueRoute ? queueChatRuntime : chatRuntime },
    max_new_tokens: 4096,
    tool_use: "agent",
    system_prompt: {},
    sse_refresh_requested: null,
    stream_version: 0,
  },
  ...(chatRoute || chatDndRoute
    ? {
        workspace:
          route === "chat-split"
            ? {
                tabs: [chatSurface(chatId)],
                activeTabId: chatSurface(chatId),
                groups: {
                  [chatSurface(chatId)]: {
                    root: {
                      kind: "split" as const,
                      id: "root:split:row",
                      dir: "row" as const,
                      sizes: [0.5, 0.5],
                      children: [
                        {
                          kind: "leaf" as const,
                          id: "root",
                          tabIds: [chatSurface(chatId)],
                          activeTabId: chatSurface(chatId),
                        },
                        {
                          kind: "leaf" as const,
                          id: `root:sibling:${chatSurface(chatId)}`,
                          tabIds: [chatSurface(splitChatId)],
                          activeTabId: chatSurface(splitChatId),
                        },
                      ],
                    },
                    focusedLeafId: `root:sibling:${chatSurface(chatId)}`,
                  },
                },
              }
            : {
                tabs:
                  chatDndRoute && rebuildRoute
                    ? [chatSurface(chatId), chatSurface(splitChatId)]
                    : [chatSurface(chatId)],
                activeTabId: chatSurface(chatId),
                // Current workspaces consolidate open surfaces into one group.
                // Seed DnD at that valid single-leaf state, not legacy top tabs.
                groups:
                  chatDndRoute && !rebuildRoute
                    ? {
                        [chatSurface(chatId)]: {
                          root: {
                            kind: "leaf" as const,
                            id: "root",
                            tabIds: [
                              chatSurface(chatId),
                              chatSurface(splitChatId),
                            ],
                            activeTabId: chatSurface(chatId),
                          },
                          focusedLeafId: "root",
                        },
                      }
                    : {},
                ...(queueRoute
                  ? {
                      dock: {
                        open: false,
                        width: 280,
                        section: "files" as const,
                      },
                    }
                  : {}),
                ...(workspaceChromeRoute
                  ? {
                      dock: workspaceGitRoute
                        ? {
                            open: !workspaceGitLogRoute,
                            width: 420,
                            section: "git" as const,
                          }
                        : {
                            open: true,
                            width: 280,
                            section: "files" as const,
                          },
                      drawer: { open: !workspaceGitRoute, height: 280 },
                    }
                  : {}),
              },
      }
    : {}),
  ...(workspaceChromeRoute
    ? {
        terminal: {
          sessionsByChat: {},
          activeProcessIdByChat: {},
          workbenchOpenByChat: { [chatId]: true },
        },
      }
    : {}),
  history: {
    chats: {
      alpha: historyItem("alpha", "Dashboard route smoke coverage", now),
      beta: historyItem(
        "beta",
        "Chat surface responsive check",
        "2026-06-06T10:00:00Z",
      ),
    },
    isLoading: false,
    loadError: null,
    pagination: { cursor: null, hasMore: false, totalCount: 2, generation: 1 },
  },
  pages: [{ name: "history" }],
};

const toolbarAlignmentRoute = new URLSearchParams(window.location.search).has(
  "toolbar_alignment",
);
if (toolbarAlignmentRoute) {
  const leftTabs = [chatId, "alignment-middle", "alignment-last"];
  const activeIndex = Number(
    new URLSearchParams(window.location.search).get("active") ?? "2",
  );
  const activeSurface = chatSurface(leftTabs[activeIndex] ?? chatId);
  preloadedState.pages = [{ name: "chat" }];
  preloadedState.chat = {
    ...preloadedState.chat!,
    current_thread_id: leftTabs[activeIndex] ?? chatId,
    open_thread_ids: [...leftTabs, splitChatId],
    threads: Object.fromEntries(
      [...leftTabs, splitChatId].map((id, index) => [
        id,
        {
          ...chatRuntime,
          thread: {
            ...chatRuntime.thread,
            id,
            title: ["First chat", "Middle chat", "Last chat", "Companion"][
              index
            ],
            messages: [],
          },
        },
      ]),
    ),
  };
  preloadedState.workspace = {
    tabs: [chatSurface(chatId)],
    activeTabId: chatSurface(chatId),
    dock: { open: false, width: 280, section: "files" },
    drawer: { open: false, height: 280 },
    groups: {
      [chatSurface(chatId)]: {
        root: {
          kind: "split",
          id: "alignment-split",
          dir: "row",
          sizes: [0.6, 0.4],
          children: [
            {
              kind: "leaf",
              id: "alignment-left",
              tabIds: leftTabs.map(chatSurface),
              activeTabId: activeSurface,
            },
            {
              kind: "leaf",
              id: "alignment-right",
              tabIds: [chatSurface(splitChatId)],
              activeTabId: chatSurface(splitChatId),
            },
          ],
        },
        focusedLeafId: "alignment-left",
      },
    },
  };
}

if (
  toolbarAlignmentRoute &&
  new URLSearchParams(window.location.search).has("unsplit")
) {
  preloadedState.workspace!.groups = {};
  preloadedState.workspace!.tabs = [
    chatSurface(chatId),
    chatSurface("alignment-middle"),
    chatSurface("alignment-last"),
    chatSurface(splitChatId),
  ];
}
const store = setUpStore(preloadedState);
if (toolbarAlignmentRoute) {
  Object.defineProperty(window, "__toolbarAlignmentStore", {
    get: () => store,
  });
}
if (workspaceGitActionsRoute) {
  const selector = workspaceGitPushRoute
    ? 'button[aria-label="Push…"]:not([disabled])'
    : 'button[aria-label^="Git branch:"]';
  const ready = () =>
    workspaceGitPushRoute ||
    document.querySelector('[aria-label="Git branch: main"]') !== null;
  const openWhenReady = () => {
    const target = document.querySelector<HTMLButtonElement>(selector);
    if (target && ready()) {
      target.dispatchEvent(
        new PointerEvent("pointerdown", { bubbles: true, button: 0 }),
      );
      target.click();
      return;
    }
    window.setTimeout(openWhenReady, 50);
  };
  window.setTimeout(openWhenReady, 50);
}
if (workspaceGitLogRoute) {
  const gitSurface = makeSurfaceKey("git", "main");
  store.dispatch(openTab(gitSurface));
  store.dispatch(setActiveTab(gitSurface));
}
if (workspaceGitDiffRoute) {
  const diffMode = new URLSearchParams(window.location.search).get("diff_mode");
  if (diffMode === "unified") {
    store.dispatch(setGitDiffViewerPrefs({ mode: "unified" }));
  }
  store.dispatch(
    openGitDiff({
      root: "/workspace/refact",
      path: "refact-agent/gui/src/features/Workspace/GitPanel/gitPanelSlice.ts",
      old_path: null,
      kind: "change",
      status: "MODIFIED",
    }),
  );
}
if (recentErrorsFixtureParam) {
  Object.defineProperty(window, "__routeShowcaseBuddyStore", {
    get: () => store,
  });
}
if (rebuildRoute) {
  Object.defineProperty(window, "__rebuildFixtureState", {
    get: () => store.getState().chat,
  });
}

const messageCall = {
  id: "message-receipt",
  index: 0,
  type: "function" as const,
  function: {
    name: "agent_message",
    arguments: JSON.stringify({
      to: "child",
      text: "A".repeat(4096) + "Second message section <script>bad()</script>",
    }),
  },
};
if (route === "agent-message") {
  store.dispatch(
    applyChatEvent({
      chat_id: chatId,
      seq: "1",
      type: "message_added",
      index: chatRuntime.thread.messages.length,
      message: {
        role: "tool",
        tool_call_id: messageCall.id,
        content: "Message queued for child.",
      },
    }),
  );
}

const broadcastCall = {
  id: "broadcast-receipt",
  index: 0,
  type: "function" as const,
  function: {
    name: "task_broadcast",
    arguments: JSON.stringify({ message: "Pause edits in src/shared/" }),
  },
};
if (route === "agent-broadcast") {
  const cards = Array.from({ length: 7 }, (_, index) => ({
    card_id: `T-${index}`,
    title: `Card ${index} <script>inert</script>`,
    outcome: index % 3 === 0 ? "no_live_session" : "delivered",
    reached: index % 3 !== 0,
    reason: index % 3 === 0 ? "agent session closed" : null,
  }));
  store.dispatch(
    applyChatEvent({
      chat_id: chatId,
      seq: "1",
      type: "message_added",
      index: chatRuntime.thread.messages.length,
      message: {
        role: "tool",
        tool_call_id: broadcastCall.id,
        content: "Broadcast sent to 4 of 7 agents",
        extra: {
          delivery_receipts: {
            kind: "task_broadcast",
            notified: 4,
            total: 7,
            cards,
          },
        },
      },
    }),
  );
}

const goalCall = {
  id: "goal-snapshot",
  index: 0,
  type: "function" as const,
  function: { name: "get_goal", arguments: "{}" },
};
if (route === "agent-goal") {
  store.dispatch(
    applyChatEvent({
      chat_id: chatId,
      seq: "1",
      type: "message_added",
      index: chatRuntime.thread.messages.length,
      message: {
        role: "tool",
        tool_call_id: goalCall.id,
        content: JSON.stringify({
          goal: {
            content:
              "Preserve the approved goal. ".padEnd(4096, "x") +
              "Second goal section: <script>inert evidence constraints</script>",
            status: "paused",
            version: 1,
            delta_count: 8,
            turns_used: 12,
            tokens_used: 3210,
            latest_verdict: null,
            gaps: [],
          },
        }),
      },
    }),
  );
}

const planCall = {
  id: "plan-snapshot",
  index: 0,
  type: "function" as const,
  function: { name: "get_plan", arguments: "{}" },
};
if (route === "agent-plan") {
  store.dispatch(
    applyChatEvent({
      chat_id: chatId,
      seq: "1",
      type: "message_added",
      index: chatRuntime.thread.messages.length,
      message: {
        role: "tool",
        tool_call_id: planCall.id,
        content: JSON.stringify({
          plan: {
            content:
              "Approved constraints. ".padEnd(4096, "x") +
              "Second plan section: preserve verification gates.",
            mode: "agent",
            version: 1,
            delta_count: 7,
            created_at_ms: 1789233578382,
          },
        }),
      },
    }),
  );
}

if (route === "agent-launch-preview") {
  store.dispatch(
    applyChatEvent({
      chat_id: chatId,
      seq: "1",
      type: "message_added",
      index: chatRuntime.thread.messages.length,
      message: launchPreviewReceipt(),
    }),
  );
}

const resumeCall = {
  id: "resume-inspect",
  index: 0,
  type: "function" as const,
  function: {
    name: "agent_resume",
    arguments: '{"action":"inspect","agent_id":"stopped-child"}',
  },
};
if (route === "agent-resume") {
  store.dispatch(
    applyChatEvent({
      chat_id: chatId,
      seq: "1",
      type: "message_added",
      index: chatRuntime.thread.messages.length,
      message: {
        role: "tool",
        tool_call_id: resumeCall.id,
        content: JSON.stringify(resumeReceipt()),
      },
    }),
  );
}

const approvalCall = {
  id: "approval-list",
  index: 0,
  type: "function" as const,
  function: { name: "agent_approval", arguments: '{"action":"list"}' },
};
if (route === "agent-approval") {
  store.dispatch(
    applyChatEvent({
      chat_id: chatId,
      seq: "1",
      type: "message_added",
      index: chatRuntime.thread.messages.length,
      message: {
        role: "tool",
        tool_call_id: approvalCall.id,
        content: JSON.stringify({
          requests: [
            {
              request_id: "first-request",
              agent_id: "first-child",
              decision: "pending",
              run_generation: 1,
            },
            {
              request_id: "second-request",
              agent_id: "second-child",
              decision: "approved",
              consumed: true,
              run_generation: 2,
            },
          ],
        }),
      },
    }),
  );
}

/**
 * Test-only queue server: the stubbed command endpoint mutates this list and
 * pushes a real `queue_updated` SSE envelope through the real reducer, so the
 * showcase exercises the production round trip instead of a static mock.
 */
let showcaseQueue: ChatThreadRuntime["queued_items"] = queueItems;
let showcaseQueueSeq = 100;

function pushRank(item: ChatThreadRuntime["queued_items"][number]): number {
  const mode = item.push ?? (item.priority ? "preempt" : "append");
  if (mode === "preempt") return 0;
  return mode === "when_idle" ? 2 : 1;
}

function publishShowcaseQueue(): void {
  showcaseQueue = [...showcaseQueue].sort((a, b) => pushRank(a) - pushRank(b));
  showcaseQueueSeq += 1;
  store.dispatch(
    applyChatEvent({
      chat_id: chatId,
      seq: String(showcaseQueueSeq),
      type: "queue_updated",
      queue_size: showcaseQueue.length,
      queued_items: showcaseQueue,
    }),
  );
}

let queueDemoPhase: "assistant" | "tools" | "idle" | "interrupted" =
  "assistant";
function publishQueueDemo(event: EventEnvelope): void {
  store.dispatch(
    applyChatEvent({
      ...event,
      chat_id: chatId,
      seq: String(++showcaseQueueSeq),
    }),
  );
}
function addQueueDemoMessage(message: ChatMessages[number]): void {
  publishQueueDemo({
    type: "message_added",
    chat_id: chatId,
    seq: "0",
    index: selectMessagesById(store.getState(), chatId).length,
    message,
  });
}
function deliverQueueDemo(mode: "preempt" | "append" | "when_idle"): void {
  const delivering = showcaseQueue.filter(
    (item) => (item.push ?? (item.priority ? "preempt" : "append")) === mode,
  );
  showcaseQueue = showcaseQueue.filter((item) => !delivering.includes(item));
  for (const item of delivering) {
    addQueueDemoMessage(
      item.command_type === "user_message"
        ? {
            role: "user",
            message_id: item.client_request_id,
            content: item.content ?? item.preview,
          }
        : {
            role: "event",
            message_id: item.client_request_id,
            content: item.preview || "Agent continuation delivered",
            subkind: item.event?.subkind ?? "system_notice",
            source: item.source ?? "queue.demo",
            payload: item.event?.payload ?? {},
          },
    );
  }
  publishShowcaseQueue();
}
function interruptQueueDemo(): void {
  if (queueDemoPhase === "assistant" || queueDemoPhase === "tools") {
    publishQueueDemo({
      type: "messages_truncated",
      chat_id: chatId,
      seq: "0",
      from_index: queueEventMessages.length,
    });
    addQueueDemoMessage({
      role: "event",
      message_id: "queue-cancellation",
      subkind: "cancellation_note",
      source: "chat.delivery",
      payload: { status: "cancelled", discarded: true },
      content:
        "Active step cancelled; partial assistant output and pending tool results discarded.",
    });
  }
  queueDemoPhase = "interrupted";
  publishQueueDemo({
    type: "runtime_updated",
    chat_id: chatId,
    seq: "0",
    state: "idle",
  });
  deliverQueueDemo("preempt");
}
function advanceQueueDemo(): void {
  if (queueDemoPhase === "assistant") {
    publishQueueDemo({
      type: "message_updated",
      chat_id: chatId,
      seq: "0",
      message_id: "queue-partial",
      message: {
        role: "assistant",
        message_id: "queue-partial",
        content: "Assistant response complete; waiting for the tool result.",
        tool_calls: [
          {
            id: "queue-demo-tool",
            index: 0,
            type: "function",
            function: { name: "cat", arguments: '{"paths":"README.md"}' },
          },
        ],
      },
    });
    queueDemoPhase = "tools";
  } else if (queueDemoPhase === "tools") {
    addQueueDemoMessage({
      role: "tool",
      tool_call_id: "queue-demo-tool",
      message_id: "queue-demo-result",
      content: "Tool result complete.",
      tool_failed: false,
    });
    deliverQueueDemo("append");
    queueDemoPhase = "idle";
  } else {
    publishQueueDemo({
      type: "runtime_updated",
      chat_id: chatId,
      seq: "0",
      state: "idle",
    });
    deliverQueueDemo("when_idle");
  }
}
Object.defineProperty(window, "__routeShowcaseDeliveryState", {
  get: () => ({
    phase: queueDemoPhase,
    messages: selectMessagesById(store.getState(), chatId),
    queue: showcaseQueue,
  }),
});

const showcaseQueueRouteRejections: string[] = [];
Object.assign(window, {
  __routeShowcaseQueueRouteRejections: showcaseQueueRouteRejections,
});

function showcaseQueueRouteRejection(
  path: string,
  method: string,
  body: unknown,
): string | null {
  const findItem = (id: unknown) =>
    showcaseQueue.find((item) => item.client_request_id === id);
  const queueMatch = /\/queue\/([^/]+)$/.exec(path);
  if (queueMatch) {
    const id = decodeURIComponent(queueMatch[1]);
    const item = findItem(id);
    if (!item) return `unknown queue item ${id}`;
    if (item.command_type === "delivery") {
      return `delivery ${id} must use update_pending_delivery`;
    }
    if (method === "DELETE") return null;
    if (
      method === "PATCH" &&
      typeof body === "object" &&
      body !== null &&
      Object.keys(body).length === 1 &&
      typeof (body as { priority?: unknown }).priority === "boolean"
    ) {
      return null;
    }
    return `invalid ${method} for queued command ${id}`;
  }
  if (!path.endsWith("/commands")) return null;
  if (typeof body !== "object" || body === null) return null;
  const command = body as { type?: unknown; delivery_id?: unknown };
  if (command.type !== "update_pending_delivery") return null;
  const item = findItem(command.delivery_id);
  if (!item || item.command_type !== "delivery") {
    return `update_pending_delivery rejected for ${String(
      command.delivery_id,
    )}`;
  }
  return null;
}

function applyShowcaseQueueMutation(
  path: string,
  method: string,
  body: unknown,
): void {
  if (!queueRoute) return;

  const queueMatch = /\/queue\/([^/]+)$/.exec(path);
  if (queueMatch) {
    const id = decodeURIComponent(queueMatch[1]);
    if (method === "DELETE") {
      showcaseQueue = showcaseQueue.filter(
        (item) => item.client_request_id !== id,
      );
      publishShowcaseQueue();
      return;
    }
    if (
      method === "PATCH" &&
      typeof body === "object" &&
      body !== null &&
      "priority" in body
    ) {
      const priority = Boolean((body as { priority: unknown }).priority);
      showcaseQueue = showcaseQueue.map((item) =>
        item.client_request_id === id ? { ...item, priority } : item,
      );
      publishShowcaseQueue();
    }
    return;
  }

  if (!path.endsWith("/commands")) return;
  if (typeof body !== "object" || body === null) return;
  const command = body as {
    type?: unknown;
    delivery_id?: unknown;
    push?: unknown;
    cancel?: unknown;
  };
  if (
    command.type !== "update_pending_delivery" ||
    typeof command.delivery_id !== "string"
  ) {
    return;
  }

  if (command.cancel === true) {
    showcaseQueue = showcaseQueue.filter(
      (item) => item.client_request_id !== command.delivery_id,
    );
    publishShowcaseQueue();
    return;
  }

  if (isPushMode(command.push)) {
    const push = command.push;
    showcaseQueue = showcaseQueue.map((item) =>
      item.client_request_id === command.delivery_id
        ? { ...item, push, priority: push === "preempt" }
        : item,
    );
    publishShowcaseQueue();
  }
}

// Explicit mock boundary controls; these model server events, not engine execution.
function advanceQueueBoundary(
  boundary: "assistant" | "tools" | "idle" | "interrupt",
) {
  const addMessage = (message: ChatMessages[number]) =>
    store.dispatch(
      applyChatEvent({
        chat_id: chatId,
        seq: String(++showcaseQueueSeq),
        type: "message_added",
        message,
        index:
          store.getState().chat.threads[chatId]?.thread.messages.length ?? 0,
      }),
    );
  if (boundary === "assistant") {
    addMessage({
      role: "assistant",
      message_id: "boundary-assistant",
      content: "Mock assistant response complete; tool results still pending.",
    });
    return;
  }
  if (boundary === "interrupt") {
    addMessage({
      role: "event",
      message_id: `boundary-cancel-${showcaseQueueSeq}`,
      content:
        "Active step cancelled; partial assistant and tool output discarded.",
      subkind: "cancellation_note",
      source: "chat.delivery",
      payload: { status: "cancelled" },
    });
  }
  const deliver = showcaseQueue.filter(
    (item) =>
      boundary === "idle" ||
      (boundary === "interrupt" ? pushRank(item) === 0 : pushRank(item) === 1),
  );
  if (boundary === "tools")
    addMessage({
      role: "assistant",
      message_id: "boundary-tools",
      content:
        "Mock assistant and all tool results complete: safe boundary reached.",
    });
  for (const item of deliver) {
    addMessage({
      role: "event",
      message_id: `delivered-${item.client_request_id}`,
      content: item.preview || "Agent continuation delivered",
      subkind: item.event?.subkind ?? "system_notice",
      source: item.source ?? "mock.delivery",
      payload: item.event?.payload ?? {},
    } as ChatMessages[number]);
  }
  showcaseQueue = showcaseQueue.filter((item) => !deliver.includes(item));
  publishShowcaseQueue();
  if (boundary === "idle")
    store.dispatch(
      applyChatEvent({
        chat_id: chatId,
        seq: String(++showcaseQueueSeq),
        type: "runtime_updated",
        state: "idle",
      }),
    );
}

store.dispatch(setBackendStatus({ status: "online" }));
if (homeActivityRoute) {
  store.dispatch(setPanelsForced(true));
  store.dispatch(setDockSection("activity"));
  store.dispatch(setDockOpen(true));
}
for (const section of ["workspace", "chats", "tasks", "buddy"] as const) {
  store.dispatch(sidebarSectionSnapshotReceived({ section, status: "ready" }));
}
store.dispatch(setBuddySnapshot(buddySnapshot));
store.dispatch(tasksApi.util.upsertQueryData("listTasks", undefined, tasks));

const root = document.getElementById("refact-chat");

if (!root) {
  throw new Error("Missing #refact-chat route showcase root");
}

const OverlayRegressionSurface = () => {
  const [model, setModel] = React.useState("openai_codex_personal/gpt-5.5");
  const [mode, setMode] = React.useState("agent");

  return (
    <div
      style={{
        display: "grid",
        gap: "var(--rf-space-4)",
        justifyItems: "start",
        minWidth: 0,
        padding: "var(--rf-space-5)",
      }}
    >
      <TrajectoryButton />
      <ModelSelector
        groups={overlayRegressionModelGroups}
        models={overlayRegressionModels}
        value={model}
        onSelect={setModel}
      />
      <ModeSelect selectedMode={mode} onModeChange={setMode} />
    </div>
  );
};

const PrivacyWithheldSurface = () => (
  <WithheldOutputCard
    exitCode={0}
    files={[privacyFile]}
    localOnlyOutput="TOKEN=local-secret"
  />
);

const PrivacyBlockSurface = () => (
  <BlockCard
    model="untrusted/model"
    step={3}
    blockedFiles={[privacyFile]}
    onSwitchModel={() => showcasePrivacyActions.push("switch-model")}
    onBranchCleanChat={() => showcasePrivacyActions.push("branch-clean-chat")}
  />
);

const QueueDemoControls = () => {
  const [phase, setPhase] = React.useState(queueDemoPhase);
  const [finished, setFinished] = React.useState(false);
  return (
    <Flex gap="2" p="2" wrap="wrap" align="center">
      <Text size="1">Mock backend · delivery boundary demo</Text>
      <Button
        size="1"
        disabled={finished}
        onClick={() => {
          const final =
            queueDemoPhase === "idle" || queueDemoPhase === "interrupted";
          advanceQueueDemo();
          setPhase(queueDemoPhase);
          setFinished(final);
        }}
      >
        {phase === "assistant"
          ? "Finish assistant response"
          : phase === "tools"
            ? "Finish tool results"
            : "Reach final idle"}
      </Button>
      <Button size="1" variant="soft" onClick={() => window.location.reload()}>
        Reset demo
      </Button>
    </Flex>
  );
};

const ShowcaseSurface = () => {
  const currentPage = useSelector((state: RootState) => {
    const page = state.pages[state.pages.length - 1];
    return page && isMarketplacePage(page)
      ? page
      : marketplaceTabToPage(marketplaceTab);
  });

  if (route === "overlay-regression") {
    return <OverlayRegressionSurface />;
  }

  if (route === "privacy-withheld") {
    return <PrivacyWithheldSurface />;
  }

  if (route === "privacy-block") {
    return <PrivacyBlockSurface />;
  }

  if (route === "badges") {
    const tones = [
      "default",
      "accent",
      "success",
      "warning",
      "danger",
      "muted",
    ] as const;
    const variants = ["soft", "outline", "glass"] as const;
    const sizes = ["xs", "sm", "md"] as const;

    return (
      <div>
        {tones.flatMap((tone) =>
          variants.flatMap((variant) =>
            sizes.map((size) => (
              <Badge
                key={`${tone}-${variant}-${size}`}
                data-testid="badge-contrast"
                data-tone={tone}
                data-variant={variant}
                data-size={size}
                tone={tone}
                variant={variant}
                size={size}
              >
                {`${tone} ${variant} ${size}`}
              </Badge>
            )),
          ),
        )}
      </div>
    );
  }

  if (chatDndRoute) {
    return (
      <Flex
        direction="column"
        style={{
          width: "100%",
          minWidth: 0,
          minHeight: 0,
          flex: "1 1 auto",
          overflow: "hidden",
        }}
      >
        <Flex style={{ flex: "1 1 auto", minWidth: 0, minHeight: 0 }}>
          <WorkspaceShell
            toolbar={<Toolbar activeTab={{ type: "chat", id: chatId }} />}
          >
            <WorkspaceView />
          </WorkspaceShell>
        </Flex>
      </Flex>
    );
  }

  if (queueRoute) {
    return (
      <Flex direction="column" style={{ height: "100%", minHeight: 0 }}>
        <Flex wrap="wrap" gap="1" aria-label="Mock backend boundary controls">
          <UiButton size="sm" onClick={() => advanceQueueBoundary("assistant")}>
            Mock: assistant finished
          </UiButton>
          <UiButton size="sm" onClick={() => advanceQueueBoundary("tools")}>
            Mock: tools finished
          </UiButton>
          <UiButton size="sm" onClick={() => advanceQueueBoundary("idle")}>
            Mock: final idle
          </UiButton>
          <UiButton size="sm" onClick={() => advanceQueueBoundary("interrupt")}>
            Mock: interrupt and discard
          </UiButton>
        </Flex>
        <Flex style={{ flex: "1 1 auto", minHeight: 0 }}>
          <WorkspaceShell>
            <WorkspaceView />
          </WorkspaceShell>
        </Flex>
      </Flex>
    );
  }

  if (chatRoute) {
    return (
      <>
        {queueRoute && <QueueDemoControls />}
        <WorkspaceShell>
          <WorkspaceView />
        </WorkspaceShell>
      </>
    );
  }

  if (route === "settings") {
    return (
      <SettingsHub
        page={settingsSectionToPage(settingsSection)}
        onBack={() => undefined}
        host="web"
        tabbed={false}
      />
    );
  }

  if (route === "marketplace") {
    return (
      <MarketplaceHub
        page={currentPage}
        back={() => undefined}
        host="web"
        tabbed={false}
      />
    );
  }

  if (route === "buddy") {
    return <BuddyHome />;
  }

  if (route === "work") return <WorkPanel />;
  if (homeActivityRoute) {
    return (
      <WorkspaceShell>
        <Dashboard />
      </WorkspaceShell>
    );
  }
  if (route === "agent-approval")
    return <ToolContent toolCalls={[approvalCall]} threadId={chatId} />;
  if (route === "agent-resume")
    return <ToolContent toolCalls={[resumeCall]} threadId={chatId} />;
  if (route === "agent-launch-preview")
    return <ToolContent toolCalls={[launchPreviewCall]} threadId={chatId} />;
  if (route === "agent-plan")
    return <ToolContent toolCalls={[planCall]} threadId={chatId} />;
  if (route === "agent-goal")
    return <ToolContent toolCalls={[goalCall]} threadId={chatId} />;
  if (route === "agent-message")
    return <ToolContent toolCalls={[messageCall]} threadId={chatId} />;
  if (route === "agent-broadcast")
    return <ToolContent toolCalls={[broadcastCall]} threadId={chatId} />;

  return <Dashboard />;
};

const Showcase = () => {
  return (
    <Provider store={store}>
      <Theme>
        <AbortControllerProvider>
          <Flex
            align="stretch"
            direction="column"
            data-element="app-root"
            style={{
              width: "100%",
              maxWidth: "100%",
              minWidth: 0,
              overflow: "hidden",
            }}
          >
            <ShowcaseSurface />
          </Flex>
        </AbortControllerProvider>
      </Theme>
    </Provider>
  );
};

createRoot(root).render(<Showcase />);
