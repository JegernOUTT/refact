import { act } from "react-dom/test-utils";
import { afterEach, describe, expect, test, vi } from "vitest";
import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { Provider } from "react-redux";
import { configureStore } from "@reduxjs/toolkit";
import { Theme } from "@radix-ui/themes";

import { ExecToolCard } from "./ExecToolCard";
import { CollapsibleStoreProvider } from "../useStoredOpen";
import type { CollapsibleStore } from "../CollapsibleStore";
import {
  reducer as configReducer,
  updateConfig,
  type Config,
} from "../../../features/Config/configSlice";
import type {
  ExecToolMetadata,
  ChatMessage,
  ToolCall,
  ToolEnrichment,
  ToolMessage,
} from "../../../services/refact/types";
import {
  extractExecMetadata,
  isExecToolMetadata,
} from "../../../services/refact/types";
import { INCIDENTAL_EXTRA_EXEC } from "../../../__fixtures__";

type RenderExecToolOptions = {
  toolName?: React.ComponentProps<typeof ExecToolCard>["toolName"];
  args?: Record<string, unknown>;
  content?: string;
  extra?: ExecToolMetadata;
  toolEnrichment?: ToolEnrichment;
  failed?: boolean;
  host?: Config["host"];
  subchatLog?: string[];
  collapsibleStore?: CollapsibleStore;
  busy?: boolean;
  config?: Partial<Config>;
};

function makeStore(messages: ChatMessage[], options: RenderExecToolOptions) {
  const config: Config = {
    host: options.host ?? "web",
    lspPort: 8001,
    apiKey: null,
    features: {
      statistics: true,
      vecdb: true,
      ast: true,
      images: true,
    },
    themeProps: {
      appearance: "dark",
    },
    shiftEnterToSubmit: false,
    engineServed: true,
    ...options.config,
  };

  return configureStore({
    reducer: {
      config: configReducer,
      chat: (
        state = {
          current_thread_id: "chat-1",
          threads: {
            "chat-1": {
              thread: {
                messages,
              },
              streaming: options.busy === true,
              waiting_for_response: false,
            },
          },
        },
      ) => state,
    },
    preloadedState: {
      config,
    },
  });
}

function renderExecTool(options: RenderExecToolOptions = {}) {
  const id = "tc-exec";
  const toolName = options.toolName ?? "shell";
  const args = options.args ?? { command: "npm test", workdir: "/workspace" };
  const toolCall: ToolCall = {
    id,
    index: 0,
    type: "function",
    function: {
      name: toolName,
      arguments: JSON.stringify(args),
    },
    subchat_log: options.subchatLog,
  };
  const message: ToolMessage | undefined =
    options.content !== undefined ||
    options.extra !== undefined ||
    options.toolEnrichment !== undefined
      ? {
          role: "tool",
          tool_call_id: id,
          content: options.content ?? "",
          tool_failed: options.failed,
          extra:
            options.extra ?? options.toolEnrichment
              ? {
                  ...(options.extra ? { exec: options.extra } : {}),
                  ...(options.toolEnrichment
                    ? { tool_enrichment: options.toolEnrichment }
                    : {}),
                }
              : undefined,
        }
      : undefined;
  const store = makeStore(
    [
      { role: "assistant", content: "", tool_calls: [toolCall] },
      ...(message ? [message] : []),
    ],
    options,
  );

  const card = <ExecToolCard toolCall={toolCall} toolName={toolName} />;

  return {
    store,
    ...render(
      <Provider store={store}>
        <Theme>
          {options.collapsibleStore ? (
            <CollapsibleStoreProvider value={options.collapsibleStore}>
              {card}
            </CollapsibleStoreProvider>
          ) : (
            card
          )}
        </Theme>
      </Provider>,
    ),
  };
}

