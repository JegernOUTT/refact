import React from "react";
import { describe, expect, test, beforeEach, vi } from "vitest";
import { http, HttpResponse } from "msw";
import { screen, waitFor } from "../../utils/test-utils";
import { render } from "../../utils/test-utils";
import { server } from "../../utils/mockServer";
import { createDefaultChatState } from "../../utils/test-utils";
import { ModeSelect } from "./ModeSelect";
import type { ChatModeInfo } from "../../services/refact/chatModes";

const modeDefaults = {
  include_project_info: true,
  checkpoints_enabled: true,
  auto_approve_editing_tools: false,
  auto_approve_dangerous_commands: false,
};

const modes: ChatModeInfo[] = [
  {
    id: "agent",
    title: "Agent",
    description: "Autonomous coding mode",
    tools_count: 12,
    thread_defaults: modeDefaults,
    ui: { order: 1, tags: ["editing", "tools"] },
  },
  {
    id: "ask",
    title: "Ask",
    description: "Quick answers without edits",
    tools_count: 0,
    thread_defaults: { ...modeDefaults, checkpoints_enabled: false },
    ui: { order: 2, tags: ["chat"] },
  },
];

const config = {
  apiKey: "test",
  host: "web" as const,
  dev: true,
  themeProps: {},
  lspPort: 8001,
};

function useChatModes(modesResponse = modes) {
  server.use(
    http.get("*/v1/chat-modes", () =>
      HttpResponse.json({ modes: modesResponse, errors: [] }),
    ),
  );
}

function renderModeSelect(
  ui: React.ReactElement,
  chat = createDefaultChatState(),
) {
  return render(ui, {
    preloadedState: {
      chat,
      config,
    },
  });
}

function chatStateWithMessages() {
  const chat = createDefaultChatState();
  const threadId = chat.current_thread_id;
  const runtime = chat.threads[threadId];
  runtime.thread.mode = "agent";
  runtime.thread.messages = [
    { role: "user", content: "hello", message_id: "user-1" },
  ];
  return chat;
}

describe("ModeSelect", () => {
  beforeEach(() => {
    useChatModes();
  });

  test("shows selected mode title and tool count in the trigger", async () => {
    renderModeSelect(
      <ModeSelect selectedMode="agent" onModeChange={vi.fn()} />,
    );

    await waitFor(() => expect(screen.getByText("Agent")).toBeInTheDocument());
    expect(screen.getByText("12 tools")).toBeInTheDocument();
  });

  test("lists modes with descriptions, tags, tool counts, synthetic task planner, and create action", async () => {
    const { user } = renderModeSelect(
      <ModeSelect selectedMode="agent" onModeChange={vi.fn()} />,
    );

    await user.click(await screen.findByRole("button", { name: /Agent/ }));

    expect(screen.getByText("Autonomous coding mode")).toBeInTheDocument();
    expect(screen.getByText("Quick answers without edits")).toBeInTheDocument();
    expect(screen.getByText("editing")).toBeInTheDocument();
    expect(screen.getAllByText("12 tools").length).toBeGreaterThan(0);
    expect(screen.getByText("Task Planner")).toBeInTheDocument();
    expect(screen.getByText("Create new mode...")).toBeInTheDocument();
  });

  test("selecting a mode before chat starts applies mode and thread defaults directly", async () => {
    const onModeChange = vi.fn();
    const { user } = renderModeSelect(
      <ModeSelect selectedMode="agent" onModeChange={onModeChange} />,
    );

    await user.click(await screen.findByRole("button", { name: /Agent/ }));
    await user.click(screen.getByRole("button", { name: /Ask/ }));

    expect(onModeChange).toHaveBeenCalledWith("ask", {
      ...modeDefaults,
      checkpoints_enabled: false,
    });
  });

  test("selecting a non-task mode after messages opens the mode transition dialog", async () => {
    const onModeChange = vi.fn();
    const { user } = renderModeSelect(
      <ModeSelect selectedMode="agent" onModeChange={onModeChange} />,
      chatStateWithMessages(),
    );

    await user.click(await screen.findByRole("button", { name: /Agent/ }));
    await user.click(screen.getByRole("button", { name: /Ask/ }));

    expect(onModeChange).not.toHaveBeenCalled();
    expect(
      await screen.findByRole("button", { name: "Switch Mode" }),
    ).toBeInTheDocument();
  });

  test("Create new mode navigates to customization modes page", async () => {
    const { user, store } = renderModeSelect(
      <ModeSelect selectedMode="agent" onModeChange={vi.fn()} />,
    );

    await user.click(await screen.findByRole("button", { name: /Agent/ }));
    await user.click(screen.getByText("Create new mode..."));

    expect(store.getState().pages.at(-1)).toEqual({
      name: "customization",
      kind: "modes",
    });
  });
});
