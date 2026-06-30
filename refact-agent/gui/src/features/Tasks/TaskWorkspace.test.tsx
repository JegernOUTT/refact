import { readFile } from "node:fs/promises";
import { resolve } from "node:path";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { delay, http, HttpResponse } from "msw";
import {
  cleanup,
  render,
  screen,
  waitFor,
  within,
} from "../../utils/test-utils";
import { PlannerItem, TaskWorkspace } from "./TaskWorkspace";
import {
  isActionableWorktree,
  resolveCardWorktree,
} from "./TaskWorkspaceWorktree";
import type { CardWorktreeTarget } from "./TaskWorkspaceWorktree";
import type { PlannerInfo } from "./tasksSlice";
import { openTask, taskSseEventReceived } from "./tasksSlice";
import { createChatWithId, switchToThread } from "../Chat/Thread";
import { push } from "../Pages/pagesSlice";
import {
  loadTaskWorkspaceTab,
  setProjectStorageNamespace,
} from "../../utils/chatUiPersistence";
import type { ChatThreadRuntime } from "../Chat/Thread/types";
import {
  tasksApi,
  type BoardCard,
  type TaskBoard,
  type TaskMeta,
  type TrajectoryInfo,
} from "../../services/refact/tasks";
import type {
  WorktreeListResponse,
  WorktreeMeta,
  WorktreeRecordView,
} from "../../services/refact";
import { taskDocumentsApi } from "../../services/refact/taskDocumentsApi";
import { taskMemoriesApi } from "../../services/refact/taskMemoriesApi";
import { server } from "../../utils/mockServer";

const TASK_ID = "task-1";
const CARD_ID = "T-1";
const PLANNER_ID = "planner-test-1";
const LEGACY_PATH = "/tmp/refact/legacy/wt-path";
const LEGACY_TOOLTIP =
  "This worktree was created before the registry; recreate it via `restart_agent(mode=fresh)` to enable actions.";

function readGuiSource(path: string): Promise<string> {
  return readFile(resolve(process.cwd(), "src", path), "utf8");
}

function readCssBlock(source: string, selector: string): string {
  const escapedSelector = selector.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const match = new RegExp(`(^|\\n)\\s*${escapedSelector}\\s*{`).exec(source);
  if (match?.index === undefined) {
    throw new Error(`Missing CSS block for ${selector}`);
  }
  const start = source.indexOf("{", match.index);
  const end = source.indexOf("\n}", start);
  if (start === -1 || end === -1) {
    throw new Error(`Malformed CSS block for ${selector}`);
  }
  return source.slice(start + 1, end);
}

function expectCardWorktreeTarget(
  target: CardWorktreeTarget | null,
): asserts target is CardWorktreeTarget {
  expect(target).not.toBeNull();
}

type MockWorktreePanelProps = {
  open: boolean;
  worktreeId?: string | null;
};

const worktreeDiffPanelProps = vi.hoisted((): MockWorktreePanelProps[] => []);
const mergeWorktreeModalProps = vi.hoisted((): MockWorktreePanelProps[] => []);

vi.mock("../../hooks/useCopyToClipboard", () => ({
  useCopyToClipboard: () => vi.fn(),
}));

vi.mock("../Worktrees/BranchIcon", () => ({
  BranchIcon: () => <span data-testid="branch-icon" />,
}));

vi.mock("../Worktrees/WorktreeDiffPanel", () => ({
  WorktreeDiffPanel: (props: MockWorktreePanelProps) => {
    worktreeDiffPanelProps.push(props);
    return props.open ? (
      <div
        data-testid="worktree-diff-panel"
        data-worktree-id={props.worktreeId ?? ""}
      />
    ) : null;
  },
}));

vi.mock("../Worktrees/MergeWorktreeModal", () => ({
  MergeWorktreeModal: (props: MockWorktreePanelProps) => {
    mergeWorktreeModalProps.push(props);
    return props.open ? (
      <div
        data-testid="merge-worktree-modal"
        data-worktree-id={props.worktreeId ?? ""}
      />
    ) : null;
  },
}));

vi.mock("../Worktrees/WorktreeStatusBadge", () => ({
  WorktreeStatusBadge: () => <span data-testid="worktree-status-badge" />,
}));

vi.mock("../Worktrees/worktreeConflict", () => ({
  buildWorktreeConflictPrompt: () => "Resolve conflicts.",
}));

vi.mock("../Worktrees/worktreeError", () => ({
  worktreeErrorText: () => "worktree error",
}));

vi.mock("../Worktrees", () => ({
  BranchIcon: () => <span data-testid="branch-icon" />,
  WorktreeDiffPanel: (props: MockWorktreePanelProps) => {
    worktreeDiffPanelProps.push(props);
    return props.open ? (
      <div
        data-testid="worktree-diff-panel"
        data-worktree-id={props.worktreeId ?? ""}
      />
    ) : null;
  },
  MergeWorktreeModal: (props: MockWorktreePanelProps) => {
    mergeWorktreeModalProps.push(props);
    return props.open ? (
      <div
        data-testid="merge-worktree-modal"
        data-worktree-id={props.worktreeId ?? ""}
      />
    ) : null;
  },
  WorktreeStatusBadge: () => <span data-testid="worktree-status-badge" />,
  buildWorktreeConflictPrompt: () => "Resolve conflicts.",
  worktreeErrorText: () => "worktree error",
}));

const makePlanner = (waitingForCardIds?: string[]): PlannerInfo => ({
  id: PLANNER_ID,
  title: "Test Planner",
  createdAt: "2026-01-01T00:00:00Z",
  updatedAt: "2026-01-01T00:00:00Z",
  waitingForCardIds,
});

