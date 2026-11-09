import React from "react";
import { beforeEach, describe, expect, test, vi } from "vitest";
import { http, HttpResponse } from "msw";
import { act } from "react-dom/test-utils";
import { screen } from "@testing-library/react";

import {
  render,
  waitFor,
  createDefaultChatState,
} from "../../utils/test-utils";
import { ChatForm } from "./ChatForm";
import { useChatActions } from "../../hooks/useChatActions";
import {
  appContextRemoved,
  appContextStaged,
  type StagedAppContext,
} from "../../features/Apps/appsSlice";
import {
  server,
  goodCaps,
  goodPrompts,
  noTools,
  noCommandPreview,
  noCompletions,
  goodPing,
  goodUser,
  emptyTrajectories,
  trajectorySave,
} from "../../utils/mockServer";

const commandMocks = vi.hoisted(() => {
  vi.resetModules();
  return {
    sendUserMessage: vi.fn<(...args: unknown[]) => Promise<void>>(() =>
      Promise.resolve(),
    ),
    updateChatParams: vi.fn(() => Promise.resolve()),
  };
});

vi.mock("../../services/refact/chatCommands", async (importOriginal) => {
  const original =
    await importOriginal<typeof import("../../services/refact/chatCommands")>();
  return {
    ...original,
    sendUserMessage: commandMocks.sendUserMessage,
    updateChatParams: commandMocks.updateChatParams,
  };
});

const handlers = [
  goodCaps,
  goodUser,
  goodPrompts,
  noTools,
  noCommandPreview,
  noCompletions,
  goodPing,
  emptyTrajectories,
  trajectorySave,
  http.get("*/v1/chat-modes", () =>
    HttpResponse.json({ modes: [], errors: [] }),
  ),
  http.post("*/v1/chats/:id/commands", () =>
    HttpResponse.json({ status: "queued" }),
  ),
  http.get("*/v1/providers/:provider/quota", () =>
    HttpResponse.json({ available: false }),
  ),
  http.get("*/v1/voice/status", () => HttpResponse.json({ available: false })),
  http.get("*/v1/worktrees", () =>
    HttpResponse.json({
      project_hash: "test",
      source_workspace_root: "/tmp/refact-test",
      worktrees: [],
    }),
  ),
  http.get("*/v1/chats/:id/skills-status", () =>
    HttpResponse.json({
      skills_available: 0,
      skills_included: [],
      skills_enabled: false,
      active_skill: null,
    }),
  ),
];

const engineConfigState = {
  config: { host: "vscode" as const, themeProps: {}, lspPort: 8001 },
};

function chatState() {
  const chat = createDefaultChatState();
  const id = chat.current_thread_id;
  chat.threads[id].thread = {
    ...chat.threads[id].thread,
    model: "openai/gpt-4o",
  };
  return chat;
}

const Harness: React.FC = () => {
  const { submit } = useChatActions();
  return (
    <ChatForm
      onSubmit={(value, policy) => {
        void submit(value, policy === "immediate").catch(() => undefined);
      }}
    />
  );
};

const TICKET = "0b9a3f5e-1c2d-4e6f-8a7b-9c0d1e2f3a4b";
const LATE_TICKET = "0b9a3f5e-1c2d-4e6f-8a7b-9c0d1e2f3a4c";
const FOREIGN_TICKET = "0b9a3f5e-1c2d-4e6f-8a7b-9c0d1e2f3a4d";

function context(patch: Partial<StagedAppContext> = {}): StagedAppContext {
  return {
    id: TICKET,
    app_id: "board",
    binding_id: "binding-1",
    revision: "r1",
    title: "Selected card",
    text: "Card body",
    ...patch,
  };
}

function sentContent(call: number): string {
  const content = commandMocks.sendUserMessage.mock.calls[call]?.[1];
  if (typeof content !== "string") throw new Error("expected text content");
  return content;
}

function sentIds(call: number): unknown {
  return commandMocks.sendUserMessage.mock.calls[call]?.[7];
}

