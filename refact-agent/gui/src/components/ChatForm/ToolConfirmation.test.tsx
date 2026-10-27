import { beforeEach, describe, expect, it, vi } from "vitest";
import { act } from "react-dom/test-utils";
import { fireEvent, render, screen, waitFor } from "../../utils/test-utils";
import type {
  ChatMessages,
  ToolCall,
  ToolConfirmationPauseReason,
} from "../../services/refact";
import { ToolConfirmation } from "./ToolConfirmation";
import { createDefaultChatState } from "../../utils/test-utils";
import type {
  ChatCommand,
  ChatCommandBase,
} from "../../services/refact/chatCommands";
import { applyChatEvent } from "../../features/Chat/Thread";
import { selectRuntimeById } from "../../features/Chat/Thread/selectors";
import type { AppStore } from "../../app/store";

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
  runtime.confirmation.pause = true;
  runtime.confirmation.pause_reasons = pauseReasons;
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

function emitAck(
  store: AppStore,
  fetchMock: ReturnType<typeof vi.fn>,
  accepted: boolean,
  result: unknown = { queued: true },
) {
  const chatId = store.getState().chat.current_thread_id;
  const command = lastCommandBody(fetchMock) as ChatCommand;
  act(() => {
    store.dispatch(
      applyChatEvent({
        chat_id: chatId,
        seq: String(
          Number(
            selectRuntimeById(store.getState(), chatId)?.last_applied_seq ?? 0,
          ) + 1,
        ),
        type: "ack",
        client_request_id: command.client_request_id,
        accepted,
        result,
      }),
    );
  });
}