const makeRuntime = (
  sessionState?: string,
  id = PLANNER_ID,
  worktree?: WorktreeMeta | null,
): ChatThreadRuntime => ({
  thread: {
    id,
    messages: [],
    title: "Test Planner",
    model: "",
    last_user_message_id: "",
    new_chat_suggested: { wasSuggested: false },
    worktree,
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
  session_state: sessionState,
});

const makePreloadedState = (sessionState?: string) => ({
  chat: {
    current_thread_id: PLANNER_ID,
    open_thread_ids: [PLANNER_ID],
    threads: { [PLANNER_ID]: makeRuntime(sessionState) },
    system_prompt: {},
    tool_use: "explore" as const,
    sse_refresh_requested: null,
    stream_version: 0,
  },
});

function configState() {
  return {
    host: "web" as const,
    lspPort: 8001,
    apiKey: null,
    themeProps: { appearance: "dark" as const },
    features: { images: true, statistics: true, vecdb: true, ast: true },
  };
}

function makeMeta(overrides: Partial<WorktreeMeta> = {}): WorktreeMeta {
  const id = overrides.id ?? "wt-name";
  return {
    id,
    kind: "task_agent",
    root: `/tmp/refact/${id}`,
    source_workspace_root: "/repo",
    repo_root: "/repo",
    branch: "refact/task/T-1",
    base_branch: "main",
    base_commit: "abc123",
    task_id: TASK_ID,
    card_id: CARD_ID,
    enforce: true,
    ...overrides,
  };
}

function makeRecord(
  metaOverrides: Partial<WorktreeMeta> = {},
  statusOverrides: Partial<WorktreeRecordView["status"]> = {},
  references: WorktreeRecordView["references"] = [],
): WorktreeRecordView {
  const meta = makeMeta(metaOverrides);
  const referenceCount = meta.reference_count ?? 1;
  return {
    meta,
    created_at: "2026-04-30T00:00:00Z",
    updated_at: "2026-04-30T00:00:00Z",
    references,
    reference_count: referenceCount,
    referencing_chat_ids: [],
    status: {
      path_exists: true,
      is_git_worktree: true,
      dirty: true,
      staged_count: 1,
      unstaged_count: 1,
      untracked_count: 0,
      branch: meta.branch,
      head_commit: "def456",
      ...statusOverrides,
    },
  };
}

function makeCard(overrides: Partial<BoardCard> = {}): BoardCard {
  return {
    id: CARD_ID,
    title: "Implement worktree",
    column: "doing",
    priority: "P1",
    depends_on: [],
    instructions: "Use a worktree.",
    assignee: "agent-1",
    agent_chat_id: "agent-T-1",
    status_updates: [],
    final_report: null,
    created_at: "2026-04-30T00:00:00Z",
    started_at: "2026-04-30T00:00:00Z",
    completed_at: null,
    target_files: [],
    ...overrides,
  };
}

function makeTask(): TaskMeta {
  return {
    id: TASK_ID,
    name: "Task with worktree",
    status: "active",
    created_at: "2026-04-30T00:00:00Z",
    updated_at: "2026-04-30T00:00:00Z",
    cards_total: 1,
    cards_done: 0,
    cards_failed: 0,
    agents_active: 1,
    base_branch: "main",
  };
}

function makeBoard(card: BoardCard): TaskBoard {
  return {
    schema_version: 1,
    rev: 1,
    columns: [
      { id: "planned", title: "Planned" },
      { id: "doing", title: "Doing" },
      { id: "done", title: "Done" },
      { id: "failed", title: "Failed" },
    ],
    cards: [card],
  };
}

function makeWorktreeList(records: WorktreeRecordView[]): WorktreeListResponse {
  return {
    project_hash: "project-hash",
    source_workspace_root: "/repo",
    source_current_branch: "main",
    worktrees: records,
  };
}

function taskWorkspaceHandlers(
  card: BoardCard,
  records: WorktreeRecordView[],
  openCalls: string[] = [],
  deleteCalls: string[] = [],
) {
  return [
    http.get("*/v1/tasks/task-1", () =>
      HttpResponse.json({ meta: makeTask() }),
    ),
    http.get("*/v1/tasks/task-1/board", () =>
      HttpResponse.json(makeBoard(card)),
    ),
    http.get("*/v1/tasks/task-1/trajectories/planner", () =>
      HttpResponse.json([]),
    ),
    http.get("*/v1/tasks/task-1/trajectories/agents", () =>
      HttpResponse.json([]),
    ),
    http.get("*/v1/worktrees", () =>
      HttpResponse.json(makeWorktreeList(records)),
    ),
    http.get("*/v1/ping", () => HttpResponse.json({ status: "ok" })),
    http.get("*/v1/chat-modes", () =>
      HttpResponse.json({ modes: [], errors: [] }),
    ),
    http.get("*/v1/caps", () =>
      HttpResponse.json({ chat_models: [], completion_models: [] }),
    ),
    http.get("*/v1/voice/status", () =>
      HttpResponse.json({ enabled: false, available: false }),
    ),
    http.get("*/v1/chats/:id/skills-status", () =>
      HttpResponse.json({ enabled: false, skills: [] }),
    ),
    http.get("*/v1/buddy/opportunities", () =>
      HttpResponse.json({ opportunities: [] }),
    ),
    http.get("*/v1/task/:id/memories", () =>
      HttpResponse.json({
        task_id: TASK_ID,
        since: "",
        new_count: 0,
        memories: [],
        warnings: [],
      }),
    ),
    http.get("*/v1/task/:id/memories/facets", () =>
      HttpResponse.json({
        task_id: TASK_ID,
        namespaces: [],
        tags: [],
        kinds: [],
        total_count: 0,
        pinned_count: 0,
      }),
    ),
    http.get("*/v1/task/:id/documents", () =>
      HttpResponse.json({ task_id: TASK_ID, documents: [] }),
    ),
    http.post("*/v1/buddy/diagnostics/collect", () => HttpResponse.json({})),
    http.get("*/v1/worktrees/:id/diff", ({ params }) => {
      const id = String(params.id);
      return HttpResponse.json({
        id,
        branch: "refact/task/T-1",
        base_branch: "main",
        base_commit: "abc123",
        status: {
          path_exists: true,
          is_git_worktree: true,
          dirty: false,
          staged_count: 0,
          unstaged_count: 0,
          untracked_count: 0,
          branch: "refact/task/T-1",
        },
        files: [],
        stats: {
          committed_files: 0,
          staged_files: 0,
          unstaged_files: 0,
          untracked_files: 0,
          files_changed: 0,
        },
        patch: "",
        patch_truncated: false,
      });
    }),
    http.post("*/v1/worktrees/:id/open", ({ params }) => {
      const id = String(params.id);
      openCalls.push(id);
      return HttpResponse.json({
        id,
        path: `/tmp/refact/${id}`,
        branch: "refact/task/T-1",
        can_open_folder: false,
      });
    }),
    http.delete("*/v1/worktrees/:id", ({ params }) => {
      deleteCalls.push(String(params.id));
      return HttpResponse.json({
        deleted: true,
        branch_deleted: false,
        stale_path: false,
        affected_references: [],
        affected_reference_count: 1,
        warnings: [],
      });
    }),
  ];
}

function workspacePreloadedState(
  chatId = "agent-T-1",
  worktree?: WorktreeMeta | null,
) {
  return {
    config: configState(),
    chat: {
      current_thread_id: chatId,
      open_thread_ids: [chatId],
      threads: { [chatId]: makeRuntime(undefined, chatId, worktree) },
      system_prompt: {},
      tool_use: "agent" as const,
      sse_refresh_requested: null,
      stream_version: 0,
    },
    tasksUI: { openTasks: [] },
  };
}

async function openCardDetail(card: BoardCard) {
  const titles = await screen.findAllByText(card.title);
  await waitFor(() =>
    expect(screen.getAllByText(card.id).length).toBeGreaterThan(0),
  );
  const kanbanTitle = titles.find((title) =>
    title.closest("[class*='kanbanCard']"),
  );
  if (!kanbanTitle) {
    throw new Error(`Kanban card title not found for ${card.title}`);
  }
  return kanbanTitle;
}

function openedIds(props: MockWorktreePanelProps[]): string[] {
  const ids: string[] = [];
  for (const prop of props) {
    if (prop.open && prop.worktreeId) ids.push(prop.worktreeId);
  }
  return ids;
}

beforeEach(() => {
  worktreeDiffPanelProps.length = 0;
  mergeWorktreeModalProps.length = 0;
});

function clearWorkspaceStorage() {
  localStorage.clear();
  sessionStorage.clear();
  setProjectStorageNamespace(undefined);
}

describe("PlannerItem waiting chips", () => {
  it("renders waiting card chips when session_state === 'waiting_user_input'", () => {
    const planner = makePlanner(["T-2", "T-3", "T-5"]);

    render(
      <PlannerItem
        planner={planner}
        isSelected={false}
        onSelect={vi.fn()}
        onRemove={vi.fn()}
      />,
      { preloadedState: makePreloadedState("waiting_user_input") },
    );

    expect(screen.getByText("T-2")).toBeInTheDocument();
    expect(screen.getByText("T-3")).toBeInTheDocument();
    expect(screen.getByText("T-5")).toBeInTheDocument();
  });

  it("caps chip list at 5 with '… and N more'", () => {
    const planner = makePlanner([
      "T-1",
      "T-2",
      "T-3",
      "T-4",
      "T-5",
      "T-6",
      "T-7",
      "T-8",
    ]);

    render(
      <PlannerItem
        planner={planner}
        isSelected={false}
        onSelect={vi.fn()}
        onRemove={vi.fn()}
      />,
      { preloadedState: makePreloadedState("waiting_user_input") },
    );

    expect(screen.getByText("T-1")).toBeInTheDocument();
    expect(screen.getByText("T-5")).toBeInTheDocument();
    expect(screen.queryByText("T-6")).not.toBeInTheDocument();
    expect(screen.getByText(/and 3 more/)).toBeInTheDocument();
  });

  it("does not render chips when session_state !== 'waiting_user_input'", () => {
    const planner = makePlanner(["T-2", "T-3", "T-5"]);

    render(
      <PlannerItem
        planner={planner}
        isSelected={false}
        onSelect={vi.fn()}
        onRemove={vi.fn()}
      />,
      { preloadedState: makePreloadedState("generating") },
    );

    expect(
      screen.queryByTestId(`planner-waiting-chips-${planner.id}`),
    ).not.toBeInTheDocument();
  });

  it("does not render chips when waitingForCardIds is empty", () => {
    const planner = makePlanner([]);

    render(
      <PlannerItem
        planner={planner}
        isSelected={false}
        onSelect={vi.fn()}
        onRemove={vi.fn()}
      />,
      { preloadedState: makePreloadedState("waiting_user_input") },
    );

    expect(
      screen.queryByTestId(`planner-waiting-chips-${planner.id}`),
    ).not.toBeInTheDocument();
  });

  it("does not render chips when waitingForCardIds is undefined", () => {
    const planner = makePlanner(undefined);

    render(
      <PlannerItem
        planner={planner}
        isSelected={false}
        onSelect={vi.fn()}
        onRemove={vi.fn()}
      />,
      { preloadedState: makePreloadedState("waiting_user_input") },
    );

    expect(
      screen.queryByTestId(`planner-waiting-chips-${planner.id}`),
    ).not.toBeInTheDocument();
  });

  it("pressing_enter_on_focused_planner_item_invokes_onSelect", async () => {
    const planner = makePlanner();
    const onSelect = vi.fn();

    const { user } = render(
      <PlannerItem
        planner={planner}
        isSelected={false}
        onSelect={onSelect}
        onRemove={vi.fn()}
      />,
      { preloadedState: makePreloadedState() },
    );

    const item = screen.getByRole("button", {
      name: /Open chat/,
    });
    item.focus();
    await user.keyboard("{Enter}");

    expect(onSelect).toHaveBeenCalledOnce();
  });

  it("renders a mode badge for non-planner chats", () => {
    const planner = { ...makePlanner(), mode: "agent" };

    render(
      <PlannerItem
        planner={planner}
        isSelected={false}
        onSelect={vi.fn()}
        onRemove={vi.fn()}
      />,
      { preloadedState: makePreloadedState("idle") },
    );

    expect(screen.getByText("agent")).toBeInTheDocument();
  });

  it("does not render a mode badge for task_planner chats", () => {
    const planner = { ...makePlanner(), mode: "task_planner" };

    render(
      <PlannerItem
        planner={planner}
        isSelected={false}
        onSelect={vi.fn()}
        onRemove={vi.fn()}
      />,
      { preloadedState: makePreloadedState("idle") },
    );

    expect(screen.queryByText("task_planner")).not.toBeInTheDocument();
  });
});

describe("PlannerItem linked cards", () => {
  it("renders linked card badges for cards whose agent the chat spawned", () => {
    const planner = makePlanner();

    render(
      <PlannerItem
        planner={planner}
        isSelected={false}
        linkedCardIds={["G-1", "G-2"]}
        onSelect={vi.fn()}
        onRemove={vi.fn()}
      />,
      { preloadedState: makePreloadedState("idle") },
    );

    expect(
      screen.getByTestId(`planner-linked-cards-${planner.id}`),
    ).toBeInTheDocument();
    expect(screen.getByText("G-1")).toBeInTheDocument();
    expect(screen.getByText("G-2")).toBeInTheDocument();
  });

  it("caps linked card badges at 4 with '+N'", () => {
    const planner = makePlanner();

    render(
      <PlannerItem
        planner={planner}
        isSelected={false}
        linkedCardIds={["G-1", "G-2", "G-3", "G-4", "G-5", "G-6"]}
        onSelect={vi.fn()}
        onRemove={vi.fn()}
      />,
      { preloadedState: makePreloadedState("idle") },
    );

    expect(screen.getByText("G-4")).toBeInTheDocument();
    expect(screen.queryByText("G-5")).not.toBeInTheDocument();
    expect(screen.getByText("+2")).toBeInTheDocument();
  });

  it("does not render the linked cards row when there are none", () => {
    const planner = makePlanner();

    render(
      <PlannerItem
        planner={planner}
        isSelected={false}
        onSelect={vi.fn()}
        onRemove={vi.fn()}
      />,
      { preloadedState: makePreloadedState("idle") },
    );

    expect(
      screen.queryByTestId(`planner-linked-cards-${planner.id}`),
    ).not.toBeInTheDocument();
  });
});

describe("TaskWorkspace worktree resolution", () => {
  it("resolves_worktree_by_agent_worktree_name_field", () => {
    const card = makeCard({
      agent_worktree: LEGACY_PATH,
      agent_worktree_name: "wt-name",
      agent_branch: "refact/task/by-name",
    });
    const record = makeRecord({ id: "wt-name", branch: "refact/task/by-name" });

    const target = resolveCardWorktree(TASK_ID, card, [record]);

    expect(target).toMatchObject({ id: "wt-name", record, legacy: false });
    expect(target?.id).not.toBe(LEGACY_PATH);
  });

  it("resolves_worktree_by_thread_metadata_when_name_missing", () => {
    const card = makeCard({ agent_worktree: LEGACY_PATH });
    const threadWorktree = makeMeta({ id: "wt-thread" });
    const record = makeRecord({ id: "wt-thread" });

    const target = resolveCardWorktree(TASK_ID, card, [record], threadWorktree);

    expect(target).toMatchObject({ id: "wt-thread", record, legacy: false });
    expect(target?.id).not.toBe(LEGACY_PATH);
  });

  it("resolves_worktree_by_task_card_pair_when_name_missing", () => {
    const card = makeCard({ agent_worktree: LEGACY_PATH });
    const record = makeRecord({
      id: "wt-card",
      task_id: TASK_ID,
      card_id: CARD_ID,
    });

    const target = resolveCardWorktree(TASK_ID, card, [record]);

    expect(target).toMatchObject({ id: "wt-card", record, legacy: false });
    expect(target?.id).not.toBe(LEGACY_PATH);
  });

  it("resolves_worktree_by_attached_references_when_meta_is_missing", () => {
    const card = makeCard({
      agent_worktree: LEGACY_PATH,
      agent_chat_id: "agent-chat-from-reference",
    });
    const record = makeRecord(
      {
        id: "wt-reference",
        task_id: null,
        card_id: null,
      },
      {},
      [
        {
          kind: "task_agent",
          task_id: TASK_ID,
          card_id: CARD_ID,
          chat_id: "agent-chat-from-reference",
        },
      ],
    );

    const target = resolveCardWorktree(TASK_ID, card, [record]);

    expect(target).toMatchObject({
      id: "wt-reference",
      record,
      legacy: false,
    });
    expectCardWorktreeTarget(target);
    expect(isActionableWorktree(target)).toBe(true);
  });

  it("unresolved_registry_id_is_stale_and_not_actionable", () => {
    const card = makeCard({
      agent_worktree: LEGACY_PATH,
      agent_worktree_name: "missing-wt",
      agent_branch: "refact/task/stale-id",
    });

    const target = resolveCardWorktree(TASK_ID, card, []);

    expect(target).toMatchObject({
      id: "missing-wt",
      legacy: false,
      stale: true,
    });
    expect(target?.record).toBeUndefined();
    expect(target?.label).toBe("missing-wt");
    expectCardWorktreeTarget(target);
    expect(isActionableWorktree(target)).toBe(false);
  });

  it("resolves_worktree_by_branch_for_legacy_cards", () => {
    const card = makeCard({
      agent_worktree: LEGACY_PATH,
      agent_branch: "refact/task/by-branch",
    });
    const record = makeRecord({
      id: "wt-branch",
      branch: "refact/task/by-branch",
      task_id: null,
      card_id: null,
    });

    const target = resolveCardWorktree(TASK_ID, card, [record]);

    expect(target).toMatchObject({ id: "wt-branch", record, legacy: false });
    expect(target?.id).not.toBe(LEGACY_PATH);
  });

  it("card_with_only_filesystem_path_returns_legacy_target", () => {
    const card = makeCard({ agent_worktree: LEGACY_PATH });

    const target = resolveCardWorktree(TASK_ID, card, []);

    expect(target).toMatchObject({ id: "", legacy: true, stale: false });
    expect(target?.id).not.toBe(LEGACY_PATH);
    expect(target?.label).toBe("legacy/wt-path");
  });
});

describe("TaskWorkspace worktree actions", () => {
  it("renders_server_defined_kanban_columns_and_card_detail_fields", async () => {
    const card = makeCard({
      title: "Current server column card",
      column: "blocked",
      priority: "P0",
      depends_on: ["T-0", "T-7"],
      instructions: "Follow the **server** card instructions.",
      assignee: "agent-2",
      agent_chat_id: "agent-blocked",
      status_updates: [
        { timestamp: "2026-01-02T00:00:00Z", message: "Waiting on review" },
      ],
      final_report: "Final parity report",
      comments: [
        {
          id: "comment-1",
          author_role: "planner",
          author_id: PLANNER_ID,
          timestamp: "2026-01-02T00:00:00Z",
          body: "Planner comment body",
          reply_to: null,
        },
      ],
    });
    server.use(...taskWorkspaceHandlers(card, []));
    server.use(
      http.get("*/v1/tasks/task-1/board", () =>
        HttpResponse.json({
          ...makeBoard(card),
          columns: [
            { id: "blocked", title: "Blocked by Review" },
            { id: "qa", title: "QA Ready" },
          ],
        }),
      ),
    );

    const { user } = render(<TaskWorkspace taskId={TASK_ID} />, {
      preloadedState: workspacePreloadedState("planner-test-1"),
    });

    expect(await screen.findByText("Blocked by Review")).toBeInTheDocument();
    expect(screen.getByText("QA Ready")).toBeInTheDocument();
    expect(screen.getByText(card.title)).toBeInTheDocument();
    expect(screen.getByText("P0")).toBeInTheDocument();
    expect(screen.getByText("Agent")).toBeInTheDocument();
    expect(screen.getByText("2")).toBeInTheDocument();
    expect(screen.getAllByText("1").length).toBeGreaterThanOrEqual(2);

    await user.click(await openCardDetail(card));

    const dialog = await screen.findByRole("dialog", {
      name: /Current server column card/,
    });
    expect(dialog).toHaveTextContent(card.id);
    expect(dialog).toHaveTextContent("blocked");
    expect(dialog).toHaveTextContent("Dependencies");
    expect(dialog).toHaveTextContent("T-0");
    expect(dialog).toHaveTextContent("T-7");
    expect(dialog).toHaveTextContent("Instructions");
    expect(dialog).toHaveTextContent("Follow the server card instructions.");
    expect(dialog).toHaveTextContent("Final Report");
    expect(dialog).toHaveTextContent("Final parity report");
    expect(dialog).toHaveTextContent("Updates");
    expect(dialog).toHaveTextContent("Waiting on review");
    expect(dialog).toHaveTextContent("Planner comment body");
  });

  it("legacy_target_disables_diff_merge_open_delete_buttons", async () => {
    const card = makeCard({ agent_worktree: LEGACY_PATH });
    server.use(...taskWorkspaceHandlers(card, []));

    const { user } = render(<TaskWorkspace taskId={TASK_ID} />, {
      preloadedState: workspacePreloadedState(),
    });

    await user.click(await openCardDetail(card));

    expect(
      screen.getByText("Legacy / unregistered worktree"),
    ).toBeInTheDocument();
    const buttons = [
      screen.getByRole("button", { name: "View Diff" }),
      screen.getByRole("button", { name: "Merge" }),
      screen.getByRole("button", { name: "Open" }),
      screen.getByRole("button", { name: "Discard/Delete" }),
    ];
    for (const button of buttons) {
      expect(button).toBeDisabled();
      expect(button).toHaveAttribute("title", LEGACY_TOOLTIP);
    }
  });

  it("stale_record_disables_buttons_with_amber_text", async () => {
    const record = makeRecord(
      { id: "wt-stale", lifecycle_state: "deleted" },
      { path_exists: false },
    );
    const card = makeCard({ agent_worktree_name: record.meta.id });
    server.use(...taskWorkspaceHandlers(card, [record]));

    const { user } = render(<TaskWorkspace taskId={TASK_ID} />, {
      preloadedState: workspacePreloadedState(),
    });

    await user.click(await openCardDetail(card));

    expect(
      screen.getByText("This worktree appears stale, missing, or deleted."),
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "View Diff" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Merge" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Open" })).toBeDisabled();
    expect(
      screen.getByRole("button", { name: "Discard/Delete" }),
    ).toBeDisabled();
  });

  it("unresolved_registry_id_renders_label_only_without_worktree_actions", async () => {
    const card = makeCard({
      agent_worktree: LEGACY_PATH,
      agent_worktree_name: "missing-wt",
      agent_branch: "refact/task/missing-wt",
    });
    const openCalls: string[] = [];
    const deleteCalls: string[] = [];
    server.use(...taskWorkspaceHandlers(card, [], openCalls, deleteCalls));

    const { user } = render(<TaskWorkspace taskId={TASK_ID} />, {
      preloadedState: workspacePreloadedState(),
    });

    await user.click(await openCardDetail(card));

    expect(screen.getAllByTitle("Worktree: missing-wt").length).toBeGreaterThan(
      0,
    );
    expect(
      screen.getByText("This worktree appears stale, missing, or deleted."),
    ).toBeInTheDocument();
    const buttons = [
      screen.getByRole("button", { name: "View Diff" }),
      screen.getByRole("button", { name: "Merge" }),
      screen.getByRole("button", { name: "Open" }),
      screen.getByRole("button", { name: "Discard/Delete" }),
    ];
    for (const button of buttons) {
      expect(button).toBeDisabled();
    }
    await user.click(buttons[0]);
    await user.click(buttons[1]);
    await user.click(buttons[2]);
    await user.click(buttons[3]);
    expect(openedIds(worktreeDiffPanelProps)).toEqual([]);
    expect(openedIds(mergeWorktreeModalProps)).toEqual([]);
    expect(openCalls).toEqual([]);
    expect(deleteCalls).toEqual([]);
  });

  it("worktree_merge_and_delete_refresh_inventory_summary", async () => {
    const source = await readGuiSource("services/refact/worktrees.ts");
    const mutationInvalidatesSummary = (endpoint: string) => {
      const match = new RegExp(
        `${endpoint}: builder\\.mutation[\\s\\S]*?invalidatesTags:[\\s\\S]*?\\],`,
      ).exec(source);
      if (!match) throw new Error(`Missing invalidation block for ${endpoint}`);
      return match[0].includes('id: "SUMMARY"');
    };

    expect(mutationInvalidatesSummary("mergeWorktree")).toBe(true);
    expect(mutationInvalidatesSummary("deleteWorktree")).toBe(true);
  });

  it("worktree_id_passed_to_apis_is_never_a_filesystem_path", async () => {
    const scenarios: {
      card: BoardCard;
      records: WorktreeRecordView[];
      threadWorktree?: WorktreeMeta | null;
      expectedId?: string;
    }[] = [
      {
        card: makeCard({
          title: "By name",
          agent_worktree: LEGACY_PATH,
          agent_worktree_name: "wt-name",
        }),
        records: [makeRecord({ id: "wt-name" })],
        expectedId: "wt-name",
      },
      {
        card: makeCard({ title: "By thread", agent_worktree: LEGACY_PATH }),
        records: [makeRecord({ id: "wt-thread" })],
        threadWorktree: makeMeta({ id: "wt-thread" }),
        expectedId: "wt-thread",
      },
      {
        card: makeCard({ title: "By task card", agent_worktree: LEGACY_PATH }),
        records: [
          makeRecord({ id: "wt-card", task_id: TASK_ID, card_id: CARD_ID }),
        ],
        expectedId: "wt-card",
      },
      {
        card: makeCard({
          title: "By branch",
          agent_worktree: LEGACY_PATH,
          agent_branch: "refact/task/by-branch",
        }),
        records: [
          makeRecord({
            id: "wt-branch",
            branch: "refact/task/by-branch",
            task_id: null,
            card_id: null,
          }),
        ],
        expectedId: "wt-branch",
      },
      {
        card: makeCard({ title: "Path only", agent_worktree: LEGACY_PATH }),
        records: [],
      },
    ];

    for (const scenario of scenarios) {
      cleanup();
      server.resetHandlers();
      worktreeDiffPanelProps.length = 0;
      mergeWorktreeModalProps.length = 0;
      const openCalls: string[] = [];
      const deleteCalls: string[] = [];
      server.use(
        ...taskWorkspaceHandlers(
          scenario.card,
          scenario.records,
          openCalls,
          deleteCalls,
        ),
      );

      const { user } = render(<TaskWorkspace taskId={TASK_ID} />, {
        preloadedState: workspacePreloadedState(
          scenario.card.agent_chat_id ?? "agent-T-1",
          scenario.threadWorktree,
        ),
      });

      await user.click(await openCardDetail(scenario.card));
      const viewDiff = screen.getByRole("button", { name: "View Diff" });
      const merge = screen.getByRole("button", { name: "Merge" });
      const open = screen.getByRole("button", { name: "Open" });
      const discard = screen.getByRole("button", { name: "Discard/Delete" });

      if (!scenario.expectedId) {
        expect(viewDiff).toBeDisabled();
        expect(merge).toBeDisabled();
        expect(open).toBeDisabled();
        expect(discard).toBeDisabled();
        expect(openedIds(worktreeDiffPanelProps)).toEqual([]);
        expect(openedIds(mergeWorktreeModalProps)).toEqual([]);
        expect(openCalls).toEqual([]);
        expect(deleteCalls).toEqual([]);
        continue;
      }

      await user.click(viewDiff);
      expect(openedIds(worktreeDiffPanelProps)).toEqual([]);
      await waitFor(() =>
        expect(
          screen.getByText("No changed files reported."),
        ).toBeInTheDocument(),
      );
      const diffRequest = encodeURIComponent(scenario.expectedId);
      const legacyRequest = encodeURIComponent(LEGACY_PATH);
      expect(document.body.innerHTML).toContain(diffRequest);
      expect(document.body.innerHTML).not.toContain(legacyRequest);

      await user.click(screen.getByRole("button", { name: "Close" }));
      document.body.style.pointerEvents = "";
      await user.click(merge);
      expect(openedIds(mergeWorktreeModalProps)).toEqual([]);
      const mergeDialog = screen.getByRole("dialog", {
        name: "Merge worktree",
      });
      expect(mergeDialog).toBeInTheDocument();
      expect(mergeDialog).toHaveTextContent(
        scenario.records[0].meta.branch ?? "",
      );
      expect(mergeDialog.innerHTML).not.toContain(LEGACY_PATH);

      await user.click(screen.getByRole("button", { name: "Close" }));
      document.body.style.pointerEvents = "";
      await user.click(open);
      await user.click(discard);
      await user.click(
        await screen.findByRole("button", { name: "Delete worktree" }),
      );

      await waitFor(() => expect(openCalls).toEqual([scenario.expectedId]));
      await waitFor(() => expect(deleteCalls).toEqual([scenario.expectedId]));
      expect([
        ...openCalls,
        ...deleteCalls,
        ...openedIds(worktreeDiffPanelProps),
        ...openedIds(mergeWorktreeModalProps),
      ]).not.toContain(LEGACY_PATH);
    }
  });
});