describe("ExecToolCard", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });
  test("shell card renders short description, command, cwd, status, and process id", () => {
    renderExecTool({
      content:
        "hello\n\nThe command was running 0.2s, finished with exit code 0\n",
      extra: {
        process_id: "exec_shell_1",
        status: "exited",
        short_description: "Run tests",
        command: "npm test",
        cwd: "/workspace",
        mode: "foreground",
        exit_code: 0,
        duration_ms: 200,
        transcript: {
          current_bytes: 5,
          next_seq: 1,
          latest_seq: 0,
        },
      },
    });

    expect(screen.getByTestId("exec-tool-card")).toHaveAttribute(
      "data-exec-process-id",
      "exec_shell_1",
    );
    expect(screen.getByText("Run tests")).toBeInTheDocument();
    expect(screen.getByText("exec_shell_1")).toBeInTheDocument();
    expect(screen.getByTestId("exec-status-exited")).toHaveTextContent(
      "exited",
    );

    fireEvent.click(screen.getByText("Run tests"));

    expect(screen.getByText("npm test")).toBeInTheDocument();
    expect(screen.getByText("/workspace")).toBeInTheDocument();
    expect(screen.getByText("0.2s")).toBeInTheDocument();
    expect(screen.getByText("0")).toBeInTheDocument();
  });

  test("renders unified path references and opens a relative IDE file payload", () => {
    const postMessageSpy = vi
      .spyOn(window, "postMessage")
      .mockImplementation(() => undefined);
    vi.stubGlobal("postIntellijMessage", (message: Record<string, unknown>) =>
      window.postMessage(message, "*"),
    );
    try {
      renderExecTool({
        host: "jetbrains",
        extra: {
          process_id: "exec_paths_1",
          status: "exited",
          short_description: "Run path diagnostics",
        },
        toolEnrichment: {
          schema_version: 1,
          references: [
            {
              kind: "path",
              target: "src/main.rs",
              provenance: "heuristic",
              line1: 7,
              line2: 7,
              source: "diagnostic",
              confidence: 0.9,
            },
          ],
        },
      });

      fireEvent.click(screen.getByText("Run path diagnostics"));
      fireEvent.click(screen.getByRole("button", { name: "src/main.rs:7" }));

      expect(postMessageSpy).toHaveBeenCalledWith(
        {
          type: "ide/openFile",
          payload: { file_path: "src/main.rs", line: 7 },
        },
        "*",
      );
    } finally {
      postMessageSpy.mockRestore();
    }
  });

  test("hides path references from a restricted unified envelope", () => {
    renderExecTool({
      extra: {
        process_id: "exec_paths_restricted",
        status: "exited",
        short_description: "Run restricted diagnostics",
      },
      toolEnrichment: {
        schema_version: 1,
        references: [
          {
            kind: "path",
            target: "src/main.rs",
            provenance: "heuristic",
          },
        ],
        privacy: { redacted: true, restricted: true },
      },
    });

    fireEvent.click(screen.getByText("Run restricted diagnostics"));
    expect(
      screen.queryByTestId("exec-path-references"),
    ).not.toBeInTheDocument();
  });

  test("collapses with delayed unmount and persists reopened state by tool-call id", () => {
    vi.useFakeTimers();
    const storedOpen = new Map<string, boolean>([["tc:tc-exec", true]]);
    const advanceTimersByTime: (ms: number) => void =
      vi.advanceTimersByTime.bind(vi);
    const collapsibleStore: CollapsibleStore = {
      get: (key) => storedOpen.get(key),
      set: (key, open) => storedOpen.set(key, open),
    };

    try {
      renderExecTool({
        content: "stdout:\npersisted output\nstderr:\n<empty>\n",
        extra: {
          process_id: "exec_persisted_open",
          status: "exited",
          short_description: "Persisted card",
          command: "npm test",
        },
        collapsibleStore,
      });

      expect(screen.getByTestId("exec-output-view")).toBeInTheDocument();

      fireEvent.click(screen.getByText("Persisted card"));
      expect(storedOpen.get("tc:tc-exec")).toBe(false);
      expect(screen.getByTestId("exec-output-view")).toBeInTheDocument();

      act(() => {
        advanceTimersByTime(199);
      });
      expect(screen.getByTestId("exec-output-view")).toBeInTheDocument();

      act(() => {
        advanceTimersByTime(1);
      });
      expect(screen.queryByTestId("exec-output-view")).not.toBeInTheDocument();

      fireEvent.click(screen.getByText("Persisted card"));
      expect(storedOpen.get("tc:tc-exec")).toBe(true);
      expect(screen.getByTestId("exec-output-view")).toBeInTheDocument();
    } finally {
      vi.useRealTimers();
    }
  });

  test("process_start running background card uses runtime metadata", () => {
    renderExecTool({
      toolName: "process_start",
      args: { command: "npm run dev", mode: "background" },
      content: "Process started\nstdout:\nserver ready\nstderr:\n<empty>\n",
      extra: {
        process_id: "exec_bg_1",
        status: "running",
        short_description: "Start dev server",
        command: "npm run dev",
        cwd: "/workspace/app",
        mode: "background",
        started_at_ms: Date.now() - 4000,
        transcript: {
          current_bytes: 12,
          next_seq: 3,
          latest_seq: 2,
        },
      },
    });

    expect(screen.getByTestId("exec-tool-process_start")).toBeInTheDocument();
    expect(screen.getByText("Start dev server")).toBeInTheDocument();
    expect(
      screen.getByTestId("exec-status-running_in_background"),
    ).toHaveTextContent("background");
    expect(screen.getByText("exec_bg_1")).toBeInTheDocument();

    fireEvent.click(screen.getByText("Start dev server"));

    expect(screen.getByText("server ready")).toBeInTheDocument();
  });

  test("persisted_output_path offers retained transcript and copy path, never a cache URL", () => {
    const copy = vi.fn().mockResolvedValue(undefined);
    vi.stubGlobal("navigator", { clipboard: { writeText: copy } });
    renderExecTool({
      content: "output",
      extra: {
        process_id: "exec_log_1",
        status: "exited",
        short_description: "Read log",
        command: "cat big.log",
        persisted_output_path: "/tmp/refact/exec/exec_log_1.log",
      },
    });

    fireEvent.click(screen.getByText("Read log"));

    expect(screen.queryByRole("button", { name: /open log/i })).toBeNull();
    expect(
      screen.getByRole("button", { name: "Read process transcript" }),
    ).toBeEnabled();
    fireEvent.click(screen.getByRole("button", { name: "Copy log path" }));
    expect(copy).toHaveBeenCalledWith("/tmp/refact/exec/exec_log_1.log");
    expect(
      screen.getByText(/Full disk log unavailable here/u),
    ).toBeInTheDocument();
  });

  test("web transcript reads use exact process, chat, endpoint and authorization without popups", async () => {
    const copy = vi.fn().mockResolvedValue(undefined);
    vi.stubGlobal("navigator", { clipboard: { writeText: copy } });
    const fetchMock = vi.fn<typeof fetch>().mockResolvedValue(
      new Response(
        JSON.stringify({
          chunks: [{ seq: 3, stream: "stdout", text: "retained log output\n" }],
          next_seq: 4,
          status: "exited",
        }),
        { status: 200 },
      ),
    );
    vi.stubGlobal("fetch", fetchMock);
    const openSpy = vi.spyOn(window, "open").mockReturnValue(null);
    renderExecTool({
      config: {
        engineServed: false,
        lspUrl: "https://engine.example/api",
        apiKey: "log-token",
      },
      content: "output",
      extra: {
        process_id: "exec/log web",
        status: "exited",
        short_description: "Read web log",
        persisted_output_path: "/tmp/refact/private.log",
      },
    });
    fireEvent.click(screen.getByText("Read web log"));
    fireEvent.click(
      screen.getByRole("button", { name: "Read process transcript" }),
    );
    await waitFor(() =>
      expect(screen.getByText("retained log output")).toBeInTheDocument(),
    );
    expect(fetchMock).toHaveBeenCalledTimes(1);
    expect(fetchMock).toHaveBeenCalledWith(
      "https://engine.example/api/v1/exec/exec%2Flog%20web/read?since_seq=0&limit=10000&chat_id=chat-1",
      expect.objectContaining({
        headers: expect.objectContaining({
          Authorization: "Bearer log-token",
        }) as unknown,
      }),
    );
    expect(screen.getByText(/Not the full disk log/u)).toBeInTheDocument();
    expect(openSpy).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "Copy transcript" }));
    expect(copy).toHaveBeenCalledWith("retained log output\n");
  });

  test("transcript read errors are visible and late output cannot cross engine endpoints", async () => {
    let resolveRead: (response: Response) => void = () => {
      throw new Error("read not started");
    };
    const fetchMock = vi
      .fn<typeof fetch>()
      .mockResolvedValueOnce(
        new Response(JSON.stringify({ detail: "Process access denied" }), {
          status: 403,
        }),
      )
      .mockImplementationOnce(
        () =>
          new Promise<Response>((resolve) => {
            resolveRead = resolve;
          }),
      );
    vi.stubGlobal("fetch", fetchMock);
    const { store } = renderExecTool({
      content: "recorded output",
      extra: {
        process_id: "owned-process",
        status: "exited",
        short_description: "Scoped log",
      },
    });
    fireEvent.click(screen.getByText("Scoped log"));
    fireEvent.click(
      screen.getByRole("button", { name: "Read process transcript" }),
    );
    await waitFor(() =>
      expect(screen.getByRole("alert")).toHaveTextContent(
        "Process access denied",
      ),
    );
    fireEvent.click(
      screen.getByRole("button", { name: "Read process transcript" }),
    );
    act(() => {
      store.dispatch(
        updateConfig({
          engineServed: false,
          lspUrl: "https://other-engine.example",
        }),
      );
    });
    await act(() => {
      resolveRead(
        new Response(
          JSON.stringify({
            chunks: [{ seq: 0, stream: "stdout", text: "foreign late output" }],
            next_seq: 1,
            status: "exited",
          }),
        ),
      );
      return Promise.resolve();
    });
    expect(screen.queryByText("foreign late output")).toBeNull();
    expect(screen.queryByRole("alert")).toBeNull();
    expect(screen.getByText("recorded output")).toBeInTheDocument();
  });

  test("Open log uses only a supported native bridge and never a web fallback", () => {
    const nativePost = vi.fn();
    vi.stubGlobal("postIntellijMessage", nativePost);
    const fallback = vi
      .spyOn(window, "postMessage")
      .mockImplementation(() => undefined);
    renderExecTool({
      host: "jetbrains",
      content: "output",
      extra: {
        process_id: "exec_log_ide",
        status: "exited",
        short_description: "Read IDE log",
        persisted_output_path: "/tmp/refact/exec/exec_log_ide.log",
      },
    });
    fireEvent.click(screen.getByText("Read IDE log"));
    fireEvent.click(screen.getByRole("button", { name: "Open log in IDE" }));
    expect(nativePost).toHaveBeenCalledWith({
      type: "ide/openFile",
      payload: { file_path: "/tmp/refact/exec/exec_log_ide.log" },
    });
    expect(fallback).not.toHaveBeenCalled();
    nativePost.mockImplementation(() => {
      throw new Error("IDE bridge unavailable");
    });
    fireEvent.click(screen.getByRole("button", { name: "Open log in IDE" }));
    expect(screen.getByRole("alert")).toHaveTextContent(
      "IDE bridge unavailable",
    );
    cleanup();
    vi.unstubAllGlobals();
    renderExecTool({
      host: "jetbrains",
      content: "output",
      extra: {
        process_id: "unsupported",
        status: "exited",
        short_description: "Unsupported IDE",
        persisted_output_path: "/tmp/unsupported.log",
      },
    });
    fireEvent.click(screen.getByText("Unsupported IDE"));
    expect(
      screen.queryByRole("button", { name: "Open log in IDE" }),
    ).toBeNull();
    expect(
      screen.getByText(/requires a supported IDE bridge/u),
    ).toBeInTheDocument();
    expect(fallback).not.toHaveBeenCalled();
  });

  test("process_write_stdin result shows bytes_written and chunks_returned", () => {
    renderExecTool({
      toolName: "process_write_stdin",
      args: { process_id: "exec_pty_1", chars: "echo hi\n" },
      content: "Process stdin written",
      extra: {
        process_id: "exec_pty_1",
        status: "running",
        short_description: "Interactive shell",
        command: "bash",
        mode: "background",
        bytes_written: 8,
        chunks_returned: 2,
      },
    });

    fireEvent.click(screen.getByText("Interactive shell"));

    expect(
      screen.getByText("Wrote 8 bytes, got 2 new chunks"),
    ).toBeInTheDocument();
  });

  test("tty true shows PTY chip", () => {
    renderExecTool({
      content: "pty output",
      extra: {
        process_id: "exec_pty_chip",
        status: "running",
        short_description: "Interactive shell",
        command: "bash",
        mode: "background",
        tty: true,
      },
    });

    expect(screen.getByTestId("exec-pty-chip")).toHaveTextContent("PTY");
  });

  test("process_read output renders stdout stderr and cursor metadata", () => {
    renderExecTool({
      toolName: "process_read",
      args: { process_id: "exec_read_1", stream: "all" },
      content: [
        "Process output",
        "process_id: exec_read_1",
        "stdout:",
        "hello out",
        "stderr:",
        "warn err",
        "transcript: next_seq=5, latest_seq=4, current_bytes=18, dropped_bytes=0, truncated_chunks=0, is_truncated=false",
      ].join("\n"),
      extra: {
        process_id: "exec_read_1",
        status: "running",
        short_description: "Read dev server",
        command: "npm run dev",
        mode: "background",
        transcript: {
          since_seq: 2,
          next_seq: 5,
          latest_seq: 4,
          current_bytes: 18,
          is_truncated: false,
        },
      },
    });

    fireEvent.click(screen.getByText("Read dev server"));

    expect(
      screen.getByText(/Cursor: since 2 · next 5 · latest 4/u),
    ).toBeInTheDocument();
    expect(screen.getByText("stdout")).toBeInTheDocument();
    expect(screen.getByText("stderr")).toBeInTheDocument();
    expect(screen.getByText("hello out")).toBeInTheDocument();
    expect(screen.getByText("warn err")).toBeInTheDocument();
  });

  test.each([
    ["failed", "failed"],
    ["killed", "killed"],
    ["timed_out", "timed out"],
  ] as const)("renders %s status distinctly", (status, label) => {
    renderExecTool({
      content: "output",
      extra: {
        process_id: `exec_${status}`,
        status,
        short_description: `Status ${status}`,
        command: "cmd",
      },
      failed: true,
    });

    expect(screen.getByTestId(`exec-status-${status}`)).toHaveTextContent(
      label,
    );
  });

  test("large output is capped and rendered as plain pre text", () => {
    const largeOutput = `stdout:\n${"x".repeat(40_000)}\nstderr:\n<empty>\n`;
    renderExecTool({
      content: largeOutput,
      extra: {
        process_id: "exec_large",
        status: "exited",
        short_description: "Large output",
        command: "cat huge.log",
        transcript: {
          current_bytes: 40_000,
          next_seq: 2,
          latest_seq: 1,
          is_truncated: false,
        },
      },
    });

    fireEvent.click(screen.getByText("Large output"));

    expect(screen.getByTestId("exec-output-view")).toBeInTheDocument();
    expect(screen.getByText(/output capped in UI/u)).toBeInTheDocument();
    expect(screen.getByTestId("exec-truncation-notice")).toBeInTheDocument();
    expect(screen.queryByRole("heading")).not.toBeInTheDocument();
  });

  test("process_list metadata routes with an empty process array", () => {
    renderExecTool({
      toolName: "process_list",
      args: { status: "all" },
      content: "Processes (status: all, scope: all)\ncount: 0\n",
      extra: {
        count: 0,
        status_filter: "all",
        scope_filter: "all",
        processes: [],
      },
    });

    expect(screen.getByTestId("exec-tool-process_list")).toBeInTheDocument();
    expect(screen.getByText("0 processes · all · all")).toBeInTheDocument();
    expect(
      screen.queryByText(/structured process metadata was not available/u),
    ).not.toBeInTheDocument();
  });

  test("plain text exec result without metadata degrades gracefully", () => {
    renderExecTool({ content: "legacy plain output" });

    expect(screen.getByText("Run npm test")).toBeInTheDocument();
    fireEvent.click(screen.getByText("Run npm test"));
    expect(screen.getByText("legacy plain output")).toBeInTheDocument();
    expect(
      screen.getByText(/structured process metadata was not available/u),
    ).toBeInTheDocument();
  });

  test("output is visible after expanding exec cards", () => {
    renderExecTool({
      content: "stdout:\nall tests passed\nstderr:\n<empty>\n",
      extra: {
        process_id: "exec_success_visible",
        status: "exited",
        short_description: "Run tests",
        command: "npm test",
        exit_code: 0,
      },
    });

    expect(
      screen.queryByRole("button", { name: /show output/i }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: /hide output/i }),
    ).not.toBeInTheDocument();
    expect(screen.queryByTestId("exec-output-view")).not.toBeInTheDocument();

    fireEvent.click(screen.getByText("Run tests"));
    expect(screen.getByTestId("exec-output-view")).toBeInTheDocument();
    expect(screen.getByText("all tests passed")).toBeInTheDocument();
  });
  test("recorded process state without output is not presented as live activity", () => {
    renderExecTool({
      toolName: "process_start",
      args: { command: "npm run dev", mode: "background" },
      content: "Process started",
      extra: {
        process_id: "exec_waiting_preview",
        status: "running",
        short_description: "Start dev server",
        command: "npm run dev",
        mode: "background",
        transcript: {
          next_seq: 0,
          latest_seq: 0,
          chunks: [],
        },
      },
    });

    expect(screen.queryByTestId("exec-live-preview")).not.toBeInTheDocument();
    expect(document.querySelector(".rf-text-shimmer")).toBeNull();
    expect(screen.getByText("Recorded")).toBeInTheDocument();
    expect(screen.getByText("Start dev server")).toBeInTheDocument();
    expect(
      screen.getByTestId("exec-status-running_in_background"),
    ).toHaveTextContent("background");
  });

  test("running process shows latest output chunk preview", () => {
    renderExecTool({
      toolName: "process_start",
      args: { command: "npm run dev", mode: "background" },
      content: "Process started",
      extra: {
        process_id: "exec_live_preview",
        status: "running",
        short_description: "Start dev server",
        command: "npm run dev",
        mode: "background",
        transcript: {
          next_seq: 2,
          latest_seq: 1,
          chunks: [
            {
              process_id: "exec_live_preview",
              seq: 0,
              stream: "stdout",
              text: "booting\n",
              timestamp_ms: 1,
            },
            {
              process_id: "exec_live_preview",
              seq: 1,
              stream: "stderr",
              text: "server ready\n",
              timestamp_ms: 2,
            },
          ],
        },
      },
    });

    expect(screen.getByTestId("exec-live-preview")).toHaveTextContent(
      "stderr: server ready",
    );
  });

  test("running process shows subchat progress preview", () => {
    renderExecTool({
      toolName: "shell",
      args: { command: "npm test" },
      busy: true,
      subchatLog: ["stdout:\ncompiling\nstderr:\nwarning: noisy gremlin"],
    });

    expect(screen.getByTestId("exec-live-preview")).toHaveTextContent(
      "warning: noisy gremlin",
    );
  });

  test("failed process stays collapsed until opened", () => {
    renderExecTool({
      content: "stdout:\n<empty>\nstderr:\nboom\n",
      failed: true,
      extra: {
        process_id: "exec_failed_open",
        status: "failed",
        short_description: "Run failing command",
        command: "npm test",
      },
    });

    expect(
      screen.queryByRole("button", { name: /show output/i }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: /hide output/i }),
    ).not.toBeInTheDocument();
    expect(screen.queryByTestId("exec-output-view")).not.toBeInTheDocument();

    fireEvent.click(screen.getByText("Run failing command"));
    expect(screen.getByTestId("exec-output-view")).toBeInTheDocument();
    expect(screen.getByText("boom")).toBeInTheDocument();
    expect(screen.getByTestId("exec-output-view")).toBeInTheDocument();
    expect(screen.getByText("boom")).toBeInTheDocument();
  });
});
describe("isExecToolMetadata", () => {
  test("isExecToolMetadata_accepts_single_process_shape", () => {
    expect(
      isExecToolMetadata({ process_id: "exec_shell_1", status: "running" }),
    ).toBe(true);
  });

  test("isExecToolMetadata_accepts_process_list_shape", () => {
    expect(isExecToolMetadata({ processes: [] })).toBe(true);
  });

  test("isExecToolMetadata_rejects_incidental_command_only", () => {
    expect(isExecToolMetadata(INCIDENTAL_EXTRA_EXEC.exec)).toBe(false);
    expect(extractExecMetadata(INCIDENTAL_EXTRA_EXEC)).toBeUndefined();
  });

  test("isExecToolMetadata_rejects_status_only_without_process_id", () => {
    expect(isExecToolMetadata({ status: "running" })).toBe(false);
  });

  test("isExecToolMetadata_rejects_empty_object", () => {
    expect(isExecToolMetadata({})).toBe(false);
  });
});