function resetConfirmation(
  store: AppStore,
  reasons: ToolConfirmationPauseReason[],
) {
  const chatId = store.getState().chat.current_thread_id;
  act(() => {
    store.dispatch(
      applyChatEvent({
        chat_id: chatId,
        seq: String(
          Number(
            selectRuntimeById(store.getState(), chatId)?.last_applied_seq ?? 0,
          ) + 1,
        ),
        type: "pause_cleared",
      }),
    );
  });
  act(() => {
    store.dispatch(
      applyChatEvent({
        chat_id: chatId,
        seq: String(
          Number(
            selectRuntimeById(store.getState(), chatId)?.last_applied_seq ?? 0,
          ) + 1,
        ),
        type: "pause_required",
        reasons,
      }),
    );
  });
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

    const { store } = renderToolConfirmation(pauseReasons);

    expect(screen.getByText("Prompt cache may be broken")).toBeInTheDocument();
    expect(screen.getByText("$0.42 USD")).toBeInTheDocument();
    expect(screen.getByText(/- cached prefix/u)).toBeInTheDocument();
    expect(screen.getByText(/\+ changed prefix/u)).toBeInTheDocument();

    fireEvent.click(
      screen.getByRole("button", { name: /force and continue/i }),
    );
    await waitFor(() => expect(fetchMock).toHaveBeenCalled());
    expect(lastCommandBody(fetchMock)).toMatchObject({
      type: "tool_decisions",
      decisions: [{ tool_call_id: "cacheguard_1", accepted: true }],
    });

    emitAck(store, fetchMock, true);
    resetConfirmation(store, pauseReasons);
    await waitFor(() =>
      expect(screen.getByRole("button", { name: /stop/i })).toBeEnabled(),
    );
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

    const { store } = renderToolConfirmation(patchReasons, [
      patchToolCall("patch-1", "src/components/Foo.tsx"),
      patchToolCall("patch-2", "src/components/nested/Bar.module.css"),
    ]);

    expect(
      screen.getByText("Model wants to apply changes:"),
    ).toBeInTheDocument();
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

    emitAck(store, fetchMock, true);
    resetConfirmation(store, patchReasons);
    await waitFor(() =>
      expect(screen.getByRole("button", { name: /allow once/i })).toBeEnabled(),
    );
    fetchMock.mockClear();
    fireEvent.click(
      screen.getByRole("button", { name: /allow for this chat/i }),
    );
    await waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(1));
    expect(lastCommandBody(fetchMock)).toMatchObject({ type: "set_params" });
    act(() => {
      const chatId = store.getState().chat.current_thread_id;
      store.dispatch(
        applyChatEvent({
          chat_id: chatId,
          seq: String(
            Number(
              selectRuntimeById(store.getState(), chatId)?.last_applied_seq ??
                0,
            ) + 1,
          ),
          type: "thread_updated",
          auto_approve_editing_tools: true,
        }),
      );
    });
    emitAck(store, fetchMock, true, { applied: true });
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

    emitAck(store, fetchMock, true);
    resetConfirmation(store, patchReasons);
    await waitFor(() =>
      expect(screen.getByRole("button", { name: /stop/i })).toBeEnabled(),
    );
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
  it("keeps Allow Once pending until its own ACK and shows a later rejection", async () => {
    const fetchMock = vi.fn().mockResolvedValue({ ok: true });
    vi.stubGlobal("fetch", fetchMock);
    const { store } = renderToolConfirmation(patchReasons);
    fireEvent.click(screen.getByRole("button", { name: "Allow Once" }));
    fireEvent.click(screen.getByRole("button", { name: "Allow Once" }));
    await waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(1));
    expect(screen.getByRole("button", { name: "Allow Once" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Stop" })).toBeDisabled();
    expect(
      screen.getByRole("button", { name: "Allow for This Chat" }),
    ).toBeDisabled();
    emitAck(store, fetchMock, true);
    await waitFor(() =>
      expect(screen.getByRole("status")).toHaveTextContent("accepted"),
    );
    expect(screen.getByRole("button", { name: "Allow Once" })).toBeDisabled();
    emitAck(store, fetchMock, false, {
      error: "Approval changed; refresh before responding",
    });
    await waitFor(() =>
      expect(screen.getByRole("alert")).toHaveTextContent("Approval changed"),
    );
    expect(screen.getByRole("button", { name: "Allow Once" })).toBeEnabled();
    expect(fetchMock).toHaveBeenCalledTimes(1);
  });

  it("preserves Stop on HTTP rejection and sends only rejected decisions", async () => {
    const fetchMock = vi.fn().mockResolvedValue({
      ok: false,
      status: 403,
      statusText: "Forbidden",
      text: () => Promise.resolve("Authority refused"),
    });
    vi.stubGlobal("fetch", fetchMock);
    renderToolConfirmation(patchReasons);
    fireEvent.click(screen.getByRole("button", { name: "Stop" }));
    await waitFor(() =>
      expect(screen.getByRole("alert")).toHaveTextContent("Authority refused"),
    );
    expect(lastCommandBody(fetchMock)).toMatchObject({
      type: "tool_decisions",
      decisions: [
        { tool_call_id: "patch-1", accepted: false },
        { tool_call_id: "patch-2", accepted: false },
      ],
    });
    expect(screen.getByRole("button", { name: "Stop" })).toBeEnabled();
    expect(fetchMock).toHaveBeenCalledTimes(1);
  });

  it("separates setting rejection from the tool decision and cancels chaining on source replacement", async () => {
    const fetchMock = vi.fn().mockResolvedValue({ ok: true });
    vi.stubGlobal("fetch", fetchMock);
    const { store, rerender } = renderToolConfirmation(patchReasons);
    fireEvent.click(
      screen.getByRole("button", { name: "Allow for This Chat" }),
    );
    await waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(1));
    emitAck(store, fetchMock, false, { error: "Setting refused" });
    await waitFor(() =>
      expect(screen.getByRole("alert")).toHaveTextContent("Setting refused"),
    );
    expect(screen.getByText(/no tool decision was sent/)).toBeInTheDocument();
    expect(fetchMock).toHaveBeenCalledTimes(1);
    fireEvent.click(
      screen.getByRole("button", { name: "Allow for This Chat" }),
    );
    await waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(2));
    rerender(
      <ToolConfirmation
        pauseReasons={[{ ...patchReasons[0], tool_call_id: "replacement" }]}
      />,
    );
    emitAck(store, fetchMock, true, { applied: true });
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
    expect(fetchMock).toHaveBeenCalledTimes(2);
  });

  it("reports an applied chat setting separately when its actual tool decision fails", async () => {
    const fetchMock = vi.fn().mockResolvedValue({ ok: true });
    vi.stubGlobal("fetch", fetchMock);
    const { store } = renderToolConfirmation(patchReasons);
    fireEvent.click(
      screen.getByRole("button", { name: "Allow for This Chat" }),
    );
    await waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(1));
    act(() => {
      const chatId = store.getState().chat.current_thread_id;
      store.dispatch(
        applyChatEvent({
          chat_id: chatId,
          seq: "1",
          type: "thread_updated",
          auto_approve_editing_tools: true,
        }),
      );
    });
    emitAck(store, fetchMock, true, { applied: true });
    await waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(2));
    emitAck(store, fetchMock, false, { error: "Tool decision refused" });
    await waitFor(() =>
      expect(screen.getByRole("alert")).toHaveTextContent(
        "Tool decision refused",
      ),
    );
    expect(
      screen.getByText(/Chat editing approval is enabled/),
    ).toBeInTheDocument();
    expect(commandBodies(fetchMock).map((command) => command.type)).toEqual([
      "set_params",
      "tool_decisions",
    ]);
  });
});