describe("TaskWorkspace layout and chat surfaces", () => {
  beforeEach(() => {
    clearWorkspaceStorage();
  });

  it("persists_workspace_tab_choice_round_trip", async () => {
    setProjectStorageNamespace("ds-204-layout");
    const card = makeCard();
    server.use(...taskWorkspaceHandlers(card, []));

    const { user, unmount } = render(<TaskWorkspace taskId={TASK_ID} />, {
      preloadedState: workspacePreloadedState(PLANNER_ID),
    });

    await screen.findAllByText(card.title);
    await user.click(screen.getByRole("tab", { name: "Memories" }));

    await waitFor(() => expect(loadTaskWorkspaceTab(TASK_ID)).toBe("memories"));

    unmount();
    server.resetHandlers();
    server.use(...taskWorkspaceHandlers(card, []));
    render(<TaskWorkspace taskId={TASK_ID} />, {
      preloadedState: workspacePreloadedState(PLANNER_ID),
    });

    await waitFor(() =>
      expect(screen.getByRole("tab", { name: "Memories" })).toHaveAttribute(
        "data-state",
        "active",
      ),
    );
  });

  it("defaults_to_board_tab_for_fresh_tasks_without_planners", async () => {
    const card = makeCard();
    server.use(...taskWorkspaceHandlers(card, []));

    render(<TaskWorkspace taskId={TASK_ID} />, {
      preloadedState: workspacePreloadedState(),
    });

    await screen.findAllByText(card.title);
    expect(screen.getByRole("tab", { name: /^Board/ })).toHaveAttribute(
      "data-state",
      "active",
    );
    expect(
      screen.getByRole("button", { name: "New task planner" }),
    ).toBeVisible();
  });

  it("defaults_to_chat_tab_when_saved_planners_exist", async () => {
    const card = makeCard();
    server.use(...taskWorkspaceHandlers(card, []));
    server.use(
      http.get("*/v1/tasks/task-1/trajectories/planner", () =>
        HttpResponse.json([
          {
            id: PLANNER_ID,
            title: "Saved planner",
            created_at: "2026-01-01T00:00:00Z",
            updated_at: "2026-01-02T00:00:00Z",
          },
        ]),
      ),
    );

    render(<TaskWorkspace taskId={TASK_ID} />, {
      preloadedState: workspacePreloadedState(PLANNER_ID),
    });

    await waitFor(() =>
      expect(screen.getByRole("tab", { name: "Chat" })).toHaveAttribute(
        "data-state",
        "active",
      ),
    );
    expect(
      screen.getByRole("button", { name: "Switch chat" }),
    ).toBeInTheDocument();
    expect(screen.queryByText(card.title)).not.toBeInTheDocument();
  });

  it("board_tab_shows_running_and_waiting_badges", async () => {
    const card = makeCard({ column: "doing", agent_chat_id: "agent-T-1" });
    server.use(...taskWorkspaceHandlers(card, []));
    server.use(
      http.get("*/v1/tasks/task-1/trajectories/planner", () =>
        HttpResponse.json([
          {
            id: PLANNER_ID,
            title: "Waiting planner",
            created_at: "2026-01-01T00:00:00Z",
            updated_at: "2026-01-02T00:00:00Z",
            session_state: "waiting_user_input",
            waiting_for_card_ids: [CARD_ID],
          },
        ]),
      ),
    );

    render(<TaskWorkspace taskId={TASK_ID} />, {
      preloadedState: workspacePreloadedState(PLANNER_ID),
    });

    const boardTab = await screen.findByRole("tab", { name: /^Board/ });
    await waitFor(() =>
      expect(within(boardTab).getByTitle(/running agent/)).toHaveTextContent(
        "1",
      ),
    );
    await waitFor(() =>
      expect(
        within(boardTab).getByTitle(/waiting for input/),
      ).toHaveTextContent("1"),
    );
  });

  it("board_rail_shows_linked_card_badge_for_agent_spawned_by_chat", async () => {
    const card = makeCard({ column: "doing", agent_chat_id: "agent-T-1" });
    server.use(...taskWorkspaceHandlers(card, []));
    server.use(
      http.get("*/v1/tasks/task-1/trajectories/planner", () =>
        HttpResponse.json([
          {
            id: PLANNER_ID,
            title: "Spawning planner",
            created_at: "2026-01-01T00:00:00Z",
            updated_at: "2026-01-02T00:00:00Z",
          },
        ]),
      ),
      http.get("*/v1/tasks/task-1/trajectories/agents", () =>
        HttpResponse.json([
          {
            id: "agent-T-1",
            title: "Agent T-1",
            created_at: "2026-01-01T00:00:00Z",
            updated_at: "2026-01-02T00:00:00Z",
            parent_id: PLANNER_ID,
          },
        ]),
      ),
    );

    const { user } = render(<TaskWorkspace taskId={TASK_ID} />, {
      preloadedState: workspacePreloadedState(PLANNER_ID),
    });

    await user.click(await screen.findByRole("tab", { name: /^Board/ }));

    const linkedCards = await screen.findByTestId(
      `planner-linked-cards-${PLANNER_ID}`,
    );
    expect(within(linkedCards).getByText(CARD_ID)).toBeInTheDocument();
  });

  it("planner_and_agents_overflow_inside_single_panel_scroll_owners", async () => {
    const planners = Array.from({ length: 18 }, (_, index) => ({
      id: `planner-overflow-${index}`,
      title: `Overflow planner ${index}`,
      created_at: `2026-01-01T00:${String(index).padStart(2, "0")}:00Z`,
      updated_at: `2026-01-01T00:${String(index).padStart(2, "0")}:00Z`,
    }));
    const cards = Array.from({ length: 24 }, (_, index) =>
      makeCard({
        id: `T-${index}`,
        title: `Overflow agent ${index}`,
        column: index % 3 === 0 ? "doing" : index % 3 === 1 ? "done" : "failed",
        agent_chat_id: `agent-overflow-${index}`,
      }),
    );
    server.use(...taskWorkspaceHandlers(cards[0], []));
    server.use(
      http.get("*/v1/tasks/task-1/trajectories/planner", () =>
        HttpResponse.json(planners),
      ),
      http.get("*/v1/tasks/task-1/board", () =>
        HttpResponse.json({ ...makeBoard(cards[0]), cards }),
      ),
    );

    const { user } = render(
      <div style={{ height: 360 }}>
        <TaskWorkspace taskId={TASK_ID} />
      </div>,
      {
        preloadedState: workspacePreloadedState("planner-overflow-0"),
      },
    );

    await user.click(await screen.findByRole("tab", { name: /^Board/ }));

    await screen.findByText("Overflow planner 1");
    await waitFor(() =>
      expect(screen.getAllByText("Overflow agent 23").length).toBeGreaterThan(
        1,
      ),
    );

    const plannerScrollOwners = screen.getAllByTestId(
      "planner-panel-scroll-owner",
    );
    const agentsScrollOwners = screen.getAllByTestId(
      "agents-panel-scroll-owner",
    );
    expect(plannerScrollOwners).toHaveLength(1);
    expect(agentsScrollOwners).toHaveLength(1);
    expect(plannerScrollOwners[0].parentElement?.className).toContain(
      "panelScrollArea",
    );
    expect(agentsScrollOwners[0].parentElement?.className).toContain(
      "panelScrollArea",
    );
    expect(plannerScrollOwners[0].querySelector("div")).not.toBeNull();
    expect(agentsScrollOwners[0].querySelector("div")).not.toBeNull();

    const css = await readGuiSource("features/Tasks/Tasks.module.css");
    const boardRail = readCssBlock(css, ".boardRail");
    const panelList = readCssBlock(css, ".panelList");
    const panelContent = readCssBlock(css, ".panelContent");
    const panelScrollArea = readCssBlock(css, ".panelScrollArea");

    expect(boardRail).toContain("min-height: 0");
    expect(boardRail).toContain("min-width: 0");
    expect(panelList).toContain("min-height: 0");
    expect(panelContent).toContain("min-height: 0");
    expect(panelScrollArea).toContain("flex: 1 1 0");
    expect(panelScrollArea).toContain("min-height: 0");
    expect(panelScrollArea).toContain("overflow: hidden");
  });

  it("selects_planner_and_agent_chats_and_renders_workspace_tabs", async () => {
    const card = makeCard({
      title: "Agent selectable card",
      column: "doing",
      agent_chat_id: "agent-T-1",
      assignee: "agent-1",
    });
    server.use(...taskWorkspaceHandlers(card, []));
    server.use(
      http.get("*/v1/tasks/task-1/trajectories/planner", () =>
        HttpResponse.json([
          {
            id: PLANNER_ID,
            title: "Planner Restored",
            created_at: "2026-01-01T00:00:00Z",
            updated_at: "2026-01-02T00:00:00Z",
          },
        ]),
      ),
    );

    const { store, user } = render(<TaskWorkspace taskId={TASK_ID} />, {
      preloadedState: {
        ...workspacePreloadedState(PLANNER_ID),
        tasksUI: {
          openTasks: [
            {
              id: TASK_ID,
              name: "Task with worktree",
              plannerChats: [
                {
                  id: PLANNER_ID,
                  title: "Planner Restored",
                  createdAt: "2026-01-01T00:00:00Z",
                  updatedAt: "2026-01-02T00:00:00Z",
                },
              ],
              activeChat: { type: "planner", chatId: PLANNER_ID },
            },
          ],
        },
      },
    });

    await waitFor(() =>
      expect(screen.getByRole("tab", { name: "Chat" })).toHaveAttribute(
        "data-state",
        "active",
      ),
    );
    expect(screen.getByRole("tab", { name: /^Board/ })).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: "Memories" })).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: "Documents" })).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Switch chat" }),
    ).toHaveTextContent("Planner");

    await user.click(screen.getByRole("tab", { name: /^Board/ }));
    await screen.findAllByText(card.title);
    await user.click(screen.getByRole("button", { name: "Agent" }));

    await waitFor(() =>
      expect(
        store.getState().tasksUI.openTasks.find((task) => task.id === TASK_ID)
          ?.activeChat,
      ).toEqual({ type: "agent", cardId: CARD_ID, chatId: "agent-T-1" }),
    );
    await waitFor(() =>
      expect(screen.getByRole("tab", { name: "Chat" })).toHaveAttribute(
        "data-state",
        "active",
      ),
    );
    expect(
      screen.getByText(/Agent: T-1 Agent selectable card/),
    ).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Switch chat" }));
    await user.click(screen.getByRole("button", { name: /Open chat/ }));

    await waitFor(() =>
      expect(
        store.getState().tasksUI.openTasks.find((task) => task.id === TASK_ID)
          ?.activeChat,
      ).toEqual({ type: "planner", chatId: PLANNER_ID }),
    );
    expect(
      screen.getByRole("button", { name: "Switch chat" }),
    ).toHaveTextContent("Planner");

    await user.click(screen.getByRole("tab", { name: "Memories" }));
    await screen.findByText(/memories shown/i);
    await user.click(screen.getByRole("tab", { name: "Documents" }));
    await screen.findByText(/No documents yet/);
  });
});

