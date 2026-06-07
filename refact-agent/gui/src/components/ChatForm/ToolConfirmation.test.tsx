import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "../../utils/test-utils";
import type {
  ChatMessages,
  ToolCall,
  ToolConfirmationPauseReason,
} from "../../services/refact";
import { ToolConfirmation } from "./ToolConfirmation";
import { createDefaultChatState } from "../../utils/test-utils";
import type { ChatCommandBase } from "../../services/refact/chatCommands";

function patchToolCall(id: string, path: string): ToolCall {
  return {
    id,
    index: 0,
    type: "function",
    function: {
      name: "update_textdoc",
      arguments: JSON.stringify({ path }),
    },
  };
}

function renderToolConfirmation(
  pauseReasons: ToolConfirmationPauseReason[],
  toolCalls: ToolCall[] = [],
) {
  const chat = createDefaultChatState();
  const runtime = chat.threads[chat.current_thread_id];
  runtime.thread.messages = toolCalls.length
    ? ([
        {
          role: "assistant",
          content: "",
          tool_calls: toolCalls,
        },
      ] as ChatMessages)
    : [];

  return render(<ToolConfirmation pauseReasons={pauseReasons} />, {
    preloadedState: { chat },
  });
}

function commandBodies(fetchMock: ReturnType<typeof vi.fn>): ChatCommandBase[] {
  return fetchMock.mock.calls.map((call): ChatCommandBase => {
    const init = call[1] as RequestInit;
    return JSON.parse(init.body as string) as ChatCommandBase;
  });
}

function lastCommandBody(fetchMock: ReturnType<typeof vi.fn>): ChatCommandBase {
  const bodies = commandBodies(fetchMock);
  const last = bodies.at(-1);
  if (!last) throw new Error("expected command body");
  return last;
}

const patchReasons: ToolConfirmationPauseReason[] = [
  {
    type: "confirmation",
    tool_name: "update_textdoc",
    tool_call_id: "patch-1",
    command: "update_textdoc",
    rule: "default",
    integr_config_path: null,
  },
  {
    type: "confirmation",
    tool_name: "update_textdoc",
    tool_call_id: "patch-2",
    command: "update_textdoc",
    rule: "default",
    integr_config_path: null,
  },
];

describe("ToolConfirmation", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
  });

  it("extracts cache-guard diff, displays estimated USD, and sends allow/stop decisions", async () => {
    const fetchMock = vi.fn().mockResolvedValue({ ok: true });
    vi.stubGlobal("fetch", fetchMock);
    const pauseReasons: ToolConfirmationPauseReason[] = [
      {
        type: "confirmation",
        tool_name: "cache_guard",
        tool_call_id: "cacheguard_1",
        command: [
          "Estimated extra cost: `$0.42` USD",
          "```diff",
          "- cached prefix",
          "+ changed prefix",
          "```",
        ].join("\n"),
        rule: "cache_guard",
        integr_config_path: null,
      },
    ];

    renderToolConfirmation(pauseReasons);

    expect(screen.getByText("Prompt cache may be broken")).toBeInTheDocument();
    expect(screen.getByText("$0.42 USD")).toBeInTheDocument();
    expect(screen.getByText(/- cached prefix/u)).toBeInTheDocument();
    expect(screen.getByText(/\+ changed prefix/u)).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: /force and continue/i }));
    await waitFor(() => expect(fetchMock).toHaveBeenCalled());
    expect(lastCommandBody(fetchMock)).toMatchObject({
      type: "tool_decisions",
      decisions: [{ tool_call_id: "cacheguard_1", accepted: true }],
    });

    fetchMock.mockClear();
    fireEvent.click(screen.getByRole("button", { name: /stop/i }));
    await waitFor(() => expect(fetchMock).toHaveBeenCalled());
    expect(lastCommandBody(fetchMock)).toMatchObject({
      type: "tool_decisions",
      decisions: [{ tool_call_id: "cacheguard_1", accepted: false }],
    });
  });

  it("parses patch filenames and sends Allow Once, Allow for This Chat, and Stop callbacks", async () => {
    const fetchMock = vi.fn().mockResolvedValue({ ok: true });
    vi.stubGlobal("fetch", fetchMock);

    renderToolConfirmation(patchReasons, [
      patchToolCall("patch-1", "src/components/Foo.tsx"),
      patchToolCall("patch-2", "src/components/nested/Bar.module.css"),
    ]);

    expect(screen.getByText("Model wants to apply changes:")).toBeInTheDocument();
    expect(screen.getByText(/Patch/u)).toBeInTheDocument();
    expect(screen.getByText("Foo.tsx")).toBeInTheDocument();
    expect(screen.getByText("Bar.module.css")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: /allow once/i }));
    await waitFor(() => expect(fetchMock).toHaveBeenCalled());
    expect(lastCommandBody(fetchMock)).toMatchObject({
      type: "tool_decisions",
      decisions: [
        { tool_call_id: "patch-1", accepted: true },
        { tool_call_id: "patch-2", accepted: true },
      ],
    });

    fetchMock.mockClear();
    fireEvent.click(screen.getByRole("button", { name: /allow for this chat/i }));
    await waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(2));
    const bodies = commandBodies(fetchMock);
    expect(bodies[0]).toMatchObject({
      type: "set_params",
      patch: { auto_approve_editing_tools: true },
    });
    expect(bodies[1]).toMatchObject({
      type: "tool_decisions",
      decisions: [
        { tool_call_id: "patch-1", accepted: true },
        { tool_call_id: "patch-2", accepted: true },
      ],
    });

    fetchMock.mockClear();
    fireEvent.click(screen.getByRole("button", { name: /stop/i }));
    await waitFor(() => expect(fetchMock).toHaveBeenCalled());
    expect(lastCommandBody(fetchMock)).toMatchObject({
      type: "tool_decisions",
      decisions: [
        { tool_call_id: "patch-1", accepted: false },
        { tool_call_id: "patch-2", accepted: false },
      ],
    });
  });

  it("dispatches integration configuration page link from generic confirmation", () => {
    const integrationPath =
      "\\\\?\\d:\\work\\refact.ai\\refact-lsp\\.refact\\integrations\\postgres.yaml";
    const { store } = renderToolConfirmation([
      {
        type: "confirmation",
        tool_name: "postgres",
        tool_call_id: "sql-1",
        command: "SELECT *",
        rule: "*",
        integr_config_path: integrationPath,
      },
    ]);

    fireEvent.click(screen.getByText("Configuration Page"));

    expect(store.getState().pages.at(-1)).toEqual({
      name: "integrations page",
      integrationPath,
      wasOpenedThroughChat: true,
    });
  });
});
