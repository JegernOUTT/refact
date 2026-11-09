import { beforeEach, describe, expect, test, vi } from "vitest";
import { http, HttpResponse } from "msw";
import { render, screen, waitFor } from "../../utils/test-utils";
import { server } from "../../utils/mockServer";
import type {
  MergeWorktreeResponse,
  WorktreeMeta,
  WorktreeRecordView,
  WorktreeSyncOperation,
  WorktreeSyncRequest,
  WorktreeSyncResponse,
} from "../../services/refact";
import { MergeWorktreeModal } from "./MergeWorktreeModal";
import { useWorktreeSync } from "../../hooks/useWorktreeActions";

const configState = () => ({
  host: "web" as const,
  lspPort: 8001,
  apiKey: null,
  themeProps: { appearance: "dark" as const },
  features: { images: true, statistics: true, vecdb: true, ast: true },
});

function makeMeta(): WorktreeMeta {
  return {
    id: "wt-1",
    kind: "task_agent",
    root: "/tmp/refact/wt-1",
    source_workspace_root: "/repo",
    repo_root: "/repo",
    branch: "refact/task/T-1",
    base_branch: "main",
    base_commit: "abc123",
    task_id: "task-1",
    card_id: "T-1",
    enforce: true,
  };
}

function makeRecord(dirty = false): WorktreeRecordView {
  return {
    meta: makeMeta(),
    created_at: "2026-04-30T00:00:00Z",
    updated_at: "2026-04-30T00:00:00Z",
    references: [],
    reference_count: 0,
    status: {
      path_exists: true,
      is_git_worktree: true,
      dirty,
      staged_count: 0,
      unstaged_count: dirty ? 1 : 0,
      untracked_count: 0,
      branch: "refact/task/T-1",
      head_commit: "src-head",
    },
  };
}

const conflictMerge: MergeWorktreeResponse = {
  id: "wt-1",
  status: "conflict",
  merged: false,
  strategy: "merge",
  source_branch: "refact/task/T-1",
  target_branch: "main",
  conflict: {
    files: ["src/conflict.ts"],
    aborted: true,
    merge_in_progress: false,
    instructions: "Resolve conflict markers.",
  },
  warnings: [],
};

function operation(
  overrides: Partial<WorktreeSyncOperation> = {},
): WorktreeSyncOperation {
  return {
    operation_id: "op-1",
    revision: 4,
    source_head: "src-head",
    target_branch: "main",
    target_head: "tgt-head",
    phase: "integrating",
    ...overrides,
  };
}

function syncResponse(
  overrides: Partial<WorktreeSyncResponse> = {},
): WorktreeSyncResponse {
  return {
    id: "wt-1",
    status: "idle",
    source_head: "src-head",
    target_branch: "main",
    target_head: "tgt-head",
    operation: null,
    conflicts: [],
    warnings: [],
    instructions: "",
    ...overrides,
  };
}

type SyncServer = {
  calls: WorktreeSyncRequest[];
  urls: string[];
  state: { current: WorktreeSyncResponse };
};

function installSyncServer(
  initial: WorktreeSyncResponse,
  respond?: (
    request: WorktreeSyncRequest,
    state: SyncServer["state"],
  ) => Response | WorktreeSyncResponse | undefined,
  log?: string[],
): SyncServer {
  const context: SyncServer = {
    calls: [],
    urls: [],
    state: { current: initial },
  };
  server.use(
    http.post("*/v1/worktrees/:id/sync", async ({ request }) => {
      const body = (await request.json()) as WorktreeSyncRequest;
      context.calls.push(body);
      context.urls.push(request.url);
      log?.push(`sync:${body.action}`);
      const custom = respond?.(body, context.state);
      if (custom instanceof Response) return custom;
      if (custom) context.state.current = custom;
      return HttpResponse.json(context.state.current);
    }),
  );
  return context;
}