describe("TaskWorkspace SSE invalidation", () => {
  it("simulated_board_changed_event_updates_board_without_refetch", async () => {
    const card = makeCard();
    const updatedCard = makeCard({ title: "Updated board card" });
    let boardFetchCount = 0;
    server.use(...taskWorkspaceHandlers(card, []));
    server.use(
      http.get("*/v1/tasks/task-1/board", () => {
        boardFetchCount++;
        return HttpResponse.json(makeBoard(card));
      }),
    );

    const { store } = render(<TaskWorkspace taskId={TASK_ID} />, {
      preloadedState: workspacePreloadedState(),
    });

    await screen.findAllByText(card.title);
    const initialCount = boardFetchCount;

    store.dispatch(
      taskSseEventReceived({
        type: "board_changed",
        task_id: TASK_ID,
        rev: 2,
        board: makeBoard(updatedCard),
      }),
    );

    await screen.findAllByText(updatedCard.title);
    expect(boardFetchCount).toBe(initialCount);
  });

  it("selected_card_modal_shows_latest_data_after_board_refresh", async () => {
    const card = makeCard({ status_updates: [] });
    const updatedCard = makeCard({
      status_updates: [
        { timestamp: "2026-01-01T00:00:00Z", message: "Agent progress update" },
      ],
    });
    let returnUpdated = false;
    server.use(...taskWorkspaceHandlers(card, []));
    server.use(
      http.get("*/v1/tasks/task-1/board", () =>
        HttpResponse.json(makeBoard(returnUpdated ? updatedCard : card)),
      ),
    );

    const { store, user } = render(<TaskWorkspace taskId={TASK_ID} />, {
      preloadedState: workspacePreloadedState(),
    });

    await user.click(await openCardDetail(card));
    expect(screen.queryByText(/Agent progress update/)).not.toBeInTheDocument();

    returnUpdated = true;
    store.dispatch(
      taskSseEventReceived({
        type: "board_changed",
        task_id: TASK_ID,
        rev: 2,
        board: makeBoard(updatedCard),
      }),
    );

    await screen.findByText(/Agent progress update/);
  });

  it("selected_card_modal_closes_with_notification_when_card_deleted", async () => {
    const card = makeCard({
      agent_worktree: undefined,
      agent_worktree_name: undefined,
      agent_branch: undefined,
    });
    server.use(...taskWorkspaceHandlers(card, []));

    const { store, user } = render(<TaskWorkspace taskId={TASK_ID} />, {
      preloadedState: workspacePreloadedState(),
    });

    await user.click(await openCardDetail(card));
    expect(screen.getByRole("button", { name: "Close" })).toBeInTheDocument();

    server.use(
      http.get("*/v1/tasks/task-1/board", () =>
        HttpResponse.json({ ...makeBoard(card), cards: [] }),
      ),
    );

    store.dispatch(
      taskSseEventReceived({
        type: "board_changed",
        task_id: TASK_ID,
        rev: 2,
        board: { ...makeBoard(card), cards: [] },
      }),
    );

    await waitFor(() =>
      expect(
        screen.queryByRole("button", { name: "Close" }),
      ).not.toBeInTheDocument(),
    );
    expect(
      screen.getByText("Card was deleted by another planner."),
    ).toBeInTheDocument();
  });

  it("task_updated_event_refreshes_task_meta", async () => {
    const card = makeCard();
    let returnActive = false;
    server.use(...taskWorkspaceHandlers(card, []));
    server.use(
      http.get("*/v1/tasks/task-1", () =>
        HttpResponse.json({
          meta: {
            ...makeTask(),
            status: returnActive ? "active" : "planning",
          },
        }),
      ),
    );

    const { store } = render(<TaskWorkspace taskId={TASK_ID} />, {
      preloadedState: workspacePreloadedState(),
    });

    await screen.findAllByText(card.title);

    returnActive = true;
    store.dispatch(
      taskSseEventReceived({
        type: "task_updated",
        task_id: TASK_ID,
        meta: { ...makeTask(), status: "active" },
      }),
    );

    await screen.findByText("Planning complete! You can now spawn agents.");
  });

  it("task_updated_event_replaces_stale_planner_session_state", async () => {
    const card = makeCard();
    server.use(...taskWorkspaceHandlers(card, []));

    const { store } = render(<TaskWorkspace taskId={TASK_ID} />, {
      preloadedState: workspacePreloadedState(),
    });

    await screen.findAllByText(card.title);
    store.dispatch(
      taskSseEventReceived({
        type: "task_updated",
        task_id: TASK_ID,
        meta: { ...makeTask(), planner_session_state: "paused" },
      }),
    );

    await waitFor(() =>
      expect(
        tasksApi.endpoints.listTasks.select(undefined)(store.getState())
          .data?.[0].planner_session_state,
      ).toBe("paused"),
    );

    store.dispatch(
      taskSseEventReceived({
        type: "task_updated",
        task_id: TASK_ID,
        meta: { ...makeTask(), updated_at: "2026-04-30T00:00:01Z" },
      }),
    );

    await waitFor(() =>
      expect(
        tasksApi.endpoints.listTasks.select(undefined)(store.getState())
          .data?.[0].planner_session_state,
      ).toBeUndefined(),
    );
    expect(
      tasksApi.endpoints.getTask.select(TASK_ID)(store.getState()).data
        ?.planner_session_state,
    ).toBeUndefined();
  });

  it("task_updated_event_refreshes_open_task_name_and_list_cache", async () => {
    const card = makeCard();
    server.use(...taskWorkspaceHandlers(card, []));

    const { store } = render(<TaskWorkspace taskId={TASK_ID} />, {
      preloadedState: workspacePreloadedState(),
    });

    await screen.findAllByText(card.title);

    store.dispatch(
      taskSseEventReceived({
        type: "task_updated",
        task_id: TASK_ID,
        meta: {
          ...makeTask(),
          name: "Renamed task",
          updated_at: "2026-04-30T00:00:01Z",
        },
      }),
    );

    await waitFor(() =>
      expect(
        tasksApi.endpoints.listTasks.select(undefined)(store.getState())
          .data?.[0].name,
      ).toBe("Renamed task"),
    );
    await waitFor(() =>
      expect(
        store.getState().tasksUI.openTasks.find((task) => task.id === TASK_ID)
          ?.name,
      ).toBe("Renamed task"),
    );
  });

  it("task_deleted_event_clears_open_workspace_and_subresource_caches", async () => {
    const card = makeCard();
    let deleted = false;
    server.use(...taskWorkspaceHandlers(card, []));
    server.use(
      http.get("*/v1/tasks/task-1", () =>
        deleted
          ? HttpResponse.json({ error: "not found" }, { status: 404 })
          : HttpResponse.json({ meta: makeTask() }),
      ),
      http.get("*/v1/tasks/task-1/board", () =>
        deleted
          ? HttpResponse.json({ error: "not found" }, { status: 404 })
          : HttpResponse.json(makeBoard(card)),
      ),
      http.get("*/v1/tasks/task-1/trajectories/planner", () =>
        deleted
          ? HttpResponse.json({ error: "not found" }, { status: 404 })
          : HttpResponse.json([]),
      ),
      http.get("*/v1/task/:id/documents", () =>
        deleted
          ? HttpResponse.json({ error: "not found" }, { status: 404 })
          : HttpResponse.json({ task_id: TASK_ID, documents: [] }),
      ),
      http.get("*/v1/task/:id/memories", () =>
        deleted
          ? HttpResponse.json({ error: "not found" }, { status: 404 })
          : HttpResponse.json({
              task_id: TASK_ID,
              since: "",
              new_count: 0,
              memories: [],
              warnings: [],
            }),
      ),
    );

    const { store } = render(<TaskWorkspace taskId={TASK_ID} />, {
      preloadedState: workspacePreloadedState(),
    });

    await screen.findAllByText(card.title);
    store.dispatch(push({ name: "task workspace", taskId: TASK_ID }));
    store.dispatch(openTask({ id: TASK_ID, name: "Task with worktree" }));
    const taskThreadId = "agent-task-deleted";
    store.dispatch(
      createChatWithId({
        id: taskThreadId,
        title: "Deleted task agent",
        isTaskChat: true,
        mode: "TASK_AGENT",
        taskMeta: {
          task_id: TASK_ID,
          role: "agents",
          card_id: CARD_ID,
        },
      }),
    );
    store.dispatch(switchToThread({ id: taskThreadId, openTab: false }));

    await store.dispatch(
      tasksApi.endpoints.listTaskTrajectories.initiate({
        taskId: TASK_ID,
        role: "planner",
      }),
    );
    await store.dispatch(
      taskDocumentsApi.endpoints.listTaskDocuments.initiate({
        taskId: TASK_ID,
      }),
    );
    await store.dispatch(
      taskMemoriesApi.endpoints.listTaskMemories.initiate({ taskId: TASK_ID }),
    );

    expect(
      taskDocumentsApi.endpoints.listTaskDocuments.select({ taskId: TASK_ID })(
        store.getState(),
      ).status,
    ).toBe("fulfilled");
    expect(
      taskMemoriesApi.endpoints.listTaskMemories.select({ taskId: TASK_ID })(
        store.getState(),
      ).status,
    ).toBe("fulfilled");

    deleted = true;
    store.dispatch(
      taskSseEventReceived({ type: "task_deleted", task_id: TASK_ID }),
    );

    await waitFor(() =>
      expect(
        store.getState().tasksUI.openTasks.some((task) => task.id === TASK_ID),
      ).toBe(false),
    );
    expect(store.getState().pages.at(-1)).toEqual({ name: "history" });
    expect(store.getState().chat.threads[taskThreadId]).toBeUndefined();
    await screen.findByText("Task is no longer available.");
    await waitFor(() =>
      expect(
        tasksApi.endpoints.getTask.select(TASK_ID)(store.getState()).status,
      ).toBe("rejected"),
    );
    expect(
      tasksApi.endpoints.getBoard.select(TASK_ID)(store.getState()).status,
    ).toBe("rejected");
    expect(
      tasksApi.endpoints.listTaskTrajectories.select({
        taskId: TASK_ID,
        role: "planner",
      })(store.getState()).status,
    ).toBe("rejected");
    expect(
      taskDocumentsApi.endpoints.listTaskDocuments.select({ taskId: TASK_ID })(
        store.getState(),
      ).status,
    ).toBe("rejected");
    expect(
      taskMemoriesApi.endpoints.listTaskMemories.select({ taskId: TASK_ID })(
        store.getState(),
      ).status,
    ).toBe("rejected");
  });

  it("task_comments_changed_event_refetches_board_comments", async () => {
    const card = makeCard({ comments: [] });
    const updatedCard = makeCard({
      comments: [
        {
          id: "comment-2",
          author_role: "agents",
          author_id: "agent-2",
          timestamp: "2026-01-03T00:00:00Z",
          body: "External agent comment",
          reply_to: null,
        },
      ],
    });
    let returnUpdated = false;
    server.use(...taskWorkspaceHandlers(card, []));
    server.use(
      http.get("*/v1/tasks/task-1/board", () =>
        HttpResponse.json(makeBoard(returnUpdated ? updatedCard : card)),
      ),
    );

    const { store, user } = render(<TaskWorkspace taskId={TASK_ID} />, {
      preloadedState: workspacePreloadedState(),
    });

    await user.click(await openCardDetail(card));
    expect(
      screen.queryByText("External agent comment"),
    ).not.toBeInTheDocument();

    returnUpdated = true;
    store.dispatch(
      taskSseEventReceived({
        type: "task_comments_changed",
        task_id: TASK_ID,
        card_id: CARD_ID,
      }),
    );

    await screen.findByText("External agent comment");
  });

  it("task_document_changed_event_refetches_open_documents_panel", async () => {
    const card = makeCard();
    let returnUpdated = false;
    server.use(...taskWorkspaceHandlers(card, []));
    server.use(
      http.get("*/v1/task/:id/documents", () =>
        HttpResponse.json({
          task_id: TASK_ID,
          documents: returnUpdated
            ? [
                {
                  slug: "main-plan",
                  name: "Main Plan",
                  kind: "plan",
                  pinned: true,
                  version: 1,
                  updated_at: "2026-01-03T00:00:00Z",
                  created_at: "2026-01-03T00:00:00Z",
                  author_role: "planner",
                  relevant_cards: [],
                },
              ]
            : [],
        }),
      ),
    );

    const { store, user } = render(<TaskWorkspace taskId={TASK_ID} />, {
      preloadedState: workspacePreloadedState(),
    });

    await screen.findAllByText(card.title);
    await user.click(screen.getByRole("tab", { name: "Documents" }));
    await screen.findByText(/No documents yet/);

    returnUpdated = true;
    store.dispatch(
      taskSseEventReceived({
        type: "task_document_changed",
        task_id: TASK_ID,
        slug: "main-plan",
      }),
    );

    await screen.findByText("Main Plan");
  });

  it("task_memories_changed_event_refetches_open_memories_panel", async () => {
    const card = makeCard();
    let returnUpdated = false;
    server.use(...taskWorkspaceHandlers(card, []));
    server.use(
      http.get("*/v1/task/:id/memories", () =>
        HttpResponse.json({
          task_id: TASK_ID,
          since: "",
          new_count: returnUpdated ? 1 : 0,
          memories: returnUpdated
            ? [
                {
                  filename: "decision.md",
                  created_at: "2026-01-03T00:00:00Z",
                  created_at_known: true,
                  title: "External memory",
                  content: "Remember the refresh path.",
                  tags: ["refresh-tag"],
                  kind: "decision",
                  namespace: "task",
                  pinned: false,
                  status: "active",
                },
              ]
            : [],
          warnings: [],
        }),
      ),
      http.get("*/v1/task/:id/memories/facets", () =>
        HttpResponse.json({
          task_id: TASK_ID,
          namespaces: ["task"],
          tags: returnUpdated ? ["refresh-tag"] : [],
          kinds: ["decision"],
          total_count: returnUpdated ? 1 : 0,
          pinned_count: 0,
        }),
      ),
    );

    const { store, user } = render(<TaskWorkspace taskId={TASK_ID} />, {
      preloadedState: workspacePreloadedState(),
    });

    await screen.findAllByText(card.title);
    await user.click(screen.getByRole("tab", { name: "Memories" }));
    await screen.findByText(/No memories match/);

    returnUpdated = true;
    store.dispatch(
      taskSseEventReceived({
        type: "task_memories_changed",
        task_id: TASK_ID,
      }),
    );

    await screen.findByText("External memory");
    await screen.findByText(/Show all 1 tags/);
  });

  it("visibilitychange_to_visible_invalidates_board", async () => {
    const card = makeCard();
    let boardFetchCount = 0;
    server.use(...taskWorkspaceHandlers(card, []));
    server.use(
      http.get("*/v1/tasks/task-1/board", () => {
        boardFetchCount++;
        return HttpResponse.json(makeBoard(card));
      }),
    );

    render(<TaskWorkspace taskId={TASK_ID} />, {
      preloadedState: workspacePreloadedState(),
    });

    await screen.findAllByText(card.title);
    const initialCount = boardFetchCount;

    document.dispatchEvent(new Event("visibilitychange"));

    await waitFor(() => expect(boardFetchCount).toBeGreaterThan(initialCount));
  });
});

