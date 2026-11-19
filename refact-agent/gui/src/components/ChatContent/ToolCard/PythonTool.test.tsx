vi.hoisted(() => vi.resetModules());
import { act } from "react-dom/test-utils";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "../../../utils/test-utils";
import { ToolJobResultsContext } from "./ToolJobResultsContext";
import { ToolJobResult } from "./ToolJobResult";
import { ToolContent } from "../ToolsContent";
import { CollapsibleStoreProvider } from "../useStoredOpen";
import {
  ChatCommandHttpError,
  sendPythonRunControl,
} from "../../../services/refact/chatCommands";
import { pythonApproval } from "../../../__tests__/pythonFixtures";
import {
  BACKGROUND_NOTE,
  PythonTool,
  STOP_RETRY_MS,
  WAITING_NOTE,
} from "./PythonTool";
import type {
  PythonCallRecord,
  PythonRunExtra,
  ToolCall,
} from "../../../services/refact/types";
import { pythonCall } from "../../../__tests__/pythonFixtures";
import {
  ENGINE_REFUSAL_MESSAGE,
  engineCall,
  engineChange,
  unknownOutcomeChange,
} from "./engineFixtures";
import { READ_ONLY_NOTE } from "./ReadOnlyChip";

vi.mock("./useToolCallActivity", async (original) => ({
  ...(await original<typeof import("./useToolCallActivity")>()),
  useToolCallActivity: () => ({ threadId: "chat", isRunning: false }),
}));
vi.mock("../../../hooks/useOpenFileInApp", () => ({
  useOpenFileInApp: () => ({ canOpen: true, openFile: openFile }),
}));
const live = vi.hoisted(() => ({
  status: "running" as "running" | "background" | "finished" | undefined,
  approvals: false,
  calls: [] as PythonCallRecord[],
  startedAt: 1,
  run: undefined as unknown,
  runKey: "",
  runCalls: undefined as unknown,
  approvalList: [] as unknown[],
  noApprovals: [] as unknown[],
  stdout: "live print",
  displays: undefined as unknown[] | undefined,
}));
const rowRenders = vi.hoisted(() => new Map<number, number>());
vi.mock("./pythonDisplay", async (original) => {
  const actual = await original<typeof import("./pythonDisplay")>();
  return {
    ...actual,
    cutCodeLine: (text: string) => {
      const number = Number(
        /^[a-z](\d+)/.exec(text)?.[1] ?? text.charCodeAt(0),
      );
      rowRenders.set(number, (rowRenders.get(number) ?? 0) + 1);
      return actual.cutCodeLine(text);
    },
  };
});
vi.mock("../../../features/Chat/Thread/selectors", async (original) => ({
  ...(await original<
    typeof import("../../../features/Chat/Thread/selectors")
  >()),
  selectPythonRun: () => {
    if (!live.status) return undefined;
    const key = `${live.status}:${live.startedAt}`;
    if (
      live.runKey !== key ||
      live.runCalls !== live.calls ||
      (live.run as { displays?: unknown } | undefined)?.displays !==
        live.displays ||
      (live.run as { stdout?: string } | undefined)?.stdout !== live.stdout
    ) {
      live.runKey = key;
      live.runCalls = live.calls;
      live.run = {
        run_id: "r",
        status: live.status,
        background: live.status === "background",
        started_at_ms: live.startedAt,
        calls: live.calls,
        stdout: live.stdout,
        ...(live.displays ? { displays: live.displays } : {}),
      };
    }
    return live.run;
  },
  selectPythonApprovals: () =>
    live.approvals ? live.approvalList : live.noApprovals,
}));
vi.mock("../../../services/refact/chatCommands", async (original) => ({
  ...(await original<typeof import("../../../services/refact/chatCommands")>()),
  sendPythonRunControl: vi.fn().mockResolvedValue(undefined),
}));
const openFile = vi.fn();
const call: ToolCall = {
  id: "t",
  index: 0,
  type: "function",
  function: { name: "python", arguments: '{"code":"print(42)"}' },
};
const extra: PythonRunExtra = {
  schema: 1,
  run_id: "r",
  status: "completed",
  background: false,
  started_at_ms: 1,
  finished_at_ms: 1001,
  workspace_dir: "",
  run_dir: "",
  calls: [{ ...pythonCall, status: "ok" }],
  calls_total: 1,
  calls_truncated: false,
  stdout_preview: "printed",
  stdout_bytes: 7,
  result_preview: "answer",
  result_bytes: 6,
  error: null,
  files_read: [],
  files_written: [{ path: "/file", bytes: 5, diffs: [] }],
  artifacts: [{ path: "/artifact", bytes: 9 }],
  state: { restored: false, reset_reason: null, persisted: true, bytes: 0 },
};