describe("ChatForm app context staging", () => {
  beforeEach(() => {
    server.use(...handlers);
    commandMocks.sendUserMessage.mockReset();
    commandMocks.sendUserMessage.mockResolvedValue(undefined);
  });

  test("shows a chip with preview, preserves the draft, and sends a marked block", async () => {
    const { user, store } = render(<Harness />, {
      preloadedState: { chat: chatState(), ...engineConfigState },
    });
    const id = store.getState().chat.current_thread_id;
    const textarea = screen.getByTestId("chat-form-textarea");
    await user.type(textarea, "my draft");

    act(() => {
      store.dispatch(appContextStaged({ chatId: id, context: context() }));
    });
    expect(textarea).toHaveValue("my draft");

    const chip = await screen.findByRole("button", {
      name: "Preview app context: Selected card",
    });
    await user.click(chip);
    expect(await screen.findByText("Card body")).toBeInTheDocument();
    await user.keyboard("{Escape}");

    await user.click(screen.getByLabelText("Send message"));
    await waitFor(() =>
      expect(commandMocks.sendUserMessage).toHaveBeenCalledTimes(1),
    );
    const text = sentContent(0);
    expect(text.trim()).toBe("my draft");
    expect(text).not.toContain("Card body");
    expect(sentIds(0)).toEqual([TICKET]);

    await waitFor(() =>
      expect(store.getState().apps.stagedByChat[id]).toEqual([]),
    );
    await waitFor(() =>
      expect(screen.getByTestId("chat-form-textarea")).toHaveValue(""),
    );
  });

  test("a failed send retains the chip and the same snapshot revision", async () => {
    commandMocks.sendUserMessage.mockRejectedValueOnce(new Error("offline"));
    const { user, store } = render(<Harness />, {
      preloadedState: { chat: chatState(), ...engineConfigState },
    });
    const id = store.getState().chat.current_thread_id;
    act(() => {
      store.dispatch(appContextStaged({ chatId: id, context: context() }));
    });

    await user.type(screen.getByTestId("chat-form-textarea"), "try{Enter}");
    await waitFor(() =>
      expect(commandMocks.sendUserMessage).toHaveBeenCalledTimes(1),
    );
    expect(store.getState().apps.stagedByChat[id]).toEqual([context()]);
    expect(
      screen.getByRole("button", {
        name: "Preview app context: Selected card",
      }),
    ).toBeInTheDocument();

    await user.type(screen.getByTestId("chat-form-textarea"), "again{Enter}");
    await waitFor(() =>
      expect(commandMocks.sendUserMessage).toHaveBeenCalledTimes(2),
    );
    expect(sentIds(1)).toEqual([TICKET]);
    expect(sentContent(1)).not.toContain("Card body");
    await waitFor(() =>
      expect(store.getState().apps.stagedByChat[id]).toEqual([]),
    );
  });

  test("restaging a newer revision during the send keeps the new chip", async () => {
    let release: () => void = () => undefined;
    commandMocks.sendUserMessage.mockImplementationOnce(
      () =>
        new Promise<void>((resolve) => {
          release = resolve;
        }),
    );
    const { user, store } = render(<Harness />, {
      preloadedState: { chat: chatState(), ...engineConfigState },
    });
    const id = store.getState().chat.current_thread_id;
    act(() => {
      store.dispatch(appContextStaged({ chatId: id, context: context() }));
    });
    await user.type(screen.getByTestId("chat-form-textarea"), "go{Enter}");
    await waitFor(() =>
      expect(commandMocks.sendUserMessage).toHaveBeenCalledTimes(1),
    );

    act(() => {
      store.dispatch(
        appContextStaged({
          chatId: id,
          context: context({ revision: "r2", text: "Newer body" }),
        }),
      );
      store.dispatch(
        appContextStaged({
          chatId: id,
          context: context({ id: LATE_TICKET, text: "Late" }),
        }),
      );
    });
    await act(async () => {
      release();
      await Promise.resolve();
    });

    await waitFor(() =>
      expect(
        store.getState().apps.stagedByChat[id]?.map((item) => item.id),
      ).toEqual([TICKET, LATE_TICKET]),
    );
    expect(store.getState().apps.stagedByChat[id]?.[0]?.revision).toBe("r2");
  });

  test("a chip in another chat is isolated and removable chips are not sent", async () => {
    const { user, store } = render(<Harness />, {
      preloadedState: { chat: chatState(), ...engineConfigState },
    });
    const id = store.getState().chat.current_thread_id;
    act(() => {
      store.dispatch(
        appContextStaged({
          chatId: "other-chat",
          context: context({ id: FOREIGN_TICKET }),
        }),
      );
      store.dispatch(appContextStaged({ chatId: id, context: context() }));
    });
    expect(
      screen.queryByRole("button", { name: /Preview app context/ }),
    ).toBeInTheDocument();

    await user.click(
      screen.getByRole("button", { name: "Remove app context: Selected card" }),
    );
    expect(store.getState().apps.stagedByChat[id]).toEqual([]);
    expect(store.getState().apps.stagedByChat["other-chat"]).toHaveLength(1);

    await user.type(screen.getByTestId("chat-form-textarea"), "plain{Enter}");
    await waitFor(() =>
      expect(commandMocks.sendUserMessage).toHaveBeenCalledTimes(1),
    );
    expect(sentIds(0)).toBeUndefined();
    expect(store.getState().apps.stagedByChat["other-chat"]).toHaveLength(1);
    store.dispatch(
      appContextRemoved({ chatId: "other-chat", id: FOREIGN_TICKET }),
    );
  });

  test("staged context lets an otherwise empty composer send", async () => {
    const { user, store } = render(<Harness />, {
      preloadedState: { chat: chatState(), ...engineConfigState },
    });
    const id = store.getState().chat.current_thread_id;
    act(() => {
      store.dispatch(appContextStaged({ chatId: id, context: context() }));
    });
    await user.click(await screen.findByLabelText("Send message"));
    await waitFor(() =>
      expect(commandMocks.sendUserMessage).toHaveBeenCalledTimes(1),
    );
    expect(sentContent(0).trim()).toBe("");
    expect(sentIds(0)).toEqual([TICKET]);
  });
});