describe("TaskWorkspace planner CRUD", () => {
  function makePlannerTrajectory(): TrajectoryInfo {
    return {
      id: PLANNER_ID,
      title: "Test Planner",
      created_at: "2026-01-01T00:00:00Z",
      updated_at: "2026-01-01T00:00:00Z",
    };
  }

  it("delete_planner_failure_restores_local_state", async () => {
    server.use(...taskWorkspaceHandlers(makeCard(), []));
    server.use(
      http.get("*/v1/tasks/task-1/trajectories/planner", () =>
        HttpResponse.json([makePlannerTrajectory()]),
      ),
      http.delete(`*/v1/tasks/${TASK_ID}/planner-chats/${PLANNER_ID}`, () =>
        HttpResponse.json({ error: "Internal error" }, { status: 500 }),
      ),
    );

    const { user } = render(<TaskWorkspace taskId={TASK_ID} />, {
      preloadedState: workspacePreloadedState(),
    });

    await user.click(await screen.findByRole("tab", { name: /^Board/ }));

    const deleteBtn = await screen.findByRole("button", {
      name: "Delete chat",
      hidden: true,
    });
    await user.click(deleteBtn);

    await waitFor(() =>
      expect(screen.getByText(/Delete failed/)).toBeInTheDocument(),
    );
    expect(
      screen.getByRole("button", { name: "Delete chat", hidden: true }),
    ).toBeInTheDocument();
  });

  it("delete_planner_409_shows_agent_refs_in_notification", async () => {
    server.use(...taskWorkspaceHandlers(makeCard(), []));
    server.use(
      http.get("*/v1/tasks/task-1/trajectories/planner", () =>
        HttpResponse.json([makePlannerTrajectory()]),
      ),
      http.delete(`*/v1/tasks/${TASK_ID}/planner-chats/${PLANNER_ID}`, () =>
        HttpResponse.json(
          {
            error: "Referenced by agents",
            agent_refs: [{ chat_id: "agent-ref-1" }],
          },
          { status: 409 },
        ),
      ),
    );

    const { user } = render(<TaskWorkspace taskId={TASK_ID} />, {
      preloadedState: workspacePreloadedState(),
    });

    await user.click(await screen.findByRole("tab", { name: /^Board/ }));

    const deleteBtn = await screen.findByRole("button", {
      name: "Delete chat",
      hidden: true,
    });
    await user.click(deleteBtn);

    await screen.findByText(/agent-ref-1/);
  });

  it("cached_savedPlanners_does_not_resurrect_deleted_planner", async () => {
    server.use(...taskWorkspaceHandlers(makeCard(), []));
    server.use(
      http.get("*/v1/tasks/task-1/trajectories/planner", () =>
        HttpResponse.json([]),
      ),
    );

    render(<TaskWorkspace taskId={TASK_ID} />, {
      preloadedState: workspacePreloadedState(),
    });

    await screen.findAllByText(makeCard().title);

    await waitFor(() =>
      expect(screen.getByText("No chats yet")).toBeInTheDocument(),
    );
    expect(
      screen.queryByRole("button", {
        name: "Delete chat",
        hidden: true,
      }),
    ).not.toBeInTheDocument();
  });

  it("create_planner_failure_shows_notification", async () => {
    server.use(...taskWorkspaceHandlers(makeCard(), []));
    server.use(
      http.get("*/v1/tasks/task-1/trajectories/planner", () =>
        HttpResponse.json([]),
      ),
      http.post(`*/v1/tasks/${TASK_ID}/planner-chats`, () =>
        HttpResponse.json({ error: "Server error" }, { status: 500 }),
      ),
    );

    const { user } = render(<TaskWorkspace taskId={TASK_ID} />, {
      preloadedState: workspacePreloadedState(),
    });

    await screen.findAllByText(makeCard().title);

    await user.click(screen.getByRole("button", { name: "New task planner" }));

    await screen.findByText(/Create failed/);
  });
});

