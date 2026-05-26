import { beforeEach, describe, expect, it, vi } from "vitest";
import { http, HttpResponse } from "msw";
import { cleanup, render, screen, waitFor } from "../../utils/test-utils";
import {
  PlannerItem,
  TaskWorkspace,
  resolveCardWorktree,
} from "./TaskWorkspace";
import type { PlannerInfo } from "./tasksSlice";
import { taskSseEventReceived } from "./tasksSlice";
import type { ChatThreadRuntime } from "../Chat/Thread/types";
import type {
  BoardCard,
  TaskBoard,
  TaskMeta,
  TrajectoryInfo,
} from "../../services/refact/tasks";
import type {
  WorktreeListResponse,
  WorktreeMeta,
  WorktreeRecordView,
} from "../../services/refact";
import { server } from "../../utils/mockServer";

const TASK_ID = "task-1";
const CARD_ID = "T-1";
const PLANNER_ID = "planner-test-1";
const LEGACY_PATH = "/tmp/refact/legacy/wt-path";
const LEGACY_TOOLTIP =
  "This worktree was created before the registry; recreate it via `restart_agent(mode=fresh)` to enable actions.";

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
): WorktreeRecordView {
  const meta = makeMeta(metaOverrides);
  const referenceCount = meta.reference_count ?? 1;
  return {
    meta,
    created_at: "2026-04-30T00:00:00Z",
    updated_at: "2026-04-30T00:00:00Z",
    references: [],
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
    http.get("http://127.0.0.1:8001/v1/tasks/task-1", () =>
      HttpResponse.json({ meta: makeTask() }),
    ),
    http.get("http://127.0.0.1:8001/v1/tasks/task-1/board", () =>
      HttpResponse.json(makeBoard(card)),
    ),
    http.get("http://127.0.0.1:8001/v1/tasks/task-1/trajectories/planner", () =>
      HttpResponse.json([]),
    ),
    http.get("http://127.0.0.1:8001/v1/worktrees", () =>
      HttpResponse.json(makeWorktreeList(records)),
    ),
    http.get("http://127.0.0.1:8001/v1/ping", () =>
      HttpResponse.json({ status: "ok" }),
    ),
    http.get("http://127.0.0.1:8001/v1/chat-modes", () =>
      HttpResponse.json({ modes: [], errors: [] }),
    ),
    http.get("http://127.0.0.1:8001/v1/caps", () =>
      HttpResponse.json({ chat_models: [], completion_models: [] }),
    ),
    http.post("http://127.0.0.1:8001/v1/buddy/diagnostics/collect", () =>
      HttpResponse.json({}),
    ),
    http.get("http://127.0.0.1:8001/v1/worktrees/:id/diff", ({ params }) => {
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
    http.post("http://127.0.0.1:8001/v1/worktrees/:id/open", ({ params }) => {
      const id = String(params.id);
      openCalls.push(id);
      return HttpResponse.json({
        id,
        path: `/tmp/refact/${id}`,
        branch: "refact/task/T-1",
        can_open_folder: false,
      });
    }),
    http.delete("http://127.0.0.1:8001/v1/worktrees/:id", ({ params }) => {
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
  return titles[0];
}

function openedIds(props: MockWorktreePanelProps[]): string[] {
  return props
    .filter((prop) => prop.open && prop.worktreeId)
    .map((prop) => prop.worktreeId as string);
}

beforeEach(() => {
  worktreeDiffPanelProps.length = 0;
  mergeWorktreeModalProps.length = 0;
});

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
      name: /Open planner chat/,
    });
    item.focus();
    await user.keyboard("{Enter}");

    expect(onSelect).toHaveBeenCalledOnce();
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

  it("worktree_id_passed_to_apis_is_never_a_filesystem_path", async () => {
    const scenarios: Array<{
      card: BoardCard;
      records: WorktreeRecordView[];
      threadWorktree?: WorktreeMeta | null;
      expectedId?: string;
    }> = [
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

describe("TaskWorkspace SSE invalidation", () => {
  it("simulated_board_changed_event_refreshes_board", async () => {
    const card = makeCard();
    let boardFetchCount = 0;
    server.use(...taskWorkspaceHandlers(card, []));
    server.use(
      http.get("http://127.0.0.1:8001/v1/tasks/task-1/board", () => {
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
        board: makeBoard(card),
      }),
    );

    await waitFor(() => expect(boardFetchCount).toBeGreaterThan(initialCount));
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
      http.get("http://127.0.0.1:8001/v1/tasks/task-1/board", () =>
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
      http.get("http://127.0.0.1:8001/v1/tasks/task-1/board", () =>
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
      http.get("http://127.0.0.1:8001/v1/tasks/task-1", () =>
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

  it("visibilitychange_to_visible_invalidates_board", async () => {
    const card = makeCard();
    let boardFetchCount = 0;
    server.use(...taskWorkspaceHandlers(card, []));
    server.use(
      http.get("http://127.0.0.1:8001/v1/tasks/task-1/board", () => {
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
      http.get(
        "http://127.0.0.1:8001/v1/tasks/task-1/trajectories/planner",
        () => HttpResponse.json([makePlannerTrajectory()]),
      ),
      http.delete(
        `http://127.0.0.1:8001/v1/tasks/${TASK_ID}/planner-chats/${PLANNER_ID}`,
        () => HttpResponse.json({ error: "Internal error" }, { status: 500 }),
      ),
    );

    const { user } = render(<TaskWorkspace taskId={TASK_ID} />, {
      preloadedState: workspacePreloadedState(),
    });

    const deleteBtn = await screen.findByRole("button", {
      name: "Delete planner chat",
      hidden: true,
    });
    await user.click(deleteBtn);

    await waitFor(() =>
      expect(screen.getByText(/Delete failed/)).toBeInTheDocument(),
    );
    expect(
      screen.getByRole("button", { name: "Delete planner chat", hidden: true }),
    ).toBeInTheDocument();
  });

  it("delete_planner_409_shows_agent_refs_in_notification", async () => {
    server.use(...taskWorkspaceHandlers(makeCard(), []));
    server.use(
      http.get(
        "http://127.0.0.1:8001/v1/tasks/task-1/trajectories/planner",
        () => HttpResponse.json([makePlannerTrajectory()]),
      ),
      http.delete(
        `http://127.0.0.1:8001/v1/tasks/${TASK_ID}/planner-chats/${PLANNER_ID}`,
        () =>
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

    const deleteBtn = await screen.findByRole("button", {
      name: "Delete planner chat",
      hidden: true,
    });
    await user.click(deleteBtn);

    await screen.findByText(/agent-ref-1/);
  });

  it("cached_savedPlanners_does_not_resurrect_deleted_planner", async () => {
    server.use(...taskWorkspaceHandlers(makeCard(), []));
    server.use(
      http.get(
        "http://127.0.0.1:8001/v1/tasks/task-1/trajectories/planner",
        () => HttpResponse.json([]),
      ),
    );

    render(<TaskWorkspace taskId={TASK_ID} />, {
      preloadedState: workspacePreloadedState(),
    });

    await screen.findAllByText(makeCard().title);

    await waitFor(() =>
      expect(screen.getByText("No planner chats yet")).toBeInTheDocument(),
    );
    expect(
      screen.queryByRole("button", {
        name: "Delete planner chat",
        hidden: true,
      }),
    ).not.toBeInTheDocument();
  });

  it("create_planner_failure_shows_notification", async () => {
    server.use(...taskWorkspaceHandlers(makeCard(), []));
    server.use(
      http.get(
        "http://127.0.0.1:8001/v1/tasks/task-1/trajectories/planner",
        () => HttpResponse.json([]),
      ),
      http.post(`http://127.0.0.1:8001/v1/tasks/${TASK_ID}/planner-chats`, () =>
        HttpResponse.json({ error: "Server error" }, { status: 500 }),
      ),
    );

    const { user } = render(<TaskWorkspace taskId={TASK_ID} />, {
      preloadedState: workspacePreloadedState(),
    });

    await screen.findAllByText(makeCard().title);

    await user.click(screen.getByRole("button", { name: "New planner" }));

    await screen.findByText(/Create failed/);
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
