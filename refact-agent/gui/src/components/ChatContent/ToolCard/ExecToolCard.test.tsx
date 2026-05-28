import { describe, expect, test, vi } from "vitest";
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
import {
  reducer as configReducer,
  type Config,
} from "../../../features/Config/configSlice";
import type {
  ExecToolMetadata,
  ToolCall,
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
  failed?: boolean;
  host?: Config["host"];
};

function makeStore(toolMessage?: ToolMessage, host: Config["host"] = "web") {
  const config: Config = {
    host,
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
                messages: toolMessage ? [toolMessage] : [],
              },
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
  };
  const message: ToolMessage | undefined =
    options.content !== undefined || options.extra !== undefined
      ? {
          role: "tool",
          tool_call_id: id,
          content: options.content ?? "",
          tool_failed: options.failed,
          extra: options.extra ? { exec: options.extra } : undefined,
        }
      : undefined;
  const store = makeStore(message, options.host);

  return render(
    <Provider store={store}>
      <Theme>
        <ExecToolCard toolCall={toolCall} toolName={toolName} />
      </Theme>
    </Provider>,
  );
}

describe("ExecToolCard", () => {
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
    expect(screen.getByText("npm test")).toBeInTheDocument();
    expect(screen.getByText("/workspace")).toBeInTheDocument();
    expect(screen.getByText("0.2s")).toBeInTheDocument();
    expect(screen.getByText("0")).toBeInTheDocument();
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
  });

  test("persisted_output_path renders Open log button", () => {
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

    expect(
      screen.getByRole("button", { name: /open log/i }),
    ).toBeInTheDocument();
  });

  test("Open log fetches and opens a blob in web host mode", async () => {
    const blob = new Blob(["log output"], { type: "text/plain" });
    const fetchMock = vi.fn().mockResolvedValue({
      blob: vi.fn().mockResolvedValue(blob),
    });
    const openSpy = vi.spyOn(window, "open").mockImplementation(() => null);
    const createObjectURL = vi.fn(() => "blob:exec-log");
    const revokeObjectURL = vi.fn();
    const previousFetch = globalThis.fetch;
    const previousCreateObjectURL = URL.createObjectURL.bind(URL);
    const previousRevokeObjectURL = URL.revokeObjectURL.bind(URL);
    Object.defineProperty(globalThis, "fetch", {
      configurable: true,
      value: fetchMock,
    });
    Object.defineProperty(URL, "createObjectURL", {
      configurable: true,
      value: createObjectURL,
    });
    Object.defineProperty(URL, "revokeObjectURL", {
      configurable: true,
      value: revokeObjectURL,
    });

    renderExecTool({
      host: "web",
      content: "output",
      extra: {
        process_id: "exec_log_web",
        status: "exited",
        short_description: "Read web log",
        command: "cat big.log",
        persisted_output_path: "/tmp/refact/exec/exec_log_web.log",
      },
    });

    fireEvent.click(screen.getByRole("button", { name: /open log/i }));
    expect(fetchMock).toHaveBeenCalledWith("/tmp/refact/exec/exec_log_web.log");
    await waitFor(() => {
      expect(openSpy).toHaveBeenCalledWith(
        "blob:exec-log",
        "_blank",
        "noopener,noreferrer",
      );
    });

    cleanup();
    openSpy.mockRestore();
    Object.defineProperty(globalThis, "fetch", {
      configurable: true,
      value: previousFetch,
    });
    Object.defineProperty(URL, "createObjectURL", {
      configurable: true,
      value: previousCreateObjectURL,
    });
    Object.defineProperty(URL, "revokeObjectURL", {
      configurable: true,
      value: previousRevokeObjectURL,
    });
  });

  test("Open log dispatches ideOpenFile in IDE host mode", () => {
    const postMessageSpy = vi
      .spyOn(window, "postMessage")
      .mockImplementation(() => undefined);

    renderExecTool({
      host: "jetbrains",
      content: "output",
      extra: {
        process_id: "exec_log_ide",
        status: "exited",
        short_description: "Read IDE log",
        command: "cat big.log",
        persisted_output_path: "/tmp/refact/exec/exec_log_ide.log",
      },
    });

    fireEvent.click(screen.getByRole("button", { name: /open log/i }));
    expect(postMessageSpy).toHaveBeenCalledWith(
      {
        type: "ide/openFile",
        payload: { file_path: "/tmp/refact/exec/exec_log_ide.log" },
      },
      "*",
    );

    cleanup();
    postMessageSpy.mockRestore();
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

    expect(screen.getByText("stdout")).toBeInTheDocument();
    expect(screen.getByText("stderr")).toBeInTheDocument();
    expect(screen.getByText("hello out")).toBeInTheDocument();
    expect(screen.getByText("warn err")).toBeInTheDocument();
    expect(
      screen.getByText(/Cursor: since 2 · next 5 · latest 4/u),
    ).toBeInTheDocument();
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
    expect(screen.getByText("legacy plain output")).toBeInTheDocument();
    expect(
      screen.getByText(/structured process metadata was not available/u),
    ).toBeInTheDocument();
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