describe("TaskWorkspace planner restore race", () => {
  it("prunes_stale_persisted_planner_without_switching_to_missing_runtime", async () => {
    const warnSpy = vi
      .spyOn(console, "warn")
      .mockImplementation(() => undefined);
    const card = makeCard();
    server.use(...taskWorkspaceHandlers(card, []));

    try {
      const preloaded = workspacePreloadedState("unrelated-chat");
      const { store } = render(<TaskWorkspace taskId={TASK_ID} />, {
        preloadedState: {
          ...preloaded,
          tasksUI: {
            openTasks: [
              {
                id: TASK_ID,
                name: "Task with worktree",
                plannerChats: [
                  {
                    id: "planner-missing-runtime",
                    title: "Persisted planner",
                    createdAt: "2026-01-01T00:00:00Z",
                    updatedAt: "2026-01-02T00:00:00Z",
                  },
                ],
                activeChat: {
                  type: "planner",
                  chatId: "planner-missing-runtime",
                },
              },
            ],
          },
        },
      });

      await waitFor(() =>
        expect(
          store
            .getState()
            .tasksUI.openTasks.find((task) => task.id === TASK_ID),
        ).toMatchObject({ plannerChats: [], activeChat: null }),
      );
      expect(store.getState().chat.current_thread_id).toBe("unrelated-chat");
      expect(
        warnSpy.mock.calls.some(([message]) =>
          String(message).includes("[switchToThread] No runtime"),
        ),
      ).toBe(false);
    } finally {
      warnSpy.mockRestore();
    }
  });

  it("hydrates_persisted_active_agent_before_switching", async () => {
    const warnSpy = vi
      .spyOn(console, "warn")
      .mockImplementation(() => undefined);
    const card = makeCard({
      title: "Persisted active agent",
      column: "doing",
      agent_chat_id: "agent-T-1",
    });
    server.use(...taskWorkspaceHandlers(card, []));

    try {
      const preloaded = workspacePreloadedState("unrelated-chat");
      const { store } = render(<TaskWorkspace taskId={TASK_ID} />, {
        preloadedState: {
          ...preloaded,
          tasksUI: {
            openTasks: [
              {
                id: TASK_ID,
                name: "Task with worktree",
                plannerChats: [],
                activeChat: {
                  type: "agent",
                  cardId: CARD_ID,
                  chatId: "agent-T-1",
                },
              },
            ],
          },
        },
      });

      await waitFor(() =>
        expect(store.getState().chat.threads["agent-T-1"]).toMatchObject({
          thread: {
            id: "agent-T-1",
            title: "Agent: T-1 Persisted active agent",
            is_task_chat: true,
            mode: "task_agent",
          },
        }),
      );
      await waitFor(() =>
        expect(store.getState().chat.current_thread_id).toBe("agent-T-1"),
      );
      expect(
        warnSpy.mock.calls.some(([message]) =>
          String(message).includes("[switchToThread] No runtime"),
        ),
      ).toBe(false);
    } finally {
      warnSpy.mockRestore();
    }
  });

  it("re_pins_active_chat_when_global_thread_switches_away", async () => {
    const card = makeCard({
      title: "Persisted active agent",
      column: "doing",
      agent_chat_id: "agent-T-1",
    });
    server.use(...taskWorkspaceHandlers(card, []));

    const preloaded = workspacePreloadedState("unrelated-chat");
    const { store } = render(<TaskWorkspace taskId={TASK_ID} />, {
      preloadedState: {
        ...preloaded,
        tasksUI: {
          openTasks: [
            {
              id: TASK_ID,
              name: "Task with worktree",
              plannerChats: [],
              activeChat: {
                type: "agent",
                cardId: CARD_ID,
                chatId: "agent-T-1",
              },
            },
          ],
        },
      },
    });

    await waitFor(() =>
      expect(store.getState().chat.current_thread_id).toBe("agent-T-1"),
    );

    store.dispatch(switchToThread({ id: "unrelated-chat" }));

    await waitFor(() =>
      expect(store.getState().chat.current_thread_id).toBe("agent-T-1"),
    );
    expect(
      store.getState().tasksUI.openTasks.find((task) => task.id === TASK_ID)
        ?.activeChat,
    ).toEqual({ type: "agent", cardId: CARD_ID, chatId: "agent-T-1" });
  });

  it("waits_for_current_task_ui_before_restoring_saved_planner_selection", async () => {
    const card = makeCard();
    const taskPromise = delay(50).then(() =>
      HttpResponse.json({ meta: makeTask() }),
    );
    server.use(...taskWorkspaceHandlers(card, []));
    server.use(
      http.get("*/v1/tasks/task-1", () => taskPromise),
      http.get("*/v1/tasks/task-1/trajectories/planner", () =>
        HttpResponse.json([
          {
            id: "planner-fast",
            title: "Fast saved planner",
            created_at: "2026-01-01T00:00:00Z",
            updated_at: "2026-01-03T00:00:00Z",
            session_state: "waiting_user_input",
            waiting_for_card_ids: [CARD_ID],
          },
        ]),
      ),
    );

    const { store, user } = render(<TaskWorkspace taskId={TASK_ID} />, {
      preloadedState: {
        ...workspacePreloadedState("unrelated-chat"),
        tasksUI: { openTasks: [] },
      },
    });

    expect(
      store.getState().tasksUI.openTasks.find((task) => task.id === TASK_ID),
    ).toBeUndefined();
    expect(store.getState().chat.current_thread_id).toBe("unrelated-chat");

    await user.click(await screen.findByRole("tab", { name: /^Board/ }));
    await screen.findByText("Fast saved planner");

    await waitFor(() =>
      expect(
        store.getState().tasksUI.openTasks.find((task) => task.id === TASK_ID),
      ).toMatchObject({
        id: TASK_ID,
        plannerChats: [
          expect.objectContaining({
            id: "planner-fast",
            sessionState: "waiting_user_input",
            waitingForCardIds: [CARD_ID],
          }),
        ],
        activeChat: { type: "planner", chatId: "planner-fast" },
      }),
    );
    await waitFor(() =>
      expect(store.getState().chat.current_thread_id).toBe("planner-fast"),
    );
  });
});