function renderModal(
  props: Partial<React.ComponentProps<typeof MergeWorktreeModal>> = {},
  record = makeRecord(),
) {
  return render(
    <MergeWorktreeModal
      open
      worktreeId={record.meta.id}
      record={record}
      onOpenChange={() => undefined}
      {...props}
    />,
    { preloadedState: { config: configState() } },
  );
}

beforeEach(() => {
  server.use(
    http.get("*/v1/worktrees", () =>
      HttpResponse.json({
        project_hash: "p",
        source_workspace_root: "/repo",
        worktrees: [],
      }),
    ),
  );
});

async function openConflict(user: ReturnType<typeof renderModal>["user"]) {
  server.use(
    http.post("*/v1/worktrees/:id/merge", () =>
      HttpResponse.json(conflictMerge),
    ),
  );
  await user.click(screen.getByRole("button", { name: "Merge" }));
  await screen.findByText("src/conflict.ts");
}

describe("native worktree synchronization", () => {
  test("an older A run cannot clear busy ownership of a new A run", async () => {
    const releases: (() => void)[] = [];
    let completed = 0;
    server.use(
      http.post("*/v1/worktrees/:id/sync", async () => {
        await new Promise<void>((resolve) => releases.push(resolve));
        return HttpResponse.json(syncResponse());
      }),
    );
    function Harness({ id }: { id: string }) {
      const sync = useWorktreeSync({ worktreeId: id, enabled: false });
      return (
        <>
          <span>{sync.busy ? "busy" : "available"}</span>
          <button
            onClick={() =>
              void sync.refresh().then(() => {
                completed += 1;
              })
            }
          >
            Run status
          </button>
        </>
      );
    }
    const { user, rerender } = render(<Harness id="A" />, {
      preloadedState: { config: configState() },
    });
    const run = () =>
      user.click(screen.getByRole("button", { name: "Run status" }));
    await run();
    await waitFor(() => expect(releases).toHaveLength(1));
    rerender(<Harness id="B" />);
    await run();
    await waitFor(() => expect(releases).toHaveLength(2));
    rerender(<Harness id="A" />);
    await run();
    await waitFor(() => expect(releases).toHaveLength(3));
    releases[0]();
    await waitFor(() => expect(completed).toBe(1));
    expect(screen.getByText("busy")).toBeVisible();
    await run();
    expect(releases).toHaveLength(3);
    releases[1]();
    releases[2]();
    await waitFor(() => expect(screen.getByText("available")).toBeVisible());
  });

  test.each(["worktree round trip", "close and reopen"])(
    "ignores a pre-start status after %s",
    async (transition) => {
      let release: () => void = () => undefined;
      const gate = new Promise<void>((resolve) => {
        release = resolve;
      });
      let statusCalls = 0;
      const sync = installSyncServer(syncResponse());
      server.use(
        http.post("*/v1/worktrees/:id/sync", async ({ request }) => {
          const body = (await request.json()) as WorktreeSyncRequest;
          sync.calls.push(body);
          if (body.action === "status" && ++statusCalls === 2) await gate;
          return HttpResponse.json(syncResponse());
        }),
      );
      const element = (id = "wt-1", open = true) => (
        <MergeWorktreeModal
          open={open}
          worktreeId={id}
          record={makeRecord()}
          onOpenChange={() => undefined}
        />
      );
      const { user, rerender } = render(element(), {
        preloadedState: { config: configState() },
      });
      await user.click(
        await screen.findByRole("button", { name: "Synchronize with target" }),
      );
      await waitFor(() => expect(statusCalls).toBe(2));
      rerender(
        transition === "worktree round trip"
          ? element("wt-2")
          : element("wt-1", false),
      );
      rerender(element());
      release();
      await waitFor(() =>
        expect(
          screen.getByRole("button", { name: "Refresh status" }),
        ).toBeEnabled(),
      );
      expect(sync.calls.some((call) => call.action === "start")).toBe(false);
    },
  );

  test("writer blockers stay separate from Git phase and refresh enables synchronization", async () => {
    const sync = installSyncServer(
      syncResponse({
        writer_readiness: {
          ready: false,
          blockers: [
            {
              kind: "execution_active",
              agent_id: "agent-writing",
              chat_id: "chat-owning",
              run_generation: 3,
              recovery_actions: [
                "inspect_agent",
                "wait",
                "cancel_agent",
                "retry_status",
              ],
            },
          ],
          truncated: false,
        },
      }),
      (request) =>
        request.action === "start"
          ? syncResponse({ status: "ready", operation: operation() })
          : undefined,
    );
    const { user } = renderModal();
    expect(await screen.findByText("Waiting for writers")).toBeVisible();
    expect(screen.getByText(/Agent agent-writing/)).toHaveTextContent(
      "Chat chat-owning · Run 3",
    );
    expect(
      screen.getByText(/Ask the owning chat to inspect this agent/),
    ).toBeVisible();
    const start = screen.getByRole("button", {
      name: "Synchronize with target",
    });
    expect(start).toBeDisabled();
    expect(screen.getByRole("button", { name: "Merge" })).toBeEnabled();
    expect(screen.getByLabelText("Target branch")).toBeEnabled();
    expect(screen.getAllByText("No synchronization").length).toBeGreaterThan(0);
    expect(sync.calls.every((call) => call.action === "status")).toBe(true);

    sync.state.current = syncResponse({
      writer_readiness: { ready: true, blockers: [], truncated: false },
    });
    await user.click(screen.getByRole("button", { name: "Refresh status" }));
    expect(await screen.findByText("Writers ready")).toBeVisible();
    expect(screen.queryByText(/Agent agent-writing/)).not.toBeInTheDocument();
    await waitFor(() => expect(start).toBeEnabled());
    await user.click(start);
    await waitFor(() =>
      expect(sync.calls.some((call) => call.action === "start")).toBe(true),
    );
  });

  test("a writer appearing in the fresh pre-start status prevents mutation", async () => {
    const sync = installSyncServer(syncResponse());
    const { user } = renderModal();
    const start = await screen.findByRole("button", {
      name: "Synchronize with target",
    });
    sync.state.current = syncResponse({
      writer_readiness: {
        ready: false,
        blockers: [
          {
            kind: "chat_active",
            chat_id: "new-writer",
            recovery_actions: ["stop_chat", "retry_status"],
          },
        ],
        truncated: false,
      },
    });
    await user.click(start);
    expect(await screen.findByText("Waiting for writers")).toBeVisible();
    expect(sync.calls.some((call) => call.action === "start")).toBe(false);
    expect(screen.queryByText("Synchronized")).not.toBeInTheDocument();
  });

  test("pins fresh status for the selected target branch", async () => {
    const sync = installSyncServer(syncResponse(), (request) => {
      if (request.action === "status")
        return syncResponse({
          target_branch: request.target_branch ?? "main",
          target_head:
            request.target_branch === "release" ? "release-head" : "main-head",
        });
      if (request.action === "start")
        return syncResponse({
          status: "ready",
          operation: operation({
            target_branch: request.target_branch,
            target_head: request.expected_target_head,
          }),
        });
      return undefined;
    });
    const { user } = renderModal();
    await screen.findByRole("button", { name: "Synchronize with target" });
    const target = screen.getByLabelText("Target branch");
    await user.clear(target);
    await user.type(target, "release");
    await waitFor(() =>
      expect(sync.calls).toContainEqual({
        action: "status",
        target_branch: "release",
      }),
    );
    await user.click(
      await screen.findByRole("button", { name: "Synchronize with target" }),
    );
    await waitFor(() =>
      expect(sync.calls).toContainEqual({
        action: "start",
        target_branch: "release",
        expected_source_head: "src-head",
        expected_target_head: "release-head",
        checkpoint: false,
      }),
    );
    expect(target).toBeEnabled();
  });
  test.each([false, true])(
    "hands conflicts to chat without native mutation (dirty=%s)",
    async (dirty) => {
      const sync = installSyncServer(syncResponse(), (request) =>
        request.action === "start"
          ? (HttpResponse.json(
              { error: "start must not run" },
              { status: 409 },
            ) as unknown as Response)
          : undefined,
      );
      const askRefact = vi.fn();
      const { user } = renderModal(
        { onAskRefact: askRefact },
        makeRecord(dirty),
      );
      await openConflict(user);
      const target = screen.getByLabelText("Target branch");
      await user.clear(target);
      await user.type(target, "release");
      await user.click(
        screen.getByRole("button", { name: "Ask Refact to resolve conflicts" }),
      );
      await waitFor(() => expect(askRefact).toHaveBeenCalledOnce());
      expect(sync.calls.filter((call) => call.action !== "status")).toEqual([]);
      expect(askRefact.mock.calls[0][0]).toEqual(["src/conflict.ts"]);
      expect(askRefact.mock.calls[0][1]).toMatchObject({
        target_branch: "release",
      });
      expect(
        (askRefact.mock.calls[0][2] as { sync?: WorktreeSyncResponse }).sync
          ?.operation,
      ).toBeNull();
    },
  );

  test("stale active journal does not disable ordinary merge or target selection", async () => {
    const sync = installSyncServer(
      syncResponse({
        status: "conflicts",
        operation: operation(),
        conflicts: [],
      }),
    );
    const merges: unknown[] = [];
    server.use(
      http.post("*/v1/worktrees/:id/merge", async ({ request }) => {
        merges.push(await request.json());
        return HttpResponse.json(conflictMerge);
      }),
    );
    const askRefact = vi.fn();
    const { user } = renderModal({ onAskRefact: askRefact });
    await screen.findByText(/Operation op-1/);
    const target = screen.getByLabelText("Target branch");
    expect(target).toBeEnabled();
    await user.clear(target);
    await user.type(target, "release");
    expect(screen.getByRole("button", { name: "Merge" })).toBeEnabled();
    await user.click(screen.getByRole("button", { name: "Merge" }));
    await waitFor(() => expect(merges).toHaveLength(1));
    expect(merges[0]).toMatchObject({ target_branch: "release" });
    await user.click(
      screen.getByRole("button", { name: "Ask Refact to resolve conflicts" }),
    );
    await waitFor(() => expect(askRefact).toHaveBeenCalledOnce());
    expect(askRefact.mock.calls[0][1]).toMatchObject({
      target_branch: "release",
    });
    expect(
      (askRefact.mock.calls[0][2] as { sync: WorktreeSyncResponse }).sync
        .operation,
    ).toMatchObject({
      target_branch: "main",
    });

    expect(sync.calls.filter((call) => call.action !== "status")).toEqual([]);
  });

  test("keeps native controls without a model sync tool", async () => {
    installSyncServer(syncResponse(), (request) =>
      request.action === "start"
        ? syncResponse({
            status: "conflicts",
            operation: operation(),
            conflicts: [{ path: "src/conflict.ts", kind: "unmerged" }],
          })
        : undefined,
    );
    const askRefact = vi.fn();
    const { user } = renderModal({ onAskRefact: askRefact });
    await openConflict(user);

    await user.click(
      screen.getByRole("button", { name: "Synchronize with target" }),
    );

    expect(askRefact).not.toHaveBeenCalled();
    expect(
      await screen.findByRole("button", { name: "Apply resolutions" }),
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Abort…" })).toBeInTheDocument();
  });

  test("resumes an active operation without repeating start", async () => {
    const active = syncResponse({
      status: "conflicts",
      operation: operation({ operation_id: "op-active", revision: 9 }),
      conflicts: [{ path: "src/conflict.ts", kind: "unmerged" }],
    });
    const sync = installSyncServer(active);
    const askRefact = vi.fn();
    const { user } = renderModal({ onAskRefact: askRefact });

    await user.click(
      await screen.findByRole("button", { name: "Ask Refact to help" }),
    );

    await waitFor(() => expect(askRefact).toHaveBeenCalledOnce());
    expect(sync.calls.some((call) => call.action === "start")).toBe(false);
    const [, , handoff] = askRefact.mock.calls[0] as unknown as [
      string[],
      MergeWorktreeResponse,
      { sync: WorktreeSyncResponse },
    ];
    expect(handoff.sync.operation).toMatchObject({
      operation_id: "op-active",
      revision: 9,
    });
  });

  test("does not hand off to chat or claim success when start fails", async () => {
    const sync = installSyncServer(syncResponse(), (request) =>
      request.action === "start"
        ? (HttpResponse.json(
            { error: "Sync source or target HEAD changed" },
            { status: 409 },
          ) as unknown as Response)
        : undefined,
    );
    const askRefact = vi.fn();
    const { user } = renderModal({ onAskRefact: askRefact });
    await openConflict(user);

    await user.click(
      screen.getByRole("button", { name: "Synchronize with target" }),
    );

    expect(
      await screen.findByText("Sync source or target HEAD changed"),
    ).toBeInTheDocument();
    expect(askRefact).not.toHaveBeenCalled();
    expect(sync.calls.some((call) => call.action === "start")).toBe(true);
    expect(
      screen.queryByText(/Conflict resolution request sent/),
    ).not.toBeInTheDocument();
  });

  test("requires explicit checkpoint consent for a dirty source", async () => {
    const sync = installSyncServer(syncResponse(), (request) =>
      request.action === "start"
        ? syncResponse({ status: "ready", operation: operation() })
        : undefined,
    );
    const { user } = renderModal({}, makeRecord(true));

    const startButton = await screen.findByRole("button", {
      name: "Synchronize with target",
    });
    expect(startButton).toBeDisabled();
    await user.click(
      screen.getByRole("checkbox", {
        name: "Confirm checkpointing uncommitted source changes",
      }),
    );
    await user.click(startButton);

    await waitFor(() =>
      expect(sync.calls.find((call) => call.action === "start")).toMatchObject({
        checkpoint: true,
      }),
    );
  });

  test("continues a ready operation with the current id and revision", async () => {
    const sync = installSyncServer(
      syncResponse({
        status: "ready",
        operation: operation({ operation_id: "op-ready", revision: 7 }),
      }),
      (request) =>
        request.action === "continue"
          ? syncResponse({
              status: "completed",
              operation: operation({
                operation_id: "op-ready",
                revision: 8,
                phase: "completed",
              }),
            })
          : undefined,
    );
    const { user } = renderModal();

    await user.click(
      await screen.findByRole("button", { name: "Continue synchronization" }),
    );

    await waitFor(() =>
      expect(sync.calls.find((call) => call.action === "continue")).toEqual({
        action: "continue",
        operation_id: "op-ready",
        expected_revision: 7,
      }),
    );
    expect((await screen.findAllByText("Synchronized")).length).toBeGreaterThan(
      0,
    );
    expect(
      screen.queryByRole("button", { name: "Continue synchronization" }),
    ).not.toBeInTheDocument();
  });

  test("safe abort needs confirmation and forced abort needs a second acknowledgement", async () => {
    const sync = installSyncServer(
      syncResponse({
        status: "ready",
        operation: operation({ operation_id: "op-abort", revision: 2 }),
      }),
      (request) =>
        request.action === "abort"
          ? syncResponse({
              status: "aborted",
              operation: operation({
                operation_id: "op-abort",
                revision: 3,
                phase: "aborted",
                recovery_path: "/cache/sync/wt-1-recovery.json",
              }),
            })
          : undefined,
    );
    const { user } = renderModal();

    await user.click(await screen.findByRole("button", { name: "Abort…" }));
    expect(sync.calls.some((call) => call.action === "abort")).toBe(false);
    const force = screen.getByRole("button", {
      name: "Force abort with backup",
    });
    expect(force).toBeDisabled();
    await user.click(
      screen.getByRole("checkbox", {
        name: "Acknowledge forced abort resets the source after backup",
      }),
    );
    await user.click(force);

    await waitFor(() =>
      expect(sync.calls.find((call) => call.action === "abort")).toEqual({
        action: "abort",
        operation_id: "op-abort",
        expected_revision: 2,
        force: true,
      }),
    );
    expect(
      await screen.findByText(
        "Recovery backup: /cache/sync/wt-1-recovery.json",
      ),
    ).toBeInTheDocument();
  });

  test("restores a forced-abort backup after confirmation using the current revision", async () => {
    const aborted = syncResponse({
      status: "aborted",
      operation: operation({
        operation_id: "op-aborted",
        revision: 5,
        phase: "aborted",
        recovery_path: "/cache/sync/backup.json",
      }),
    });
    const sync = installSyncServer(aborted, (request) =>
      request.action === "restore"
        ? syncResponse({
            status: "conflicts",
            operation: operation({ operation_id: "op-aborted", revision: 6 }),
          })
        : undefined,
    );
    const { user } = renderModal();

    await user.click(
      await screen.findByRole("button", { name: "Restore backup…" }),
    );
    expect(sync.calls.some((call) => call.action === "restore")).toBe(false);
    await user.click(screen.getByRole("button", { name: "Restore backup" }));

    await waitFor(() =>
      expect(sync.calls.find((call) => call.action === "restore")).toEqual({
        action: "restore",
        operation_id: "op-aborted",
        expected_revision: 5,
      }),
    );
  });

  test("selects a later recovery archive and confirms its path before restoring", async () => {
    const aborted = syncResponse({
      status: "aborted",
      recovery_archives: [
        { archive_id: "primary", path: "/cache/sync/backup.json" },
        { archive_id: "later", path: "/cache/sync/later.json" },
      ],
      operation: operation({
        operation_id: "op-aborted",
        revision: 5,
        phase: "aborted",
        recovery_path: "/cache/sync/backup.json",
      }),
    });
    const sync = installSyncServer(aborted, (request) =>
      request.action === "restore"
        ? syncResponse({
            status: "conflicts",
            operation: operation({ operation_id: "op-aborted", revision: 6 }),
          })
        : undefined,
    );
    const { user } = renderModal();

    await user.selectOptions(
      await screen.findByRole("combobox", { name: "Recovery backup" }),
      "later",
    );
    await user.click(
      await screen.findByRole("button", { name: "Restore backup…" }),
    );
    expect(screen.getByText(/Backup:.*later.json/)).toBeInTheDocument();
    expect(sync.calls.some((call) => call.action === "restore")).toBe(false);
    await user.click(screen.getByRole("button", { name: "Restore backup" }));

    await waitFor(() =>
      expect(sync.calls.find((call) => call.action === "restore")).toEqual({
        action: "restore",
        archive_id: "later",
        operation_id: "op-aborted",
        expected_revision: 5,
      }),
    );
  });

  test("target advanced on a finished operation offers a fresh start with new pins", async () => {
    const sync = installSyncServer(
      syncResponse({
        status: "target_advanced",
        target_head: "tgt-new",
        operation: operation({ phase: "completed", target_head: "tgt-old" }),
      }),
      (request) =>
        request.action === "start"
          ? syncResponse({ status: "ready", operation: operation() })
          : undefined,
    );
    const { user } = renderModal();

    expect(
      await screen.findByText(/optionally synchronize again/),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Abort…" }),
    ).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Synchronize again" }));

    await waitFor(() =>
      expect(sync.calls.find((call) => call.action === "start")).toMatchObject({
        expected_target_head: "tgt-new",
      }),
    );
  });

  test("ignores a pending start that finishes after the worktree changed", async () => {
    let releaseStart: () => void = () => undefined;
    const startGate = new Promise<void>((resolve) => {
      releaseStart = resolve;
    });
    server.use(
      http.post("*/v1/worktrees/:id/sync", async ({ request, params }) => {
        const body = (await request.json()) as WorktreeSyncRequest;
        if (body.action === "start") await startGate;
        return HttpResponse.json(
          syncResponse({
            id: String(params.id),
            status: body.action === "start" ? "ready" : "idle",
            operation:
              body.action === "start"
                ? operation({ operation_id: `op-for-${String(params.id)}` })
                : null,
          }),
        );
      }),
    );
    const askRefact = vi.fn();
    const first = makeRecord();
    const second = { ...makeRecord(), meta: { ...makeMeta(), id: "wt-2" } };
    const element = (record: WorktreeRecordView) => (
      <MergeWorktreeModal
        open
        worktreeId={record.meta.id}
        record={record}
        onOpenChange={() => undefined}
        onAskRefact={askRefact}
      />
    );
    const { user, rerender } = render(element(first), {
      preloadedState: { config: configState() },
    });
    await openConflict(user);
    await user.click(
      screen.getByRole("button", { name: "Synchronize with target" }),
    );

    rerender(element(second));
    releaseStart();

    await waitFor(() =>
      expect(screen.queryByText(/op-for-wt-1/)).not.toBeInTheDocument(),
    );
    expect(askRefact).not.toHaveBeenCalled();
  });

  test("a failed status query shows the error instead of an invented idle state", async () => {
    installSyncServer(
      syncResponse(),
      () =>
        HttpResponse.json(
          { error: "Sync journal unreadable" },
          { status: 500 },
        ) as unknown as Response,
    );

    renderModal();

    expect(await screen.findByText("Sync journal unreadable")).toBeVisible();
    expect(
      screen.queryByRole("button", { name: "Synchronize with target" }),
    ).not.toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Refresh status" }),
    ).toBeEnabled();
  });

  test("terminal archived metadata does not offer abort", async () => {
    installSyncServer(
      syncResponse({
        status: "aborted",
        operation: operation({ phase: "metadata_archived" }),
      }),
    );
    renderModal();
    await screen.findByText("metadata archived", { exact: false });
    expect(
      screen.queryByRole("button", { name: "Abort…" }),
    ).not.toBeInTheDocument();
  });

  test("healthy conflicts with a receipt do not offer journal recovery", async () => {
    installSyncServer(
      syncResponse({
        status: "conflicts",
        operation: operation(),
        recovery_journal_digest: "digest",
      }),
    );
    renderModal();
    await screen.findByRole("button", { name: "Abort…" });
    expect(
      screen.queryByRole("button", { name: "Recover journal…" }),
    ).not.toBeInTheDocument();
  });

  test("offers a confirmed metadata-only journal recovery from the status receipt", async () => {
    const sync = installSyncServer(
      syncResponse({
        status: "recovery_required",
        recovery_journal_digest: "digest-1",
        warnings: ["Synchronization journal is unreadable"],
      }),
      (request) => (request.action === "recover" ? syncResponse() : undefined),
    );
    const { user } = renderModal();

    await user.click(
      await screen.findByRole("button", { name: "Recover journal…" }),
    );
    expect(sync.calls.some((call) => call.action === "recover")).toBe(false);
    await user.click(
      screen.getByRole("button", { name: "Archive journal and recover" }),
    );

    await waitFor(() =>
      expect(sync.calls.find((call) => call.action === "recover")).toEqual({
        action: "recover",
        expected_journal_digest: "digest-1",
      }),
    );
  });

  test("reload restores the active operation from the server status", async () => {
    installSyncServer(
      syncResponse({
        status: "conflicts",
        operation: operation({ operation_id: "op-durable", revision: 12 }),
        conflicts: [{ path: "src/a.ts", kind: "unmerged" }],
      }),
    );

    renderModal();

    expect(await screen.findByText("src/a.ts")).toBeInTheDocument();
    expect(
      screen.getByText(/Operation op-durable · revision 12/),
    ).toBeInTheDocument();
  });
});