describe("PythonTool", () => {
  beforeEach(() => {
    live.status = "running";
    live.approvals = false;
    live.calls = [];
    live.startedAt = 1;
    live.approvalList = [pythonApproval];
  });
  afterEach(() => vi.useRealTimers());
  const done = (python: Partial<PythonRunExtra>, content = "") => ({
    tool_call_id: "t",
    content,
    extra: { python: { ...extra, ...python } },
  });
  const card = () => screen.getByTestId("script-card");
  const toggle = () => within(card()).getAllByTestId("transcript-toggle")[0];
  const openCard = () => fireEvent.click(toggle());
  const withCode = (code: string, description?: string): ToolCall => ({
    ...call,
    function: {
      ...call.function,
      arguments: JSON.stringify(description ? { code, description } : { code }),
    },
  });

  it("is a Script row titled by the description and never says Python", () => {
    render(
      <PythonTool
        toolCall={withCode("print(42)", "Count the answer")}
        result={done({})}
      />,
    );
    expect(screen.getByTestId("transcript-verb")).toHaveTextContent("Script");
    expect(screen.getByTestId("transcript-target")).toHaveTextContent(
      "Count the answer",
    );
    expect(card()).toHaveAttribute("data-python-run-id", "r");
    expect(card()).toHaveAttribute("aria-label", "Script: Count the answer");
    expect(card().textContent).not.toMatch(/python/i);
    expect(screen.getByTestId("transcript-meta")).toHaveTextContent(
      "1 call · 1 file · 1.0 s",
    );
    expect(screen.queryByTestId("transcript-status-pill")).toBeNull();
  });
  it("renders generic failures and old results without valid metadata", () => {
    live.status = undefined;
    render(
      <PythonTool
        toolCall={call}
        result={{
          tool_call_id: "t",
          content: "python tool is disabled",
          tool_failed: true,
        }}
      />,
    );
    openCard();
    expect(screen.getByText("python tool is disabled")).toBeInTheDocument();
    expect(screen.getByText("Failed")).toBeInTheDocument();
  });
  it("shows the shared unrecorded state when no result or live run exists", () => {
    live.status = undefined;
    render(<PythonTool toolCall={call} />);
    expect(screen.getByTestId("transcript-status-pill")).toHaveTextContent(
      "No result",
    );
  });
  it("shows a pending background receipt as Background with the note", () => {
    live.status = undefined;
    render(
      <PythonTool
        toolCall={call}
        result={{
          tool_call_id: "t",
          content: "receipt",
          extra: {
            tool_job: {
              job_id: "j",
              chat_id: "chat",
              tool_call_id: "t",
              tool_name: "python",
              model: "m",
              turn_id: "turn",
              status: "accepted",
              result_url: "/result",
              cancel_url: "/cancel",
            },
          },
        }}
      />,
    );
    openCard();
    expect(screen.getByText("Background")).toBeInTheDocument();
    expect(screen.getByText(BACKGROUND_NOTE)).toBeInTheDocument();
    expect(screen.queryByText("receipt")).toBeNull();
    expect(screen.queryByRole("button", { name: "Stop" })).toBeNull();
  });
  it("keeps Stop disabled after sending until live status changes", async () => {
    render(<PythonTool toolCall={call} />);
    fireEvent.click(screen.getByRole("button", { name: "Stop" }));
    await act(async () => {
      await Promise.resolve();
    });
    expect(screen.getByRole("button", { name: "Stopping…" })).toBeDisabled();
  });
  it("explains a finished-run Stop rejection without raw HTTP errors", async () => {
    vi.mocked(sendPythonRunControl).mockRejectedValueOnce(
      new ChatCommandHttpError(400, "raw 400"),
    );
    render(<PythonTool toolCall={call} />);
    fireEvent.click(screen.getByRole("button", { name: "Stop" }));
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "This script is no longer running.",
    );
  });
  it("suppresses duplicate deliveries for normalized tool names", () => {
    live.status = undefined;
    const aliased = {
      ...call,
      function: { ...call.function, name: "t_python" },
    };
    render(
      <ToolContent
        toolCalls={[aliased]}
        jobResultsByToolId={{
          t: [
            {
              toolName: "t_python",
              toolCall: aliased,
              message: {
                role: "tool",
                tool_call_id: "t",
                content: "",
                extra: { python: extra },
              },
            },
          ],
        }}
      />,
    );
    expect(screen.getAllByTestId("script-card")).toHaveLength(1);
  });
  it.each(["running", "background"] as const)(
    "shows %s and sends stop",
    async (status) => {
      live.status = status;
      render(<PythonTool toolCall={call} />);
      expect(
        screen.getByText(status === "running" ? "Running" : "Background"),
      ).toBeInTheDocument();
      openCard();
      if (status === "background")
        expect(screen.getByText(BACKGROUND_NOTE)).toBeInTheDocument();
      fireEvent.click(screen.getByRole("button", { name: "Stop" }));
      await waitFor(() =>
        expect(sendPythonRunControl).toHaveBeenCalledWith(
          "chat",
          "t",
          "stop",
          expect.anything(),
          undefined,
        ),
      );
    },
  );
  it("uses the final delivery rather than the background receipt", () => {
    render(
      <ToolJobResultsContext.Provider
        value={{
          t: [
            {
              toolName: "python",
              toolCall: call,
              message: {
                role: "tool",
                tool_call_id: "t",
                content: "",
                extra: { python: { ...extra, background: true } },
              },
            },
          ],
        }}
      >
        <PythonTool toolCall={call} />
      </ToolJobResultsContext.Provider>,
    );
    openCard();
    expect(screen.getByText("answer")).toBeInTheDocument();
    expect(screen.getByTestId("transcript-meta")).toHaveTextContent(
      "ran in background",
    );
    expect(screen.queryByText("Background")).toBeNull();
  });
  it("renders a standalone native job delivery", () => {
    render(
      <ToolJobResult
        result={{
          toolName: "python",
          toolCall: call,
          message: {
            role: "tool",
            tool_call_id: "t",
            content: "",
            extra: { python: extra },
          },
        }}
      />,
    );
    openCard();
    expect(screen.getByText("answer")).toBeInTheDocument();
  });
  it("highlights known tool calls lexically for calls without a line", () => {
    const { container } = render(
      <PythonTool
        toolCall={withCode(
          "def f():\n    return shell(command='ls')\nawait f()",
        )}
        result={done({ files_written: [] })}
      />,
    );
    openCard();
    expect(container.querySelector("[class*=callee]")).toHaveTextContent(
      "shell",
    );
    expect(
      screen.getByRole("region", { name: "Tool calls" }),
    ).toHaveTextContent("Tool calls · 1");
  });
  it("bounds a 1 MB single-line output and notes the truncation", () => {
    const line = "x".repeat(1_000_000);
    render(
      <PythonTool
        toolCall={call}
        result={done({
          result_preview: "",
          stdout_preview: line,
          stdout_bytes: 2_000_001,
        })}
      />,
    );
    openCard();
    const preview = screen.getByTestId("script-preview");
    expect(preview.textContent?.length).toBeLessThanOrEqual(20_000);
    expect(preview.className).toMatch(/previewCompact/);
    expect(
      screen.getByText("Preview truncated · full output 1.9 MB"),
    ).toBeInTheDocument();
  });
  it("highlights small JSON results and keeps __init__.py intact", async () => {
    const view = render(
      <PythonTool
        toolCall={call}
        result={done({ result_preview: '{"total": 3}' })}
      />,
    );
    openCard();
    await waitFor(() =>
      expect(screen.getByTestId("script-preview")).toHaveTextContent(
        '"total": 3',
      ),
    );
    view.unmount();
    render(
      <PythonTool
        toolCall={call}
        result={done({ result_preview: '"src/models/__init__.py"' })}
      />,
    );
    openCard();
    expect(screen.getByText('"src/models/__init__.py"')).toBeInTheDocument();
  });
  it("shows a result without valid metadata as plain text", () => {
    live.status = undefined;
    const { container } = render(
      <PythonTool
        toolCall={call}
        result={{ tool_call_id: "t", content: "**bold** __init__.py" }}
      />,
    );
    openCard();
    expect(screen.getByTestId("script-preview")).toHaveTextContent(
      "**bold** __init__.py",
    );
    expect(container.querySelector("strong")).toBeNull();
  });
  it("keeps historical code_mode calls generic", () => {
    render(
      <ToolContent
        toolCalls={[
          { ...call, function: { ...call.function, name: "code_mode" } },
        ]}
      />,
    );
    expect(screen.queryByTestId("script-card")).toBeNull();
    expect(screen.getByTestId("transcript-toggle")).toBeInTheDocument();
  });
  it("lists changed files once and artifacts once, with capability-aware actions", () => {
    render(
      <PythonTool
        toolCall={call}
        result={done({
          workspace_dir: "/ws",
          files_written: [
            { path: "/file", bytes: 5, diffs: [] },
            { path: "/file", bytes: 6, diffs: [] },
            { path: "/ws/artifacts/smoke.json", bytes: 51, diffs: [] },
          ],
          artifacts: [{ path: "/ws/artifacts/smoke.json", bytes: 51 }],
        })}
      />,
    );
    openCard();
    expect(screen.getByRole("region", { name: "Changes" })).toHaveTextContent(
      "Changes · 1",
    );
    expect(screen.getByRole("region", { name: "Artifacts" })).toHaveTextContent(
      "smoke.json51 B",
    );
    fireEvent.click(screen.getByRole("button", { name: "Open file" }));
    fireEvent.click(screen.getByRole("button", { name: "Open smoke.json" }));
    expect(openFile).toHaveBeenCalledWith({ path: "/file" });
    expect(openFile).toHaveBeenCalledWith({ path: "/ws/artifacts/smoke.json" });
    expect(
      screen.getByRole("button", { name: "Copy path of smoke.json" }),
    ).toBeInTheDocument();
  });
  it("shows more than five changed files behind Show more", () => {
    const files = Array.from({ length: 7 }, (_, i) => ({
      path: `/p/f${i}.txt`,
      bytes: 1,
      diffs: [],
    }));
    render(
      <PythonTool
        toolCall={call}
        result={done({ files_written: files, artifacts: [] })}
      />,
    );
    openCard();
    expect(screen.getAllByRole("button", { name: /^Open f/ })).toHaveLength(5);
    fireEvent.click(screen.getByRole("button", { name: "Show 2 more files" }));
    expect(screen.getAllByRole("button", { name: /^Open f/ })).toHaveLength(7);
  });
  it("shows a user Stop as Stopped without an error", () => {
    render(
      <PythonTool
        toolCall={call}
        result={done(
          {
            status: "cancelled",
            result_preview: "",
            error: {
              type: "CancelledError",
              message: "Python run cancelled",
              traceback: "",
            },
            calls: [
              { ...pythonCall, status: "cancelled", approval: "cancelled" },
            ],
          },
          "Error: CancelledError: Python run cancelled",
        )}
      />,
    );
    openCard();
    expect(screen.getByTestId("transcript-status-pill")).toHaveTextContent(
      "Stopped",
    );
    expect(screen.queryByRole("alert")).toBeNull();
    expect(
      screen.getByText(
        "Stopped while waiting for approval. The pending call did not run.",
      ),
    ).toBeInTheDocument();
  });
  it.each([
    [
      "old engine",
      { restored: false, reset_reason: null, persisted: false, bytes: 0 },
      "Output limit exceeded: the session was restored to the state saved before this run; variables set in this run are lost.",
    ],
    [
      "engine reset reason",
      {
        restored: false,
        reset_reason:
          "run limit exceeded: the session was restored to the state saved before this run; variables set in this run are lost",
        persisted: false,
        bytes: 0,
      },
      "Output limit exceeded: the session was restored to the state saved before this run; variables set in this run are lost.",
    ],
    [
      "engine note",
      {
        restored: false,
        reset_reason: "run limit exceeded",
        persisted: false,
        bytes: 0,
        note: "Rolled back to the snapshot before this run",
      },
      "Output limit exceeded: rolled back to the snapshot before this run.",
    ],
  ])("explains a limit run in one sentence: %s", (_name, state, text) => {
    render(
      <PythonTool
        toolCall={call}
        result={done({
          status: "limit_exceeded",
          state,
          error: { type: "OutputLimit", message: "", traceback: "" },
        })}
      />,
    );
    openCard();
    expect(screen.getByText("Limit exceeded")).toBeInTheDocument();
    expect(screen.getByText(text)).toBeInTheDocument();
  });
  it("re-enables Stop if the run is still going 10 s after the request", async () => {
    vi.useFakeTimers();
    render(<PythonTool toolCall={call} />);
    fireEvent.click(screen.getByRole("button", { name: "Stop" }));
    await act(async () => {
      await Promise.resolve();
    });
    expect(screen.getByRole("button", { name: "Stopping…" })).toBeDisabled();
    act(() => {
      vi.advanceTimersByTime(STOP_RETRY_MS);
    });
    expect(screen.getByRole("button", { name: "Stop" })).toBeEnabled();
  });
  it("clears a Stop error and the button when the run ends", async () => {
    vi.mocked(sendPythonRunControl).mockRejectedValueOnce(
      new ChatCommandHttpError(400, "raw 400"),
    );
    const view = render(<PythonTool toolCall={call} />);
    fireEvent.click(screen.getByRole("button", { name: "Stop" }));
    expect(await screen.findByRole("alert")).toBeInTheDocument();
    live.status = "finished";
    view.rerender(<PythonTool toolCall={{ ...call }} />);
    expect(screen.queryByRole("alert")).toBeNull();
    expect(screen.queryByRole("button", { name: /Stop/ })).toBeNull();
  });
  it("keeps the card structured when metadata has values from a newer engine", () => {
    render(
      <PythonTool
        toolCall={call}
        result={done(
          {
            status: "paused_by_policy" as PythonRunExtra["status"],
            calls: [
              {
                ...pythonCall,
                status: "throttled" as PythonCallRecord["status"],
                approval: "delegated" as PythonCallRecord["approval"],
              },
            ],
          },
          "RAW TEXT",
        )}
      />,
    );
    openCard();
    expect(screen.getByTestId("transcript-status-pill")).toHaveTextContent(
      "Unknown status",
    );
    expect(screen.getByRole("region", { name: "Result" })).toBeInTheDocument();
    expect(screen.queryByText("RAW TEXT")).toBeNull();
    expect(screen.getByText("Approval unknown")).toBeInTheDocument();
  });
  it("formats long durations and hides a stale time for a finished run without metadata", () => {
    const view = render(
      <PythonTool
        toolCall={call}
        result={done({ started_at_ms: 0, finished_at_ms: 7_200_000 })}
      />,
    );
    expect(screen.getByText("2 h 0 min")).toBeInTheDocument();
    view.unmount();
    live.status = "finished";
    render(<PythonTool toolCall={call} />);
    expect(screen.queryByText(/ s$|min$/)).toBeNull();
  });
  it("prefers a valid delivered result over the direct one", () => {
    render(
      <ToolJobResultsContext.Provider
        value={{
          t: [
            {
              toolName: "python",
              toolCall: call,
              message: {
                role: "tool",
                tool_call_id: "t",
                content: "",
                extra: { python: { ...extra, result_preview: "delivered" } },
              },
            },
          ],
        }}
      >
        <PythonTool
          toolCall={call}
          result={done({ result_preview: "direct" })}
        />
      </ToolJobResultsContext.Provider>,
    );
    openCard();
    expect(screen.getByText("delivered")).toBeInTheDocument();
    expect(screen.queryByText("direct")).toBeNull();
  });

  describe("collapsed by default", () => {
    const hasBody = () =>
      within(card()).queryByTestId("script-code") !== null ||
      within(card()).queryByTestId("transcript-panel") !== null;

    it("shows only the header for a finished run", () => {
      render(
        <PythonTool
          toolCall={withCode("print(42)", "Count the answer")}
          result={done({})}
        />,
      );
      expect(screen.getByTestId("transcript-verb")).toHaveTextContent("Script");
      expect(screen.getByTestId("transcript-target")).toHaveTextContent(
        "Count the answer",
      );
      expect(screen.getByTestId("transcript-meta")).toHaveTextContent(
        "1 call · 1 file · 1.0 s",
      );
      expect(toggle()).toHaveAttribute("aria-expanded", "false");
      expect(screen.queryByTestId("script-code")).toBeNull();
      expect(screen.queryByRole("region", { name: "Result" })).toBeNull();
      expect(screen.queryByRole("button", { name: "Details" })).toBeNull();
      expect(screen.queryByTestId("transcript-panel")).toBeNull();
    });
    it("keeps the Failed status pill on a collapsed failed run", () => {
      render(
        <PythonTool
          toolCall={call}
          result={done({
            status: "failed",
            error: { type: "ValueError", message: "bad", traceback: "" },
          })}
        />,
      );
      expect(screen.getByTestId("transcript-status-pill")).toHaveTextContent(
        "Failed",
      );
      expect(toggle()).toHaveAttribute("aria-expanded", "false");
      expect(hasBody()).toBe(false);
      expect(screen.queryByText("ValueError")).toBeNull();
    });
    it("opens from the header, switches to details and collapses again", async () => {
      render(<PythonTool toolCall={call} result={done({})} />);
      openCard();
      expect(toggle()).toHaveAttribute("aria-expanded", "true");
      expect(screen.getByTestId("script-code")).toBeInTheDocument();
      const details = screen.getByRole("button", { name: "Details" });
      expect(details).toHaveAttribute("aria-expanded", "false");
      expect(screen.queryByRole("button", { name: "Copy code" })).toBeNull();
      fireEvent.click(details);
      expect(details).toHaveAttribute("aria-expanded", "true");
      expect(toggle()).toHaveAttribute("aria-expanded", "true");
      expect(screen.getByRole("button", { name: "Copy code" })).toBeVisible();
      expect(screen.getByTestId("script-code")).toBeInTheDocument();
      openCard();
      expect(toggle()).toHaveAttribute("aria-expanded", "false");
      await waitFor(() => expect(hasBody()).toBe(false));
      expect(screen.queryByRole("button", { name: "Details" })).toBeNull();
    });
    it("is collapsed while running and still offers Stop", async () => {
      vi.mocked(sendPythonRunControl).mockClear();
      render(<PythonTool toolCall={call} />);
      expect(toggle()).toHaveAttribute("aria-expanded", "false");
      expect(hasBody()).toBe(false);
      expect(screen.queryByRole("button", { name: "Details" })).toBeNull();
      fireEvent.click(screen.getByRole("button", { name: "Stop" }));
      await waitFor(() =>
        expect(sendPythonRunControl).toHaveBeenCalledWith(
          "chat",
          "t",
          "stop",
          expect.anything(),
          undefined,
        ),
      );
      expect(sendPythonRunControl).toHaveBeenCalledTimes(1);
      expect(toggle()).toHaveAttribute("aria-expanded", "false");
      expect(hasBody()).toBe(false);
    });
    it("opens itself and shows the alert when Stop fails", async () => {
      vi.mocked(sendPythonRunControl).mockRejectedValueOnce(new Error("boom"));
      render(<PythonTool toolCall={call} />);
      expect(toggle()).toHaveAttribute("aria-expanded", "false");
      expect(screen.queryByRole("alert")).toBeNull();
      fireEvent.click(screen.getByRole("button", { name: "Stop" }));
      expect(await screen.findByRole("alert")).toHaveTextContent(
        "Could not stop this script. Please try again.",
      );
      expect(toggle()).toHaveAttribute("aria-expanded", "true");
      expect(screen.getByRole("button", { name: "Details" })).toBeVisible();
    });
    it("remembers the open state per tool call, apart from the details flag", () => {
      const preferences = new Map<string, boolean>();
      const collapseStore = {
        get: (key: string) => preferences.get(key),
        set: (key: string, value: boolean) => {
          preferences.set(key, value);
        },
      };
      const view = () => (
        <CollapsibleStoreProvider value={collapseStore}>
          <PythonTool toolCall={call} result={done({})} />
        </CollapsibleStoreProvider>
      );
      const first = render(view());
      expect(toggle()).toHaveAttribute("aria-expanded", "false");
      expect(preferences.get("tc:t")).toBe(false);
      openCard();
      expect(preferences.get("tc:t")).toBe(true);
      expect(preferences.get("script:t")).toBe(false);
      first.unmount();

      const second = render(view());
      expect(toggle()).toHaveAttribute("aria-expanded", "true");
      expect(screen.getByTestId("script-code")).toBeInTheDocument();
      expect(screen.getByRole("button", { name: "Details" })).toHaveAttribute(
        "aria-expanded",
        "false",
      );
      fireEvent.click(screen.getByRole("button", { name: "Details" }));
      expect(preferences.get("script:t")).toBe(true);
      expect(preferences.get("tc:t")).toBe(true);
      second.unmount();

      render(view());
      expect(toggle()).toHaveAttribute("aria-expanded", "true");
      expect(screen.getByRole("button", { name: "Details" })).toHaveAttribute(
        "aria-expanded",
        "true",
      );
      openCard();
      expect(preferences.get("tc:t")).toBe(false);
      expect(preferences.get("script:t")).toBe(true);
    });
  });

  const line = (number: number) =>
    document.querySelector<HTMLElement>(`[data-line="${number}"]`);
  const lineEl = (number: number) => {
    const node = line(number);
    if (!node) throw new Error(`line ${number} is not rendered`);
    return node;
  };

  it("ticks elapsed time without re-rendering code rows", () => {
    vi.useFakeTimers();
    try {
      vi.setSystemTime(10_000);
      live.startedAt = 9_000;
      live.calls = [{ ...pythonCall, status: "running", line: 1 }];
      rowRenders.clear();
      render(
        <PythonTool
          toolCall={withCode("x1 = await shell(command='ls')\nx2 = 2")}
        />,
      );
      openCard();
      const before = new Map(rowRenders);
      const elapsed = screen.getByText(/ s$/);
      const first = elapsed.textContent;
      act(() => {
        vi.advanceTimersByTime(3000);
      });
      expect(elapsed.textContent).not.toBe(first);
      expect(rowRenders).toEqual(before);
    } finally {
      live.calls = [];
      vi.useRealTimers();
    }
  });
  it("re-renders only the line whose calls changed during a live update", () => {
    const code =
      "a1 = await shell(command='1')\na2 = await cat(paths='x')\na3 = 3";
    live.calls = [
      { ...pythonCall, seq: 1, call_id: "a", status: "ok", line: 1 },
      {
        ...pythonCall,
        seq: 2,
        call_id: "b",
        tool: "cat",
        status: "running",
        line: 2,
      },
    ];
    rowRenders.clear();
    const view = render(<PythonTool toolCall={withCode(code)} />);
    openCard();
    const before = new Map(rowRenders);
    live.calls = [
      live.calls[0],
      { ...live.calls[1], status: "ok", duration_ms: 10 },
    ];
    view.rerender(<PythonTool toolCall={withCode(code)} />);
    expect(rowRenders.get(1)).toBe(before.get(1));
    expect(rowRenders.get(3)).toBe(before.get(3));
    expect(rowRenders.get(2)).toBe((before.get(2) ?? 0) + 1);
    live.calls = [];
  });
  it("shows a line's call status, annotation and details with diffs", () => {
    const code = 'x = 1\nawait update_textdoc(path="/file")\nprint(x)';
    const edited = {
      calls: [
        {
          ...pythonCall,
          tool: "update_textdoc",
          status: "ok" as const,
          approval: "allowed" as const,
          duration_ms: 1500,
          line: 2,
          col: 6,
          diffs: [
            {
              file_name: "/file",
              file_action: "edit",
              line1: 1,
              line2: 1,
              lines_remove: "before\n",
              lines_add: "after\nmore\n",
            },
          ],
        },
      ],
    };
    render(<PythonTool toolCall={withCode(code)} result={done(edited)} />);
    openCard();
    const row = lineEl(2);

    expect(row).toHaveAccessibleName(/^await update_textdoc/);
    expect(row).toHaveAccessibleName(/Allowed once · \+2 −1 · 1\.5 s$/);
    expect(row).toHaveAccessibleDescription("Succeeded");
    expect(row).toHaveTextContent("Allowed once · +2 −1 · 1.5 s");
    expect(row.querySelector("[class*=callee]")).toHaveTextContent(
      "update_textdoc",
    );
    expect(screen.queryByRole("region", { name: "Tool calls" })).toBeNull();
    fireEvent.click(row);
    expect(row).toHaveAttribute("aria-expanded", "true");
    expect(screen.getAllByTestId("diff-content").length).toBeGreaterThan(1);
  });
  it("aggregates several calls of one line and lists them in the detail", () => {
    const calls = [1, 2, 3].map((seq) => ({
      ...pythonCall,
      seq,
      call_id: `c${seq}`,
      tool: "cat",
      status: seq === 2 ? ("failed" as const) : ("ok" as const),
      duration_ms: 100,
      started_at_ms: seq,
      line: 1,
    }));
    render(
      <PythonTool
        toolCall={withCode(
          "r = await asyncio.gather(*[cat(paths=p) for p in ps])",
        )}
        result={done({ calls, calls_total: 3, files_written: [] })}
      />,
    );
    openCard();
    const row = lineEl(1);
    expect(row).toHaveAccessibleDescription("Failed");
    expect(row).toHaveTextContent("×3");
    fireEvent.click(row);
    expect(
      within(screen.getByTestId("transcript-panel")).getAllByRole("button", {
        name: /cat/,
      }),
    ).toHaveLength(4);
  });
  it("marks a waiting call in the code, never cutting its annotation", () => {
    live.approvals = true;
    live.calls = [{ ...pythonCall, call_id: "c", status: "running", line: 1 }];
    render(<PythonTool toolCall={withCode("await shell(command='ls')")} />);
    openCard();
    expect(screen.queryByRole("button", { name: "Allow once" })).toBeNull();
    expect(screen.getByText("Waiting for approval")).toBeInTheDocument();
    expect(screen.getByText(WAITING_NOTE)).toHaveAttribute("role", "status");
    const row = line(1);
    expect(row).toHaveAccessibleDescription("Waiting for your approval");
    expect(row?.querySelector("[class*=lineNote]")).toHaveTextContent(
      "Waiting for your approval",
    );
  });
  it("marks the failing line and shows the error above the output, without announcing history", () => {
    const code = Array.from({ length: 20 }, (_, i) => `v${i} = ${i}`).join(
      "\n",
    );
    render(
      <PythonTool
        toolCall={withCode(code)}
        result={done({
          status: "failed",
          calls: [],
          calls_total: 0,
          files_written: [],
          error: {
            type: "KeyError",
            message: "x",
            traceback: "trace",
            line: 14,
          },
        })}
      />,
    );
    openCard();
    expect(screen.queryByRole("alert")).toBeNull();
    const error = screen.getByText("KeyError").closest("p");
    if (!error) throw new Error("no state line");
    expect(error).toHaveTextContent("KeyError: x");
    expect(line(14)).toHaveAttribute("data-error", "true");
    expect(line(14)).toHaveTextContent("Failed here");
    expect(
      error.compareDocumentPosition(
        screen.getByRole("region", { name: "Result" }),
      ),
    ).toBe(Node.DOCUMENT_POSITION_FOLLOWING);
    expect(screen.queryByText("trace")).toBeNull();
  });
  it("announces a failure that happens while the card is watched", () => {
    const view = render(<PythonTool toolCall={call} />);
    openCard();
    live.status = undefined;
    view.rerender(
      <PythonTool
        toolCall={call}
        result={done({
          status: "failed",
          error: { type: "ValueError", message: "bad", traceback: "" },
        })}
      />,
    );
    expect(screen.getByRole("alert")).toHaveTextContent("ValueError: bad");
  });
  it("keeps waiting, running and failed lines when anchors exceed the budget", () => {
    const code = Array.from(
      { length: 60 },
      (_, i) => `r${i} = await cat(paths="${i}")`,
    ).join("\n");
    const calls = Array.from({ length: 60 }, (_, i) => ({
      ...pythonCall,
      seq: i + 1,
      call_id: `c${i + 1}`,
      tool: "cat",
      status: (i === 50
        ? "failed"
        : i === 55 || i === 57
          ? "running"
          : "ok") as PythonCallRecord["status"],
      line: i + 1,
    }));
    live.calls = calls;
    live.approvals = true;
    live.approvalList = [{ ...pythonApproval, call_id: "c58" }];
    render(<PythonTool toolCall={withCode(code)} />);
    openCard();
    const shown = [...document.querySelectorAll("[data-line]")].map((node) =>
      node.getAttribute("data-line"),
    );

    expect(shown).toEqual(expect.arrayContaining(["51", "56", "58"]));
    expect(line(30)).toBeNull();
    live.calls = [];
  });
  it("folds long code, expands from a fold row and focuses the first revealed line", () => {
    const code = Array.from({ length: 24 }, (_, i) => `v${i} = ${i}`).join(
      "\n",
    );
    render(
      <PythonTool toolCall={withCode(code)} result={done({ calls: [] })} />,
    );
    openCard();
    expect(screen.getAllByTestId("script-code-fold")).toHaveLength(1);
    expect(document.querySelectorAll("[data-line]")).toHaveLength(8);
    fireEvent.click(screen.getByTestId("script-code-fold"));
    expect(document.querySelectorAll("[data-line]")).toHaveLength(24);
    expect(line(9)).toHaveFocus();
    const details = screen.getByRole("button", { name: "Details" });
    expect(details).toHaveAttribute("aria-expanded", "true");
    expect(details).toHaveAttribute("aria-controls");
    fireEvent.click(screen.getByRole("button", { name: "Show less" }));
    fireEvent.click(
      screen.getByRole("button", { name: "Show all · 24 lines" }),
    );
    expect(document.querySelectorAll("[data-line]")).toHaveLength(24);
  });
  it("shows short code whole and puts details behind the Details button", () => {
    render(<PythonTool toolCall={call} result={done({})} />);
    openCard();
    expect(line(1)).toHaveTextContent("print(42)");
    expect(screen.queryByRole("button", { name: /^Show all/ })).toBeNull();
    expect(screen.queryByRole("button", { name: "Copy code" })).toBeNull();
    const details = screen.getByRole("button", { name: "Details" });
    expect(details).toHaveAttribute("aria-expanded", "false");
    fireEvent.click(details);
    expect(details).toHaveAttribute("aria-expanded", "true");
    for (const name of ["Copy code", "Copy output", "Show workspace"])
      expect(screen.getByRole("button", { name })).toBeInTheDocument();
    expect(screen.getByTestId("transcript-facts")).toHaveTextContent("Saved");
    expect(screen.getByRole("button", { name: "Raw" })).toBeInTheDocument();
  });
  it("renders 5,000 lines in steps of 400 when expanded", () => {
    const code = Array.from({ length: 5000 }, (_, i) => `v${i} = ${i}`).join(
      "\n",
    );
    render(
      <PythonTool toolCall={withCode(code)} result={done({ calls: [] })} />,
    );
    openCard();
    fireEvent.click(screen.getByRole("button", { name: "Details" }));
    expect(document.querySelectorAll("[data-line]")).toHaveLength(400);
    fireEvent.click(
      screen.getByRole("button", { name: "Show 400 more lines of 4600" }),
    );
    expect(document.querySelectorAll("[data-line]")).toHaveLength(800);
  });
  it("cuts a 1 MB code line in the DOM, also when expanded", () => {
    const code = `x = "${"y".repeat(1_000_000)}"`;
    render(
      <PythonTool toolCall={withCode(code)} result={done({ calls: [] })} />,
    );
    openCard();
    fireEvent.click(screen.getByRole("button", { name: "Details" }));
    const row = lineEl(1);
    expect(row.textContent?.length).toBeLessThan(2_100);
    expect(row).toHaveTextContent(/… 998,?00[0-9] more characters/);
  });
  it("notes a cut call list even when every shown call is on a line", () => {
    render(
      <PythonTool
        toolCall={withCode("await shell(command='ls')")}
        result={done({
          calls: [{ ...pythonCall, status: "ok", line: 1 }],
          calls_total: 250,
          calls_truncated: true,
        })}
      />,
    );
    openCard();
    expect(screen.queryByRole("region", { name: "Tool calls" })).toBeNull();
    expect(screen.getByText("Showing 1 of 250 calls")).toBeInTheDocument();
  });
  it("says when the live output is only the last part", () => {
    live.stdout = "z".repeat(16_384);
    render(<PythonTool toolCall={call} />);
    openCard();
    expect(screen.getByText("showing the last 16 KB")).toBeInTheDocument();
    live.stdout = "live print";
  });
  it("keeps a hint for printed output hidden behind a result", () => {
    render(
      <PythonTool
        toolCall={call}
        result={done({ stdout_preview: "a\nb\n" })}
      />,
    );
    openCard();
    const hint = screen.getByRole("button", {
      name: "Printed output hidden · 2 lines",
    });
    fireEvent.click(hint);
    expect(screen.getByRole("region", { name: "Output" })).toBeInTheDocument();
  });
  it("marks code cut off by truncated arguments and disables copying it", () => {
    render(
      <PythonTool
        toolCall={{
          ...call,
          function: {
            ...call.function,
            arguments: '{"code":"print(1)\\nprint(2',
          },
        }}
        result={done({})}
      />,
    );
    openCard();
    expect(screen.getByText(/^Code truncated/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Details" }));
    expect(screen.getByRole("button", { name: "Copy code" })).toBeDisabled();
  });
  it("names same-basename files by their directory", () => {
    render(
      <PythonTool
        toolCall={call}
        result={done({
          files_written: [
            { path: "/p/a/x.txt", bytes: 1, diffs: [] },
            { path: "/p/b/x.txt", bytes: 1, diffs: [] },
          ],
          artifacts: [],
        })}
      />,
    );
    openCard();
    expect(
      screen.getByRole("button", { name: "Open /p/a/x.txt" }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Open /p/b/x.txt" }),
    ).toBeInTheDocument();
  });

  describe("model calls", () => {
    const llmCall = (seq: number, line: number, extra = {}) => ({
      ...pythonCall,
      seq,
      call_id: `m${seq}`,
      tool: "llm",
      args_preview: JSON.stringify({
        prompt: `Summarise file ${seq}`,
        model: null,
        schema: false,
      }),
      status: "ok" as const,
      summary: `answer ${seq}`,
      started_at_ms: 1000 * seq,
      duration_ms: 3000,
      model: "gpt-5-mini",
      tokens_in: 3000,
      tokens_out: 433,
      line,
      ...extra,
    });
    const toolCall = (seq: number, line: number) => ({
      ...pythonCall,
      seq,
      call_id: `t${seq}`,
      tool: "cat",
      status: "ok" as const,
      duration_ms: 200,
      line,
    });
    const code = [
      "files = await cat(paths='a')",
      "notes = [await llm(f) for f in files]",
      "show(notes)",
    ].join("\n");
    it("annotates a model-call line with count, tokens and duration, highlighting llm and show", () => {
      const calls = [
        toolCall(1, 1),
        ...[2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13].map((seq) =>
          llmCall(seq, 2),
        ),
      ];
      const { container } = render(
        <PythonTool
          toolCall={withCode(code)}
          result={done({
            calls,
            calls_total: 13,
            files_written: [],
            llm: {
              calls: 12,
              input_tokens: 36_000,
              output_tokens: 5_196,
              models: ["gpt-5-mini"],
            },
          })}
        />,
      );
      openCard();
      expect(lineEl(2)).toHaveTextContent("×12 · 41.2k tokens · 14.0 s");
      expect(lineEl(1)).not.toHaveTextContent("tokens");
      const callees = [...container.querySelectorAll("[class*=callee]")].map(
        (node) => node.textContent,
      );
      expect(callees).toEqual(["cat", "llm", "show"]);
      const meta = screen.getByTestId("transcript-meta");
      expect(meta.querySelector("[class*=srOnly]")).toHaveTextContent(
        "1 call · 12 model calls",
      );
      expect(meta.querySelector("[class*=metaNarrow]")).toHaveTextContent(
        "1 + 12 model",
      );
      fireEvent.click(lineEl(2));
      expect(
        screen.getAllByRole("button", { name: /^Succeeded llm gpt-5-mini/ }),
      ).toHaveLength(12);
      expect(screen.getAllByText("3.4k tokens").length).toBeGreaterThan(0);
    });
    it("tells tool and model calls on one line apart and shows per-call details", () => {
      const calls = [toolCall(1, 1), llmCall(2, 1)];
      render(
        <PythonTool
          toolCall={withCode("r = [await cat(paths='a'), await llm('x')]")}
          result={done({ calls, calls_total: 2, files_written: [] })}
        />,
      );
      openCard();
      const row = lineEl(1);
      expect(row).toHaveTextContent("×2 · 3.4k tokens");
      fireEvent.click(row);
      fireEvent.click(screen.getByRole("button", { name: /^Succeeded llm/ }));
      expect(screen.getByText("gpt-5-mini")).toBeInTheDocument();
      expect(screen.getByText("in 3k · out 433")).toBeInTheDocument();
      expect(screen.getByText("Summarise file 2")).toBeInTheDocument();
      expect(screen.getByText("answer 2")).toBeInTheDocument();
      expect(
        screen
          .getByTestId("transcript-meta")
          .querySelector("[class*=metaWide]"),
      ).toHaveTextContent("1 call · 1 model call");
    });
    it("shows a running model call as progress and a failed one with its error", () => {
      live.calls = [
        llmCall(1, 2, {
          status: "running",
          duration_ms: null,
          tokens_in: undefined,
          tokens_out: undefined,
        }),
      ];
      const view = render(<PythonTool toolCall={withCode(code)} />);
      openCard();
      expect(lineEl(2)).toHaveAccessibleDescription("Running");
      expect(lineEl(2)).not.toHaveTextContent("tokens");
      live.status = undefined;
      view.rerender(
        <PythonTool
          toolCall={withCode(code)}
          result={done({
            status: "failed",
            calls: [
              llmCall(1, 2, {
                status: "failed",
                summary: "RateLimitError: slow down",
              }),
            ],
            calls_total: 1,
            files_written: [],
          })}
        />,
      );
      expect(lineEl(2)).toHaveAccessibleDescription("Failed");
      fireEvent.click(lineEl(2));
      expect(screen.getByText("Error")).toBeInTheDocument();
      expect(screen.getByText("RateLimitError: slow down")).toBeInTheDocument();
    });
    it("puts one model totals line in the details", () => {
      render(
        <PythonTool
          toolCall={withCode(code)}
          result={done({
            calls: [toolCall(1, 1), llmCall(2, 2)],
            calls_total: 2,
            files_written: [],
            llm: {
              calls: 1,
              input_tokens: 3000,
              output_tokens: 433,
              models: ["gpt-5-mini", "claude-haiku"],
            },
          })}
        />,
      );
      openCard();
      fireEvent.click(screen.getByRole("button", { name: "Details" }));
      expect(
        screen.getByText("1 call · 3k in · 433 out · gpt-5-mini, claude-haiku"),
      ).toBeInTheDocument();
      expect(document.body.textContent).not.toMatch(/python/i);
    });
    it("labels unplaced model calls and summarises them by model and prompt", () => {
      render(
        <PythonTool
          toolCall={withCode(code)}
          result={done({
            calls: [llmCall(1, 99)],
            calls_total: 1,
            files_written: [],
          })}
        />,
      );
      openCard();
      const region = screen.getByRole("region", { name: "Model calls" });
      expect(region).toHaveTextContent("Model calls · 1");
      expect(region).toHaveTextContent("gpt-5-mini · Summarise file 1");
      expect(region).toHaveTextContent("3.4k tokens");
    });
    it("shows Allowed for run on call lines", () => {
      render(
        <PythonTool
          toolCall={withCode(code)}
          result={done({
            calls: [
              {
                ...toolCall(1, 1),
                approval: "allowed_for_run",
                duration_ms: 1500,
              },
            ],
            calls_total: 1,
            files_written: [],
          })}
        />,
      );
      openCard();
      expect(lineEl(1)).toHaveTextContent("Allowed for run · 1.5 s");
    });
  });

  describe("displays", () => {
    const display = (index: number) => ({
      index,
      kind: "table",
      title: `Table ${index}`,
      columns: ["a"],
      rows: [[index]],
      truncated: false,
      path: `/d/${index}.json`,
    });
    it("renders a live run's displays between the code and the output, then the stored ones", () => {
      live.displays = [display(1)];
      const view = render(<PythonTool toolCall={withCode("show(x)")} />);
      openCard();
      const shown = screen.getByTestId("script-displays");
      expect(shown).toHaveTextContent("Table 1");
      expect(
        screen.getByTestId("script-code").compareDocumentPosition(shown) &
          Node.DOCUMENT_POSITION_FOLLOWING,
      ).toBeTruthy();
      live.status = undefined;
      view.rerender(
        <PythonTool
          toolCall={withCode("show(x)")}
          result={done({ displays: [display(2), display(3)] })}
        />,
      );
      expect(screen.getAllByTestId("script-display")).toHaveLength(2);
      expect(screen.getByTestId("script-displays")).not.toHaveTextContent(
        "Table 1",
      );
      expect(
        screen
          .getByTestId("script-displays")
          .compareDocumentPosition(
            screen.getByRole("region", { name: "Result" }),
          ) & Node.DOCUMENT_POSITION_FOLLOWING,
      ).toBeTruthy();
      live.displays = undefined;
    });
    const inCard = () => within(card());
    const follows = (first: HTMLElement, second: HTMLElement) =>
      Boolean(
        first.compareDocumentPosition(second) &
          Node.DOCUMENT_POSITION_FOLLOWING,
      );
    const collapsedWithDisplays = () => {
      live.status = undefined;
      return render(
        <PythonTool
          toolCall={withCode("show(x)")}
          result={done({ displays: [display(2), display(3)] })}
        />,
      );
    };
    it.each(["live", "stored"] as const)(
      "keeps a collapsed card's %s displays under the header",
      (kind) => {
        if (kind === "live") {
          live.displays = [display(1)];
          render(<PythonTool toolCall={withCode("show(x)")} />);
        } else collapsedWithDisplays();
        try {
          expect(toggle()).toHaveAttribute("aria-expanded", "false");
          const shown = inCard().getByTestId("script-displays");
          expect(inCard().getAllByTestId("script-displays")).toHaveLength(1);
          expect(inCard().getByTestId("transcript-preview")).toContainElement(
            shown,
          );
          expect(shown).toHaveTextContent(
            kind === "live" ? "Table 1" : "Table 2",
          );
          expect(inCard().queryByTestId("script-code")).toBeNull();
          expect(inCard().queryByRole("region", { name: "Result" })).toBeNull();
          expect(inCard().queryByTestId("transcript-panel")).toBeNull();
        } finally {
          live.displays = undefined;
        }
      },
    );
    it("moves the displays into the body when the card opens", () => {
      collapsedWithDisplays();
      openCard();
      expect(inCard().getAllByTestId("script-displays")).toHaveLength(1);
      expect(inCard().queryByTestId("transcript-preview")).toBeNull();
      const shown = inCard().getByTestId("script-displays");
      expect(inCard().getByTestId("transcript-panel")).toContainElement(shown);
      expect(follows(inCard().getByTestId("script-code"), shown)).toBe(true);
      expect(
        follows(shown, inCard().getByRole("region", { name: "Result" })),
      ).toBe(true);
    });
    it("shows the displays under the header again as soon as the card collapses", async () => {
      collapsedWithDisplays();
      openCard();
      openCard();
      expect(toggle()).toHaveAttribute("aria-expanded", "false");
      expect(inCard().getAllByTestId("script-displays")).toHaveLength(1);
      expect(inCard().getByTestId("transcript-preview")).toContainElement(
        inCard().getByTestId("script-displays"),
      );
      await waitFor(() =>
        expect(inCard().queryByTestId("transcript-panel")).toBeNull(),
      );
      expect(inCard().getAllByTestId("script-displays")).toHaveLength(1);
      expect(inCard().getByTestId("transcript-preview")).toContainElement(
        inCard().getByTestId("script-displays"),
      );
      expect(inCard().queryByTestId("script-code")).toBeNull();
    });
    it("has no preview slot on a collapsed card without displays", () => {
      render(<PythonTool toolCall={call} result={done({})} />);
      expect(toggle()).toHaveAttribute("aria-expanded", "false");
      expect(inCard().queryByTestId("transcript-preview")).toBeNull();
      expect(inCard().queryByTestId("script-displays")).toBeNull();
    });
  });
  describe("engine calls and changes", () => {
    const code = [
      'limit = await engine("settings.get", key="autonomy.max_steps")',
      'await engine("settings.set", key="autonomy.max_steps", value=40)',
      'await engine("chat.set_params", temperature=0.1)',
    ].join("\n");
    const calls = [
      engineCall(1, "engine.settings.get", 1, {
        summary: "autonomy.max_steps = 25",
      }),
      engineCall(2, "engine.settings.set", 2, {
        approval: "allowed",
        summary: "autonomy.max_steps: 25 → 40",
        duration_ms: 1500,
      }),
      engineCall(3, "engine.chat.set_params", 3, {
        approval: "allowed",
        summary: "temperature: 0.2 → 0.1",
      }),
    ];
    const changes = [
      engineChange(2),
      engineChange(3, {
        function: "chat.set_params",
        class: "risky",
        title: "change this chat's parameters",
        scope: "this chat",
        items: [{ label: "temperature", old: 0.2, new: 0.1 }],
      }),
    ];
    const result = (extra = {}) =>
      done({
        calls,
        calls_total: 3,
        files_written: [],
        engine_changes: changes,
        engine_changes_total: 2,
        ...extra,
      });
    it("highlights engine as a callee and annotates engine call lines", () => {
      const { container } = render(
        <PythonTool toolCall={withCode(code)} result={result()} />,
      );
      openCard();
      expect(
        [...container.querySelectorAll("[class*=callee]")].map(
          (node) => node.textContent,
        ),
      ).toEqual(["engine", "engine", "engine"]);
      expect(lineEl(2)).toHaveTextContent("Allowed once · 1.5 s");
      expect(lineEl(1)).toHaveTextContent("12 ms");
    });
    it("details an engine call with its function, kind, result and arguments", () => {
      render(<PythonTool toolCall={withCode(code)} result={result()} />);
      openCard();
      fireEvent.click(lineEl(3));
      const detail = lineEl(3).parentElement;
      expect(detail).toHaveTextContent("Functionchat.set_params");
      expect(detail).toHaveTextContent("KindRisky change");
      expect(detail).toHaveTextContent("temperature: 0.2 → 0.1");
      expect(detail).toHaveTextContent("Arguments");
      fireEvent.click(lineEl(1));
      expect(lineEl(1).parentElement).toHaveTextContent("KindRead");
    });
    it("lists applied changes in a bounded section before file changes, with markers", () => {
      render(
        <PythonTool
          toolCall={withCode(code)}
          result={result({
            engine_changes_total: 60,
            files_written: [{ path: "/p/a.txt", bytes: 1, diffs: [] }],
          })}
        />,
      );
      openCard();
      const section = screen.getByRole("region", { name: "Engine changes" });
      expect(section).toHaveTextContent("Engine changes · 60");
      expect(section).toHaveTextContent("Showing 2 of 60");
      expect(section).toHaveTextContent("Change engine settings");
      expect(section).toHaveTextContent("Risky change");
      expect(section).toHaveTextContent("this chat");
      expect(
        section.compareDocumentPosition(
          screen.getByRole("region", { name: "Changes" }),
        ) & Node.DOCUMENT_POSITION_FOLLOWING,
      ).toBeTruthy();
      const meta = screen.getByTestId("transcript-meta");
      expect(meta.querySelector("[class*=srOnly]")).toHaveTextContent(
        "3 calls · 1 file · 60 engine changes",
      );
      expect(meta.querySelector("[class*=metaNarrow]")).toHaveTextContent(
        "3 calls · 1 file · 60 changes",
      );
    });
    it("cuts long titles and labels and marks bidi characters in the changes section", () => {
      render(
        <PythonTool
          toolCall={withCode(code)}
          result={result({
            engine_changes: [
              engineChange(3, {
                title: `rename \u202Etxt.exe\u202C ${"x".repeat(5000)}`,
                function: `settings.${"f".repeat(5000)}`,
                scope: "s".repeat(5000),
                items: [
                  { label: `k\u200B${"l".repeat(5000)}`, old: 1, new: 2 },
                ],
              }),
            ],
            engine_changes_total: 1,
          })}
        />,
      );
      openCard();
      const section = screen.getByRole("region", { name: "Engine changes" });
      const text = section.textContent ?? "";
      expect(text).not.toMatch(/[\u202A-\u202E\u200B]/);
      expect(text).toContain("⟨U+202E⟩txt.exe⟨U+202C⟩");
      expect(text).toContain("k⟨U+200B⟩");
      expect(text.length).toBeLessThan(1000);
      expect(
        within(section).getAllByRole("button", {
          name: /Show all \d+ characters/,
        }).length,
      ).toBeGreaterThanOrEqual(3);
    });
    it("mentions engine changes in the meta only when there are some", () => {
      render(
        <PythonTool
          toolCall={withCode(code)}
          result={result({
            engine_changes: undefined,
            engine_changes_total: 0,
          })}
        />,
      );
      openCard();
      expect(screen.getByTestId("transcript-meta")).not.toHaveTextContent(
        "engine",
      );
      expect(
        screen.queryByRole("region", { name: "Engine changes" }),
      ).toBeNull();
    });
    it("counts approved engine calls while a run is live", () => {
      live.calls = calls;
      render(<PythonTool toolCall={withCode(code)} />);
      expect(
        screen.getByTestId("transcript-meta").querySelector("[class*=srOnly]"),
      ).toHaveTextContent("3 calls · 2 engine changes");
    });
    it("marks a change whose outcome is unknown and still counts it", () => {
      render(
        <PythonTool
          toolCall={withCode(code)}
          result={result({
            engine_changes: [unknownOutcomeChange(2), changes[1]],
          })}
        />,
      );
      openCard();
      const rows = [
        ...screen.getByRole("list", { name: "Engine changes list" }).children,
      ] as HTMLElement[];
      expect(rows).toHaveLength(2);
      expect(rows[0]).toHaveAttribute("data-outcome", "unknown");
      expect(rows[0]).toHaveTextContent("Outcome unknown");
      expect(rows[0]).toHaveTextContent(
        "The engine could not confirm whether this change was applied.",
      );
      const marker = within(rows[0]).getByText("Outcome unknown");
      expect(marker.querySelector("svg")).not.toBeNull();
      expect(marker.className).toMatch(/outcome/);
      expect(marker.className).not.toMatch(/marker/);
      expect(rows[1]).not.toHaveAttribute("data-outcome");
      expect(rows[1]).not.toHaveTextContent("Outcome unknown");
      expect(rows[1]).toHaveTextContent("Risky change");
      expect(
        screen.getByTestId("transcript-meta").querySelector("[class*=srOnly]"),
      ).toHaveTextContent("2 engine changes");
      expect(card().textContent).not.toMatch(/python/i);
    });
    it("names refused and cancelled changes as not applied, reads as reads", () => {
      const kindCode = [
        'await engine("settings.set", key="a", value=1)',
        'await engine("settings.set", key="b", value=2)',
        'await engine("settings.get", key="c")',
        'await engine("settings.set", key="d", value=3)',
      ].join("\n");
      render(
        <PythonTool
          toolCall={withCode(kindCode)}
          result={done({
            calls: [
              engineCall(1, "engine.settings.set", 1, {
                approval: "denied",
                status: "denied",
                summary: ENGINE_REFUSAL_MESSAGE,
              }),
              engineCall(2, "engine.settings.set", 2, {
                approval: "cancelled",
                status: "cancelled",
              }),
              engineCall(3, "engine.settings.get", 3),
              engineCall(4, "engine.settings.set", 4, {
                approval: "allowed",
              }),
            ],
            calls_total: 4,
            files_written: [],
          })}
        />,
      );
      openCard();
      for (const number of [1, 2, 3, 4]) fireEvent.click(lineEl(number));
      const detail = (number: number) => lineEl(number).parentElement;
      expect(detail(1)).toHaveTextContent("KindChange (not applied)");
      expect(detail(1)).toHaveTextContent(ENGINE_REFUSAL_MESSAGE);
      expect(detail(2)).toHaveTextContent("KindChange (not applied)");
      expect(detail(3)).toHaveTextContent("KindRead");
      expect(detail(4)).not.toHaveTextContent("Kind");
    });
    it("gives each engine call its kind in the nested rows of one line", () => {
      render(
        <PythonTool
          toolCall={withCode(
            'for k in keys: await engine("settings.set", key=k, value=1)',
          )}
          result={done({
            calls: [
              engineCall(1, "engine.settings.set", 1, {
                approval: "denied",
                status: "denied",
                summary: ENGINE_REFUSAL_MESSAGE,
              }),
              engineCall(2, "engine.settings.set", 1, { approval: "allowed" }),
            ],
            calls_total: 2,
            files_written: [],
            engine_changes: [engineChange(2, { class: "risky" })],
          })}
        />,
      );
      openCard();
      fireEvent.click(lineEl(1));
      const detail = lineEl(1).parentElement;
      if (!detail) throw new Error("line 1 has no detail");
      const rows = within(detail).getAllByRole("button", {
        name: /engine\.settings\.set/,
      });
      expect(rows).toHaveLength(2);
      for (const row of rows) fireEvent.click(row);
      const items = rows.map((row) => row.closest("li"));
      expect(items[0]).toHaveTextContent("KindChange (not applied)");
      expect(items[1]).toHaveTextContent("KindRisky change");
    });
    it("says when the engine cut the result, compact and expanded", () => {
      render(
        <PythonTool
          toolCall={withCode(code)}
          result={result({
            result_preview: '{"total": 3}',
            result_truncated: true,
          })}
        />,
      );
      openCard();
      const block = () => screen.getByRole("region", { name: "Result" });
      expect(block()).toHaveTextContent("Result cut by the engine");
      fireEvent.click(screen.getByRole("button", { name: "Details" }));
      expect(screen.getByRole("button", { name: "Details" })).toHaveAttribute(
        "aria-expanded",
        "true",
      );
      expect(block()).toHaveTextContent("Result cut by the engine");
    });
    it("says nothing about cutting when the engine kept the whole result", () => {
      render(
        <PythonTool
          toolCall={withCode(code)}
          result={result({ result_preview: '{"total": 3}' })}
        />,
      );
      openCard();
      expect(
        screen.getByRole("region", { name: "Result" }),
      ).not.toHaveTextContent("cut by the engine");
    });
  });

  describe("error hint, session state and read-only", () => {
    const HINT = "del is not supported: use x = None to drop a value.";
    const failure = (hint?: string, more: Partial<PythonRunExtra> = {}) =>
      done({
        status: "failed",
        error: {
          type: "NotImplementedError",
          message: "no del",
          traceback: "",
          ...(hint === undefined ? {} : { hint }),
        },
        ...more,
      });
    beforeEach(() => {
      live.status = undefined;
    });
    it("shows the error's hint under the message, quieter, never a blank one", () => {
      const view = render(
        <PythonTool toolCall={call} result={failure(HINT)} />,
      );
      openCard();
      const error = screen.getByText("NotImplementedError").closest("p");
      if (!error) throw new Error("no error line");
      const hint = screen.getByText(HINT);
      expect(error.compareDocumentPosition(hint)).toBe(
        Node.DOCUMENT_POSITION_FOLLOWING,
      );
      expect(hint.className).toMatch(/stateHint/);
      expect(hint.className).not.toMatch(/stateDanger/);
      expect(error).not.toHaveTextContent(HINT);
      view.unmount();
      render(<PythonTool toolCall={call} result={failure("   ")} />);
      openCard();
      expect(screen.getByText("NotImplementedError")).toBeInTheDocument();
      expect(document.querySelector("[class*=stateHint]")).toBeNull();
    });
    it("announces the hint with a failure that happens while the card is watched", () => {
      live.status = "running";
      const view = render(<PythonTool toolCall={call} />);
      openCard();
      live.status = undefined;
      view.rerender(<PythonTool toolCall={call} result={failure(HINT)} />);
      expect(screen.getByRole("alert")).toHaveTextContent(
        `NotImplementedError: no del${HINT}`,
      );
    });
    it("warns when the state was not saved and lists each engine note on its own line", () => {
      render(
        <PythonTool
          toolCall={call}
          result={done({
            state: {
              ...extra.state,
              persisted: false,
              not_saved:
                "not saved (disk full); variables come from the last successful save",
              note: "the session holds 18.0 MB of variables",
              notes: [
                "variables were not copied from chat a: a script there was running",
              ],
            },
          })}
        />,
      );
      openCard();
      const warning = screen.getByText(
        "State not saved (disk full); variables come from the last successful save.",
      );
      expect(warning.className).toMatch(/stateWarning/);
      const lines = [
        screen.getByText("The session holds 18.0 MB of variables."),
        screen.getByText(
          "Variables were not copied from chat a: a script there was running.",
        ),
      ];
      for (const line of lines) {
        expect(line.className).toMatch(/stateMuted/);
        expect(warning.compareDocumentPosition(line)).toBe(
          Node.DOCUMENT_POSITION_FOLLOWING,
        );
        expect(
          line.compareDocumentPosition(
            screen.getByRole("region", { name: "Result" }),
          ),
        ).toBe(Node.DOCUMENT_POSITION_FOLLOWING);
      }
    });
    it("keeps the card structured for the engine's state with null notes", () => {
      render(
        <PythonTool
          toolCall={call}
          result={{
            tool_call_id: "t",
            content: "RAW TEXT",
            extra: {
              python: {
                ...extra,
                state: { ...extra.state, not_saved: null, note: null },
              },
            },
          }}
        />,
      );
      openCard();
      expect(card()).toHaveAttribute("data-python-run-id", "r");
      expect(screen.getByRole("region", { name: "Result" })).toHaveTextContent(
        "answer",
      );
      expect(screen.queryByText("RAW TEXT")).toBeNull();
      expect(document.querySelector("[class*=stateWarning]")).toBeNull();
    });
    it("marks a read-only run with a chip after its status, explained in a tooltip", async () => {
      const view = render(
        <PythonTool
          toolCall={call}
          result={failure(undefined, { read_only: true })}
        />,
      );
      const chip = screen.getByText("Read-only").closest("[tabindex]");
      if (!(chip instanceof HTMLElement)) throw new Error("no focusable chip");
      expect(
        screen
          .getByTestId("transcript-status-pill")
          .compareDocumentPosition(chip),
      ).toBe(Node.DOCUMENT_POSITION_FOLLOWING);
      expect(chip).toHaveAttribute("tabindex", "0");
      await view.user.hover(chip);
      expect(await screen.findByRole("tooltip")).toHaveTextContent(
        READ_ONLY_NOTE,
      );
      expect(READ_ONLY_NOTE).toMatch(/artifacts/);
    });
    it("shows no read-only chip for a run that could write files", () => {
      render(<PythonTool toolCall={call} result={done({})} />);
      expect(screen.queryByText("Read-only")).toBeNull();
    });
  });
});