describe("TaskWorkspace CardDetail dialog", () => {
  it("escape_closes_card_detail_dialog", async () => {
    const card = makeCard();
    server.use(...taskWorkspaceHandlers(card, []));

    const { user } = render(<TaskWorkspace taskId={TASK_ID} />, {
      preloadedState: workspacePreloadedState(),
    });

    await user.click(await openCardDetail(card));

    expect(
      await screen.findByRole("dialog", { name: /Implement worktree/ }),
    ).toBeInTheDocument();

    await user.keyboard("{Escape}");

    await waitFor(() => {
      expect(
        screen.queryByRole("dialog", { name: /Implement worktree/ }),
      ).not.toBeInTheDocument();
    });
  });

  it("tab_cycles_focus_within_card_detail_dialog", async () => {
    const record = makeRecord();
    const card = makeCard({ agent_worktree_name: record.meta.id });
    server.use(...taskWorkspaceHandlers(card, [record]));

    const { user } = render(<TaskWorkspace taskId={TASK_ID} />, {
      preloadedState: workspacePreloadedState(),
    });

    await user.click(await openCardDetail(card));

    const dialog = await screen.findByRole("dialog", {
      name: /Implement worktree/,
    });
    expect(dialog).toBeInTheDocument();

    await user.tab();
    expect(dialog.contains(document.activeElement)).toBe(true);

    await user.tab();
    expect(dialog.contains(document.activeElement)).toBe(true);
  });
});

describe("TaskWorkspace new-chat mode picker", () => {
  const RICH_MODES = {
    modes: [
      {
        id: "agent",
        title: "Agent",
        description: "Autonomous task execution with the full toolset enabled",
        tools_count: 57,
        thread_defaults: {
          include_project_info: true,
          checkpoints_enabled: true,
          auto_approve_editing_tools: false,
          auto_approve_dangerous_commands: false,
        },
        ui: { order: 1, tags: ["coding", "autonomous"] },
      },
      {
        id: "explore",
        title: "Explore",
        description: "Read-only context gathering with quick tools",
        tools_count: 20,
        thread_defaults: {
          include_project_info: true,
          checkpoints_enabled: false,
          auto_approve_editing_tools: false,
          auto_approve_dangerous_commands: false,
        },
        ui: { order: 2, tags: ["read-only"] },
      },
      {
        id: "task_planner",
        title: "Task Planner",
        description: "Plan and manage a task",
        tools_count: 0,
        thread_defaults: {
          include_project_info: false,
          checkpoints_enabled: false,
          auto_approve_editing_tools: false,
          auto_approve_dangerous_commands: false,
        },
        ui: { order: 999, tags: ["tasks"] },
      },
    ],
    errors: [],
  };

  function configWithModes(base: ReturnType<typeof workspacePreloadedState>) {
    return { ...base.config, dev: true };
  }

  it("renders rich mode rows and excludes planner/agent modes", async () => {
    server.use(...taskWorkspaceHandlers(makeCard(), []));
    server.use(
      http.get("*/v1/chat-modes", () => HttpResponse.json(RICH_MODES)),
    );

    const preloaded = workspacePreloadedState();
    const { user } = render(<TaskWorkspace taskId={TASK_ID} />, {
      preloadedState: { ...preloaded, config: configWithModes(preloaded) },
    });

    await user.click(await screen.findByRole("button", { name: "New chat" }));

    // Rich row content reused from the chat composer's ModeSelect:
    // title + description + tags + tool count (not a bare title + tools list).
    expect(
      await screen.findByText(
        "Autonomous task execution with the full toolset enabled",
      ),
    ).toBeInTheDocument();
    expect(
      screen.getByText("Read-only context gathering with quick tools"),
    ).toBeInTheDocument();
    expect(screen.getByText("coding")).toBeInTheDocument();
    expect(screen.getByText("read-only")).toBeInTheDocument();
    expect(screen.getByText(/57 tools/)).toBeInTheDocument();

    // task_planner / task_agent must not appear in the new-chat menu.
    expect(screen.queryByText("Task Planner")).toBeNull();
  });

  it("stays on the just-created chat and does not bounce to an older planner", async () => {
    server.use(...taskWorkspaceHandlers(makeCard(), []));
    server.use(
      // The saved-planner list lags behind and never returns the new chat,
      // emulating the window before the trajectory index catches up.
      http.get("*/v1/tasks/task-1/trajectories/planner", () =>
        HttpResponse.json([
          {
            id: "planner-existing",
            title: "Existing planner",
            created_at: "2026-01-01T00:00:00Z",
            updated_at: "2026-01-05T00:00:00Z",
          },
        ]),
      ),
      http.get("*/v1/chat-modes", () => HttpResponse.json(RICH_MODES)),
      http.post(`*/v1/tasks/${TASK_ID}/planner-chats`, async ({ request }) => {
        const body = (await request.json()) as { mode?: string };
        return HttpResponse.json({
          chat_id: "planner-new",
          mode: body.mode ?? "agent",
        });
      }),
    );

    const preloaded = workspacePreloadedState("planner-existing");
    const { store, user } = render(<TaskWorkspace taskId={TASK_ID} />, {
      preloadedState: {
        ...preloaded,
        config: configWithModes(preloaded),
        tasksUI: {
          openTasks: [
            {
              id: TASK_ID,
              name: "Task",
              plannerChats: [
                {
                  id: "planner-existing",
                  title: "Existing planner",
                  createdAt: "2026-01-01T00:00:00Z",
                  updatedAt: "2026-01-05T00:00:00Z",
                },
              ],
              activeChat: { type: "planner", chatId: "planner-existing" },
            },
          ],
        },
      },
    });

    await waitFor(() =>
      expect(store.getState().chat.current_thread_id).toBe("planner-existing"),
    );

    await user.click(await screen.findByRole("button", { name: "New chat" }));
    const agentRow = await screen.findByText(
      "Autonomous task execution with the full toolset enabled",
    );
    const agentButton = agentRow.closest("button");
    if (!agentButton) throw new Error("Agent mode row button not found");
    await user.click(agentButton);

    await waitFor(() =>
      expect(store.getState().chat.threads["planner-new"]?.thread.mode).toBe(
        "agent",
      ),
    );

    await waitFor(() =>
      expect(
        store.getState().tasksUI.openTasks.find((t) => t.id === TASK_ID)
          ?.activeChat,
      ).toEqual({ type: "planner", chatId: "planner-new" }),
    );
    await waitFor(() =>
      expect(store.getState().chat.current_thread_id).toBe("planner-new"),
    );

    // The new chat must remain in the list (not removed by reconciliation).
    expect(
      store
        .getState()
        .tasksUI.openTasks.find((t) => t.id === TASK_ID)
        ?.plannerChats.some((p) => p.id === "planner-new"),
    ).toBe(true);
  });
});
