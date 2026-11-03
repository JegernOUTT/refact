import React from "react";
import { readFile } from "node:fs/promises";
import { resolve } from "node:path";
import { beforeEach, describe, expect, onTestFinished, test, vi } from "vitest";
import { http, HttpResponse } from "msw";
import { act } from "react-dom/test-utils";
import { fireEvent, screen } from "@testing-library/react";

import { render, waitFor } from "../../utils/test-utils";
import * as voiceInput from "../../hooks/useVoiceInput";
import { ChatForm, ChatFormProps } from "./ChatForm";
import { createDefaultChatState } from "../../utils/test-utils";
import { applyChatEvent } from "../../features/Chat/Thread";
import type {
  ChatMessage,
  ChatMessages,
  GoalSnapshot,
} from "../../services/refact/types";
import type { ChatEventEnvelope } from "../../services/refact/chatSubscription";

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

const modeDefaults = {
  include_project_info: true,
  checkpoints_enabled: true,
  auto_approve_editing_tools: false,
  auto_approve_dangerous_commands: false,
};

const goodChatModes = http.get("*/v1/chat-modes", () =>
  HttpResponse.json({
    modes: [
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
    ],
    errors: [],
  }),
);

const noVoiceStatus = http.get("*/v1/voice/status", () =>
  HttpResponse.json({ available: false }),
);

const noWorktrees = http.get("*/v1/worktrees", () =>
  HttpResponse.json({
    project_hash: "test",
    source_workspace_root: "/tmp/refact-test",
    worktrees: [],
  }),
);

const queuedChatCommand = http.post("*/v1/chats/:id/commands", () =>
  HttpResponse.json({ status: "queued" }),
);

const noSkills = http.get("*/v1/chats/:id/skills-status", () =>
  HttpResponse.json({
    skills_available: 0,
    skills_included: [],
    skills_enabled: false,
    active_skill: null,
  }),
);

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
  goodChatModes,
  noVoiceStatus,
  noWorktrees,
  queuedChatCommand,
  noSkills,
];

const engineConfigState = {
  config: { host: "vscode" as const, themeProps: {}, lspPort: 8001 },
};

function chatStateWithThread(
  patch: Partial<
    ReturnType<typeof createDefaultChatState>["threads"][string]["thread"]
  >,
) {
  const chat = createDefaultChatState();
  const threadId = chat.current_thread_id;
  chat.threads[threadId].thread = {
    ...chat.threads[threadId].thread,
    ...patch,
  };
  return chat;
}

function pasteFile(
  textarea: HTMLTextAreaElement,
  file: File,
): ReturnType<typeof fireEvent.paste> {
  const item = {
    kind: "file",
    getAsFile: () => file,
  };
  return fireEvent.paste(textarea, {
    clipboardData: {
      items: [item],
      files: [file],
    },
  });
}

function rebuiltReport(payloadMessages: ChatMessages): ChatMessage {
  return {
    role: "compression_report",
    content: "Context rebuilt",
    message_id: "report-1",
    compression_report: {
      kind: "reconstructed_history",
      schema_version: 1,
      payload: { messages: payloadMessages },
    },
  } as unknown as ChatMessage;
}

const blockedContextMessages: ChatMessages = [
  { role: "user", content: "original", message_id: "orig-1" },
  rebuiltReport([
    { role: "user", content: "one", message_id: "dup" },
    { role: "assistant", content: "two", message_id: "dup" },
  ]),
];

const validRebuiltMessages: ChatMessages = [
  { role: "user", content: "original", message_id: "orig-1" },
  rebuiltReport([
    { role: "user", content: "one", message_id: "p-1" },
    { role: "assistant", content: "two", message_id: "p-2" },
  ]),
];

function companionMeta(sourceId: string) {
  return {
    is_buddy_chat: true,
    buddy_chat_kind: "companion",
    workflow_id: null,
    source_chat_id: sourceId,
  };
}

function rebuiltSnapshot(
  chatId: string,
  messages: ChatMessages,
): ChatEventEnvelope {
  return rebuiltSnapshotWithMeta(chatId, messages);
}

function rebuiltSnapshotWithMeta(
  chatId: string,
  messages: ChatMessages,
  buddyMeta?: ReturnType<typeof companionMeta>,
): ChatEventEnvelope {
  return {
    chat_id: chatId,
    seq: "0",
    type: "snapshot",
    thread: {
      id: chatId,
      title: "Rebuilt",
      model: "openai/gpt-4o",
      mode: "AGENT",
      tool_use: "agent",
      boost_reasoning: false,
      context_tokens_cap: null,
      include_project_info: true,
      checkpoints_enabled: true,
      is_title_generated: false,
      ...(buddyMeta ? { buddy_meta: buddyMeta } : {}),
    },
    runtime: {
      state: "idle",
      paused: false,
      error: null,
      queue_size: 0,
      pause_reasons: [],
      queued_items: [],
    },
    messages,
    background_agents: [],
  };
}

const pausedGoal: GoalSnapshot = {
  owner: { kind: "task", id: "task-1" },
  content: "Finish the plan",
  version: 1,
  active: false,
  status: "paused",
  budget: { cooldown_ms: 0, no_progress_token_threshold: 0 },
  progress: {
    turns_used: 0,
    tokens_used: 0,
    started_at_ms: 0,
    no_progress_turns: 0,
    last_nudge_at_ms: 0,
  },
  attempts: [],
  events: [],
};

server.use(...handlers);

const App: React.FC<Partial<ChatFormProps>> = ({ ...props }) => {
  const defaultProps: ChatFormProps = {
    onSubmit: (_str: string) => undefined,
    ...props,
  };

  return <ChatForm {...defaultProps} />;
};

describe("ChatForm", () => {
  beforeEach(() => {
    server.use(...handlers);
  });

  test("companion composers reject the voice shortcut while regular composers retain it", async () => {
    const toggleRecording = vi
      .fn<() => Promise<string | null>>()
      .mockResolvedValue(null);
    const idleVoice: ReturnType<typeof voiceInput.useVoiceInput> = {
      isRecording: false,
      isFinishing: false,
      isVoiceActive: false,
      isDownloading: false,
      downloadProgress: 0,
      error: null,
      voiceEnabled: true,
      modelLoaded: true,
      liveTranscript: "",
      toggleRecording,
      cancelRecording: vi.fn(),
    };
    const voice = vi
      .spyOn(voiceInput, "useVoiceInput")
      .mockReturnValue(idleVoice);
    const app = render(<App />, {
      preloadedState: {
        chat: chatStateWithThread({ model: "openai/gpt-4o" }),
        ...engineConfigState,
      },
    });
    const id = app.store.getState().chat.current_thread_id;
    const lock = () => {
      act(() => {
        app.store.dispatch(
          applyChatEvent(
            rebuiltSnapshotWithMeta(id, [], companionMeta("source-a")),
          ),
        );
      });
    };
    try {
      await waitFor(() =>
        expect(screen.getByLabelText("Send message")).toBeEnabled(),
      );
      const textarea = screen.getByTestId("chat-form-textarea");
      textarea.focus();
      fireEvent.keyDown(textarea, {
        code: "Space",
        ctrlKey: true,
        shiftKey: true,
      });
      expect(toggleRecording).toHaveBeenCalledTimes(1);
      lock();
      textarea.focus();
      expect(screen.getByLabelText("Voice input")).toBeDisabled();
      fireEvent.keyDown(textarea, {
        code: "Space",
        ctrlKey: true,
        shiftKey: true,
      });
      expect(toggleRecording).toHaveBeenCalledTimes(1);
      voice.mockReturnValue({
        ...idleVoice,
        isRecording: true,
        isVoiceActive: true,
      });
      app.rerender(<App />);
      await waitFor(() =>
        expect(screen.getByLabelText("Voice input")).toBeEnabled(),
      );
      expect(
        screen
          .getByLabelText("Voice input")
          .closest('[data-composer-control="mic"]'),
      ).toHaveAttribute("data-hidden", "false");
      fireEvent.keyDown(textarea, {
        code: "Space",
        ctrlKey: true,
        shiftKey: true,
      });
      expect(toggleRecording).toHaveBeenCalledTimes(2);
    } finally {
      app.unmount();
      voice.mockRestore();
    }
  });

  test("Buddy chats hide model and mode pickers instead of disabling them", async () => {
    const width = vi
      .spyOn(HTMLElement.prototype, "clientWidth", "get")
      .mockImplementation(() => 4000);
    const rect = vi
      .spyOn(HTMLElement.prototype, "getBoundingClientRect")
      .mockImplementation(() => new DOMRect(0, 0, 40, 28));
    const hidden = (
      container: HTMLElement,
      id: "model" | "mode",
    ): string | undefined =>
      container.querySelector<HTMLElement>(`[data-composer-control="${id}"]`)
        ?.dataset.hidden;
    try {
      const regular = render(<App />, {
        preloadedState: {
          chat: chatStateWithThread({ model: "openai/gpt-4o" }),
          ...engineConfigState,
        },
      });
      await waitFor(() =>
        expect(hidden(regular.container, "model")).toBe("false"),
      );
      expect(hidden(regular.container, "mode")).toBe("false");
      regular.unmount();

      const buddy = render(<App />, {
        preloadedState: {
          chat: chatStateWithThread({
            model: "openai/gpt-4o",
            buddy_meta: companionMeta("source-a"),
          }),
          ...engineConfigState,
        },
      });
      await waitFor(() =>
        expect(hidden(buddy.container, "model")).toBe("true"),
      );
      expect(hidden(buddy.container, "mode")).toBe("true");
    } finally {
      width.mockRestore();
      rect.mockRestore();
    }
  });

  test("Buddy chats hide permission controls in the expanded composer", async () => {
    const width = vi
      .spyOn(HTMLElement.prototype, "clientWidth", "get")
      .mockImplementation(() => 4000);
    const rect = vi
      .spyOn(HTMLElement.prototype, "getBoundingClientRect")
      .mockImplementation(() => new DOMRect(0, 0, 40, 28));
    const editApproval = (container: HTMLElement) =>
      container.querySelector<HTMLElement>(
        '[data-composer-control="editApproval"]',
      )?.dataset.hidden;
    try {
      const regular = render(<App />, {
        preloadedState: {
          chat: chatStateWithThread({ model: "openai/gpt-4o" }),
          ...engineConfigState,
        },
      });
      await regular.user.click(
        await regular.findByTestId("chat-form-textarea"),
      );
      await waitFor(() =>
        expect(editApproval(regular.container)).toBe("false"),
      );
      regular.unmount();

      const buddy = render(<App />, {
        preloadedState: {
          chat: chatStateWithThread({
            model: "openai/gpt-4o",
            buddy_meta: companionMeta("source-a"),
          }),
          ...engineConfigState,
        },
      });
      await buddy.user.click(await buddy.findByTestId("chat-form-textarea"));
      await waitFor(() => expect(editApproval(buddy.container)).toBe("true"));
    } finally {
      width.mockRestore();
      rect.mockRestore();
    }
  });

  test("companion chats block file paste and hide attachment controls", async () => {
    const { user, ...app } = render(<App />, {
      preloadedState: {
        chat: chatStateWithThread({
          model: "openai/gpt-4o",
          buddy_meta: companionMeta("source-a"),
        }),
        ...engineConfigState,
      },
    });
    const textarea =
      screen.getByTestId<HTMLTextAreaElement>("chat-form-textarea");
    await user.click(textarea);
    expect(
      app.container
        .querySelector('[data-composer-control="image"]')
        ?.getAttribute("data-hidden"),
    ).not.toBe("false");
    expect(
      pasteFile(
        textarea,
        new File(["image"], "diagram.png", { type: "image/png" }),
      ),
    ).toBe(false);
    expect(
      pasteFile(textarea, new File(["notes"], "notes.md", { type: "text/md" })),
    ).toBe(false);
    const thread =
      app.store.getState().chat.threads[
        app.store.getState().chat.current_thread_id
      ];
    expect(thread?.attached_images).toHaveLength(0);
    expect(thread?.attached_text_files).toHaveLength(0);
  });

  test("a submit clears submitted text attachments", async () => {
    const onSubmit = vi.fn();
    const { user, store } = render(<App onSubmit={onSubmit} />, {
      preloadedState: {
        chat: chatStateWithThread({ model: "openai/gpt-4o" }),
        ...engineConfigState,
      },
    });
    const id = store.getState().chat.current_thread_id;
    const textarea =
      screen.getByTestId<HTMLTextAreaElement>("chat-form-textarea");
    pasteFile(
      textarea,
      new File(["attachment"], "notes.md", { type: "text/markdown" }),
    );
    await waitFor(() =>
      expect(
        store.getState().chat.threads[id]?.attached_text_files,
      ).toHaveLength(1),
    );
    await user.type(textarea, "first{Enter}");
    expect(onSubmit).toHaveBeenCalledTimes(1);
    await waitFor(() =>
      expect(
        store.getState().chat.threads[id]?.attached_text_files,
      ).toHaveLength(0),
    );
    expect(screen.getByTestId("chat-form-textarea")).toHaveValue("");
  });

  test("when I push enter it should call onSubmit", async () => {
    const fakeOnSubmit = vi.fn();

    const { user, ...app } = render(<App onSubmit={fakeOnSubmit} />, {
      preloadedState: engineConfigState,
    });

    const textarea: HTMLTextAreaElement | null =
      app.container.querySelector("textarea");
    expect(textarea).not.toBeNull();
    if (textarea) {
      await user.type(textarea, "hello");
      await user.type(textarea, "{Enter}");
    }

    expect(fakeOnSubmit).toHaveBeenCalled();
  });

  test("when I hold shift and push enter it should not call onSubmit", async () => {
    const fakeOnSubmit = vi.fn();

    const { user, ...app } = render(<App onSubmit={fakeOnSubmit} />, {
      preloadedState: engineConfigState,
    });
    const textarea = app.container.querySelector("textarea");
    expect(textarea).not.toBeNull();
    if (textarea) {
      await user.type(textarea, "hello");
      await user.type(textarea, "{Shift>}{enter}{/Shift}");
    }
    expect(fakeOnSubmit).not.toHaveBeenCalled();
  });

  test("checkbox snippet", async () => {
    const fakeOnSubmit = vi.fn();
    const snippet = {
      language: "python",
      code: "print(1)",
      path: "/Users/refact/projects/print1.py",
      basename: "print1.py",
    };
    const { user, ...app } = render(<App onSubmit={fakeOnSubmit} />, {
      preloadedState: {
        selected_snippet: snippet,
        active_file: {
          name: "foo.txt",
          cursor: 2,
          path: "foo.txt",
          line1: 1,
          line2: 3,
          can_paste: true,
        },
        config: { host: "vscode", themeProps: {}, lspPort: 8001 },
      },
    });

    await user.click(screen.getByTestId("chat-form-textarea"));
    const label = screen.queryByText(/Selected \d* lines/);
    expect(label).not.toBeNull();
    // eslint-disable-next-line @typescript-eslint/no-non-null-assertion
    const textarea = app.container.querySelector("textarea")!;
    await user.type(textarea, "foo");
    await user.keyboard("{Enter}");
    const markdown = "```python\nprint(1)\n```\n";
    const expected = `${markdown}\n@file foo.txt:3\nfoo\n`;
    expect(fakeOnSubmit).toHaveBeenCalledWith(expected, "after_flow");
  });

  test("accepting a slash command pastes it without submitting, then Enter sends it", async () => {
    const fakeOnSubmit = vi.fn();
    server.use(
      http.post("*/v1/at-command-completion", () => {
        return HttpResponse.json({
          completions: ["/review"],
          completion_details: {
            "/review": {
              description: "Review code for issues",
              source: "global_refact",
              kind: "skill",
            },
          },
          replace: [0, 1],
          is_cmd_executable: false,
        });
      }),
    );

    const { user, ...app } = render(<App onSubmit={fakeOnSubmit} />, {
      preloadedState: engineConfigState,
    });
    const textarea = app.container.querySelector("textarea");
    expect(textarea).not.toBeNull();
    if (!textarea) return;

    await user.type(textarea, "/");
    await waitFor(() => expect(app.queryByText("/review")).not.toBeNull());
    await user.keyboard("{Enter}");

    await waitFor(() => {
      expect(textarea.value).toEqual("/review ");
    });
    expect(fakeOnSubmit).not.toHaveBeenCalled();

    await user.keyboard("{Enter}");
    await waitFor(() => {
      expect(fakeOnSubmit).toHaveBeenCalledWith("/review\n", "after_flow");
    });
  });

  test("strips unfilled argument placeholders before sending", async () => {
    const fakeOnSubmit = vi.fn();
    server.use(
      http.post("*/v1/at-command-completion", () => {
        return HttpResponse.json({
          completions: ["/optimize"],
          completion_details: {
            "/optimize": {
              description: "Optimize code for performance",
              argument_hint: "[file-path]",
              source: "project_refact",
              kind: "cmd",
            },
          },
          replace: [0, 1],
          is_cmd_executable: false,
        });
      }),
    );

    const { user, ...app } = render(<App onSubmit={fakeOnSubmit} />, {
      preloadedState: engineConfigState,
    });
    const textarea = app.container.querySelector("textarea");
    expect(textarea).not.toBeNull();
    if (!textarea) return;

    await user.type(textarea, "/");
    await waitFor(() => expect(app.queryByText("/optimize")).not.toBeNull());
    await user.keyboard("{Enter}");

    await waitFor(() => {
      expect(textarea.value).toEqual("/optimize [file-path]");
    });

    await user.keyboard("{Enter}");
    await waitFor(() => {
      expect(fakeOnSubmit).toHaveBeenCalledWith("/optimize\n", "after_flow");
    });
  });

  test("dedupes preview tile when attached file is returned with a shortened path", async () => {
    const previewSpy = vi.fn();
    server.use(
      http.post("*/v1/at-command-preview", () => {
        previewSpy();
        return HttpResponse.json({
          messages: [
            {
              role: "context_file",
              content: [
                {
                  file_name: "refact-agent/gui/codegen.ts",
                  file_content: "export {};",
                  line1: 1,
                  line2: 38,
                },
              ],
            },
          ],
          current_context: 10,
          number_context: 10,
        });
      }),
    );

    render(<App />, {
      preloadedState: {
        selected_snippet: {
          language: "typescript",
          code: "export const x = 1;",
          path: "/home/test/refact-agent/gui/codegen.ts",
          basename: "codegen.ts",
        },
        active_file: {
          name: "codegen.ts",
          cursor: 1,
          path: "/home/test/refact-agent/gui/codegen.ts",
          line1: 1,
          line2: 1,
          can_paste: true,
        },
        config: { host: "jetbrains", themeProps: {}, lspPort: 8001 },
      },
    });

    await waitFor(() => expect(previewSpy).toHaveBeenCalled());
    await waitFor(() => {
      expect(
        document.querySelectorAll('[aria-label^="File: codegen.ts"]').length,
      ).toBe(1);
    });
  });

  test("does not request command preview for an empty composer", async () => {
    const previewSpy = vi.fn();
    server.use(
      http.post("*/v1/at-command-preview", () => {
        previewSpy();
        return HttpResponse.json({
          messages: [],
          current_context: 0,
          number_context: 10,
        });
      }),
    );

    const chat = chatStateWithThread({
      messages: [{ role: "user", content: "Existing transcript" }],
    });
    const { store } = render(<App />, {
      preloadedState: {
        chat,
        ...engineConfigState,
      },
    });

    await new Promise((resolve) => window.setTimeout(resolve, 600));
    expect(previewSpy).not.toHaveBeenCalled();

    const threadId = chat.current_thread_id;
    store.dispatch(
      applyChatEvent({
        chat_id: threadId,
        seq: "2",
        type: "message_added",
        message: { role: "assistant", content: "Streaming update" },
        index: 1,
      }),
    );

    await new Promise((resolve) => window.setTimeout(resolve, 600));
    expect(previewSpy).not.toHaveBeenCalled();
  });

  test("previews a draft without resending the existing transcript", async () => {
    const previewBodies: { messages: { role: string; content: unknown }[] }[] =
      [];
    server.use(
      http.post("*/v1/at-command-preview", async ({ request }) => {
        previewBodies.push(
          (await request.json()) as {
            messages: { role: string; content: unknown }[];
          },
        );
        return HttpResponse.json({
          messages: [],
          current_context: 1,
          number_context: 10,
        });
      }),
    );

    const chat = chatStateWithThread({
      messages: [
        {
          role: "user",
          content: "large history".repeat(100_000),
        },
      ],
    });
    const { user, container } = render(<App />, {
      preloadedState: {
        chat,
        ...engineConfigState,
      },
    });
    const textarea = container.querySelector("textarea");
    expect(textarea).not.toBeNull();
    if (!textarea) return;

    await user.type(textarea, "draft");
    await waitFor(() => expect(previewBodies).toHaveLength(1));

    expect(previewBodies[0]?.messages).toHaveLength(1);
    expect(previewBodies[0]?.messages[0]?.role).toBe("user");
    expect(String(previewBodies[0]?.messages[0]?.content).trim()).toBe("draft");
  });

  test.skip("does not submit while IME composition is active", async () => {
    // TODO: happy-dom/user-event cannot preserve native isComposing through the ComboBox
    // keydown path; keep as an executable characterization once the harness can model IME.
    const fakeOnSubmit = vi.fn();

    const { user, ...app } = render(<App onSubmit={fakeOnSubmit} />, {
      preloadedState: engineConfigState,
    });
    const textarea = app.container.querySelector("textarea");
    expect(textarea).not.toBeNull();
    if (!textarea) return;

    await user.type(textarea, "hello");
    fireEvent.keyDown(textarea, {
      key: "Enter",
      code: "Enter",
      isComposing: true,
    });

    expect(fakeOnSubmit).not.toHaveBeenCalled();
  });

  test("@help displays quick help without submitting", async () => {
    const fakeOnSubmit = vi.fn();

    const { user, ...app } = render(<App onSubmit={fakeOnSubmit} />, {
      preloadedState: engineConfigState,
    });
    const textarea = app.container.querySelector("textarea");
    expect(textarea).not.toBeNull();
    if (!textarea) return;

    await user.type(textarea, "@help");

    expect(app.getByText("Quick help for @-commands:")).toBeInTheDocument();
    expect(fakeOnSubmit).not.toHaveBeenCalled();
  });

  test("pasting a text file attaches its contents to the submitted prompt", async () => {
    const fakeOnSubmit = vi.fn();

    const { user, ...app } = render(<App onSubmit={fakeOnSubmit} />, {
      preloadedState: {
        chat: chatStateWithThread({ model: "openai/gpt-4o" }),
        ...engineConfigState,
      },
    });
    const textarea = app.container.querySelector("textarea");
    expect(textarea).not.toBeNull();
    if (!textarea) return;

    const prevented = pasteFile(
      textarea,
      new File(["alpha\nbeta"], "notes.md", { type: "text/markdown" }),
    );
    expect(prevented).toBe(false);

    await waitFor(() => {
      expect(
        app.store.getState().chat.threads[
          app.store.getState().chat.current_thread_id
        ]?.attached_text_files,
      ).toHaveLength(1);
    });

    await user.type(textarea, "summarize this");
    await user.keyboard("{Enter}");

    expect(fakeOnSubmit).toHaveBeenCalledWith(
      "```md notes.md\nalpha\nbeta\n```\n\nsummarize this\n",
      "after_flow",
    );
  });

  test("pasting an image is gated by current model multimodality support", () => {
    const imageFile = new File(["image"], "diagram.png", { type: "image/png" });

    const { ...app } = render(<App />, {
      preloadedState: {
        chat: chatStateWithThread({ model: "openai/o1-mini" }),
        ...engineConfigState,
      },
    });
    const textarea = app.container.querySelector("textarea");
    expect(textarea).not.toBeNull();
    if (!textarea) return;

    const prevented = pasteFile(textarea, imageFile);

    expect(prevented).toBe(false);
    expect(
      app.store.getState().chat.threads[
        app.store.getState().chat.current_thread_id
      ]?.attached_images,
    ).toHaveLength(0);
  });

  test("pasting WebP and GIF images uses the image attachment path", async () => {
    const { user, ...app } = render(<App />, {
      preloadedState: {
        chat: chatStateWithThread({ model: "openai/gpt-4o" }),
        ...engineConfigState,
      },
    });
    const textarea = app.container.querySelector("textarea");
    expect(textarea).not.toBeNull();
    if (!textarea) return;
    await user.click(textarea);
    await screen.findByRole("button", { name: "Attach images" });

    expect(
      pasteFile(
        textarea,
        new File(["webp"], "diagram.webp", { type: "image/webp" }),
      ),
    ).toBe(false);
    expect(
      pasteFile(
        textarea,
        new File(["gif"], "animation.gif", { type: "image/gif" }),
      ),
    ).toBe(false);

    await waitFor(() => {
      expect(
        app.store
          .getState()
          .chat.threads[
            app.store.getState().chat.current_thread_id
          ]?.attached_images.map((image) => image.type),
      ).toEqual(["image/webp", "image/gif"]);
    });
  });

  test("pasting an unknown binary reports an error and does not attach text", async () => {
    const { ...app } = render(<App />, {
      preloadedState: {
        chat: chatStateWithThread({ model: "openai/gpt-4o" }),
        ...engineConfigState,
      },
    });
    const textarea = app.container.querySelector("textarea");
    expect(textarea).not.toBeNull();
    if (!textarea) return;

    expect(
      pasteFile(
        textarea,
        new File([new Uint8Array([0, 1])], "payload.bin", {
          type: "application/octet-stream",
        }),
      ),
    ).toBe(false);

    await waitFor(() => {
      expect(app.store.getState().error.message).toBe(
        "Could not attach payload.bin: unsupported file type",
      );
    });
    expect(
      app.store.getState().chat.threads[
        app.store.getState().chat.current_thread_id
      ]?.attached_text_files,
    ).toHaveLength(0);
  });

  test("focusing the input expands and an open settings popover keeps it expanded", async () => {
    const { user, ...app } = render(<App />, {
      preloadedState: {
        chat: chatStateWithThread({ model: "openai/gpt-4o" }),
        ...engineConfigState,
      },
    });
    const textarea = screen.getByTestId("chat-form-textarea");
    const form = textarea.closest("form");
    expect(form).not.toBeNull();
    if (!form) return;

    expect(form.className).toContain("chatFormCollapsed");
    expect(
      screen.queryByRole("button", { name: "Expand composer" }),
    ).not.toBeInTheDocument();
    await user.click(textarea);
    await waitFor(() => {
      expect(form.className).toContain("chatFormExpanded");
    });

    const settingsButton = await screen.findByRole("button", {
      name: /openai\/gpt-4o/,
    });
    expect(settingsButton).not.toBeNull();

    await user.click(settingsButton);
    await waitFor(() =>
      expect(app.getByPlaceholderText("Search models")).toBeInTheDocument(),
    );
    fireEvent.blur(textarea, { relatedTarget: null });

    await waitFor(() => {
      expect(form.className).toContain("chatFormExpanded");
    });
  });

  test("reports expansion state through onExpandedChange", async () => {
    const onExpandedChange = vi.fn();
    const { user } = render(<App onExpandedChange={onExpandedChange} />, {
      preloadedState: {
        chat: chatStateWithThread({ model: "openai/gpt-4o" }),
        ...engineConfigState,
      },
    });

    expect(onExpandedChange).toHaveBeenLastCalledWith(false);
    await user.click(screen.getByTestId("chat-form-textarea"));
    await waitFor(() => {
      expect(onExpandedChange).toHaveBeenLastCalledWith(true);
    });
  });

  test("an empty composer collapses once pointer and focus leave it", async () => {
    const { user } = render(<App />, {
      preloadedState: {
        chat: chatStateWithThread({ model: "openai/gpt-4o" }),
        ...engineConfigState,
      },
    });
    const textarea = screen.getByTestId("chat-form-textarea");
    const form = textarea.closest("form");
    if (!form) throw new Error("Missing form");

    await user.click(textarea);
    await waitFor(() => {
      expect(form.className).toContain("chatFormExpanded");
    });
    fireEvent.pointerDown(document.body);
    textarea.blur();
    await waitFor(() => {
      expect(form.className).toContain("chatFormCollapsed");
    });
    expect(screen.queryByText("Dangerous commands auto-approved")).toBeNull();
  });

  test("the project information dialog keeps an empty composer expanded", async () => {
    server.use(
      http.get("*/v1/project-information", () =>
        HttpResponse.json({ detail: "unavailable" }, { status: 503 }),
      ),
      http.post("*/v1/project-information/preview", () =>
        HttpResponse.json({ detail: "unavailable" }, { status: 503 }),
      ),
    );
    const rowWidth = vi
      .spyOn(HTMLElement.prototype, "clientWidth", "get")
      .mockImplementation(function (this: HTMLElement) {
        return this.dataset.row ? 100_000 : 0;
      });
    onTestFinished(() => {
      rowWidth.mockRestore();
    });
    const { user } = render(<App />, {
      preloadedState: {
        chat: chatStateWithThread({ model: "openai/gpt-4o" }),
        ...engineConfigState,
      },
    });
    const textarea = screen.getByTestId("chat-form-textarea");
    const form = textarea.closest("form");
    if (!form) throw new Error("Missing form");
    const nextFrame = () =>
      act(async () => {
        await new Promise<void>((resolve) => {
          requestAnimationFrame(() => {
            resolve();
          });
        });
      });

    await user.click(textarea);
    await waitFor(() => {
      expect(form.className).toContain("chatFormExpanded");
    });
    const trigger = screen.getByRole("button", {
      name: "Configure project information",
    });
    await user.click(trigger);
    const dialog = await screen.findByRole("dialog");
    await waitFor(() => {
      expect(dialog.contains(document.activeElement)).toBe(true);
    });
    expect(trigger).toHaveAttribute("aria-expanded", "true");
    await nextFrame();
    await nextFrame();
    expect(form.className).toContain("chatFormExpanded");

    await user.keyboard("{Escape}");
    await waitFor(() => {
      expect(screen.queryByRole("dialog")).toBeNull();
    });
    expect(trigger).toHaveFocus();
    await nextFrame();
    await nextFrame();
    expect(form.className).toContain("chatFormExpanded");
  });

  test("status widgets remain mounted across expansion", async () => {
    const { user } = render(<App />, {
      preloadedState: {
        chat: chatStateWithThread({ model: "openai/gpt-4o" }),
        ...engineConfigState,
      },
    });

    const textarea = screen.getByTestId("chat-form-textarea");
    const form = textarea.closest("form");
    expect(form).not.toBeNull();
    if (!form) return;

    expect(form.className).toContain("chatFormCollapsed");
    const collapsedStatus = document.querySelector(
      '[data-composer-control="context"]',
    );
    if (!collapsedStatus) throw new Error("Missing context control");
    await user.click(textarea);
    await waitFor(() => {
      expect(form.className).toContain("chatFormExpanded");
    });
    expect(document.querySelector('[data-composer-control="context"]')).toBe(
      collapsedStatus,
    );
    expect(document.body.contains(collapsedStatus)).toBe(true);
    expect(
      collapsedStatus.querySelector('[class*="usageCounterContainer"]'),
    ).not.toBeNull();
  });

  test("context usage opens from its individual control", async () => {
    const { user } = render(<App />, {
      preloadedState: {
        chat: chatStateWithThread({ model: "openai/gpt-4o" }),
        ...engineConfigState,
      },
    });

    const textarea = screen.getByTestId("chat-form-textarea");
    const form = textarea.closest("form");
    expect(form).not.toBeNull();
    if (!form) return;

    expect(form.className).toContain("chatFormCollapsed");
    await user.click(textarea);
    const collapsedStatus = document.querySelector(
      '[data-composer-control="context"]',
    );
    if (!collapsedStatus) throw new Error("Missing context control");
    const usageWidget = collapsedStatus.querySelector(
      '[class*="usageCounterContainer"] [aria-haspopup]',
    );
    expect(usageWidget).not.toBeNull();
    if (!usageWidget) return;

    await user.click(usageWidget);
    await waitFor(() => {
      expect(screen.getByText("Context usage")).toBeInTheDocument();
    });
    expect(form.className).toContain("chatFormExpanded");
  });

  test("clicking stop while collapsed aborts without expanding the composer", async () => {
    const commandSpy = vi.fn();
    server.use(
      http.post("*/v1/chats/:id/commands", async ({ request }) => {
        commandSpy(await request.json());
        return HttpResponse.json({ status: "queued" });
      }),
    );

    const chat = chatStateWithThread({ model: "openai/gpt-4o" });
    chat.threads[chat.current_thread_id].streaming = true;

    const { user } = render(<App />, {
      preloadedState: { chat, ...engineConfigState },
    });

    const textarea = screen.getByTestId("chat-form-textarea");
    const form = textarea.closest("form");
    expect(form).not.toBeNull();
    if (!form) return;

    expect(form.className).toContain("chatFormCollapsed");
    await user.click(screen.getByLabelText("Stop generation"));

    await waitFor(() => {
      expect(commandSpy).toHaveBeenCalledWith(
        expect.objectContaining({ type: "abort" }),
      );
    });
    expect(form.className).toContain("chatFormCollapsed");
  });

  test("chat error banner is dismiss-only, shows detail, and clears only the local error", async () => {
    const commandSpy = vi.fn();
    const diagnosticsSpy = vi.fn();
    server.use(
      http.post("*/v1/chats/:id/commands", async ({ request }) => {
        commandSpy(await request.json());
        return HttpResponse.json({ status: "queued" });
      }),
      http.post("*/v1/buddy/diagnostics/collect", () => {
        diagnosticsSpy();
        return HttpResponse.json({});
      }),
    );

    const detail =
      "Context rebuild failed: summary model returned an empty response";
    const chat = chatStateWithThread({ model: "openai/gpt-4o" });
    const threadId = chat.current_thread_id;
    chat.threads[threadId].error = detail;

    const { user, ...app } = render(<App />, {
      preloadedState: { chat, ...engineConfigState },
    });

    expect(screen.getByText(detail, { exact: false })).toBeInTheDocument();
    expect(screen.getByText("Click to close")).toBeInTheDocument();
    expect(screen.queryByText("Click to retry")).not.toBeInTheDocument();

    await user.click(screen.getByText(/Context rebuild failed/));

    await waitFor(() => {
      expect(app.store.getState().chat.threads[threadId]?.error).toBeNull();
    });
    expect(
      screen.queryByText(detail, { exact: false }),
    ).not.toBeInTheDocument();
    expect(commandSpy).not.toHaveBeenCalled();
    const thread = app.store.getState().chat.threads[threadId];
    expect(thread?.streaming).toBe(false);
    expect(thread?.waiting_for_response).toBe(false);
  });

  test("global error banner is dismiss-only and clears the global error", async () => {
    const commandSpy = vi.fn();
    server.use(
      http.post("*/v1/chats/:id/commands", async ({ request }) => {
        commandSpy(await request.json());
        return HttpResponse.json({ status: "queued" });
      }),
    );

    const { user, ...app } = render(<App />, {
      preloadedState: {
        chat: chatStateWithThread({ model: "openai/gpt-4o" }),
        error: {
          message: "Engine unreachable",
          capsProviders: { message: null },
        },
        ...engineConfigState,
      },
    });

    expect(screen.getByText(/Engine unreachable/)).toBeInTheDocument();
    expect(screen.getByText("Click to close")).toBeInTheDocument();
    expect(screen.queryByText("Click to retry")).not.toBeInTheDocument();

    await user.click(screen.getByText(/Engine unreachable/));

    await waitFor(() => {
      expect(app.store.getState().error.message).toBeNull();
    });
    expect(commandSpy).not.toHaveBeenCalled();
  });

  test("auth error banner keeps provider guidance without dismiss or retry labels", () => {
    render(<App />, {
      preloadedState: {
        chat: chatStateWithThread({ model: "openai/gpt-4o" }),
        error: {
          message: "Unauthorized",
          isAuthError: true,
          capsProviders: { message: null },
        },
        ...engineConfigState,
      },
    });

    expect(screen.getByText(/Unauthorized/)).toBeInTheDocument();
    expect(
      screen.getByText(
        "Check your provider configuration, API key, and network access.",
      ),
    ).toBeInTheDocument();
    expect(screen.queryByText("Click to close")).not.toBeInTheDocument();
    expect(screen.queryByText("Click to retry")).not.toBeInTheDocument();
  });

  test("responsive toolbar keeps one row without clipping control groups", async () => {
    const css = await readFile(
      resolve(
        process.cwd(),
        "src/components/ChatForm/ComposerToolbar.module.css",
      ),
      "utf8",
    );
    const rowRule = css.match(/\.toolbar\s*\{([^}]+)\}/)?.[1];
    expect(rowRule).toBeDefined();
    expect(rowRule).not.toContain("overflow: hidden");
    expect(css).toContain("flex-wrap: nowrap");
    expect(css).toContain("visibility: hidden");
    expect(css).not.toContain("@container");
    expect(css).toContain("flex: 0 0 auto");
  });

  test("a multiline draft remains expanded after focus leaves", async () => {
    const { user } = render(<App />, { preloadedState: engineConfigState });
    const textarea = screen.getByTestId("chat-form-textarea");
    const draft = "first line\nsecond line\nthird line";
    fireEvent.change(textarea, { target: { value: draft } });
    await waitFor(() =>
      expect(textarea.closest("form")?.className).toContain("chatFormExpanded"),
    );
    await user.click(textarea);
    fireEvent.blur(textarea, { relatedTarget: null });
    fireEvent.pointerDown(document.body);
    await waitFor(() =>
      expect(textarea.closest("form")?.className).toContain("chatFormExpanded"),
    );
    expect(textarea).toHaveValue(draft);
  });

  test("context rebuild gate disables send, explains why, keeps the draft and wires the rebuild action", async () => {
    const fakeOnSubmit = vi.fn();
    const previewRequests: { path: string; body: unknown }[] = [];
    server.use(
      http.post(
        "*/v1/chats/:id/trajectory/llm-compress/preview",
        async ({ request }) => {
          previewRequests.push({
            path: new URL(request.url).pathname,
            body: await request.json(),
          });
          return HttpResponse.json({
            eligible: true,
            trajectory_version: 3,
            source_messages: 2,
            approximate_source_tokens: 120,
          });
        },
      ),
    );

    const chat = chatStateWithThread({
      model: "openai/gpt-4o",
      messages: blockedContextMessages,
    });
    const threadId = chat.current_thread_id;
    const { user, store } = render(<App onSubmit={fakeOnSubmit} />, {
      preloadedState: { chat, ...engineConfigState },
    });

    const textarea = screen.getByTestId("chat-form-textarea");
    const form = textarea.closest("form");
    expect(form?.className).toContain("chatFormCollapsed");
    expect(
      screen.getByText(
        "This chat's rebuilt context could not be read. Rebuild the context to continue.",
      ),
    ).toBeInTheDocument();
    const rebuildButton = screen.getByRole("button", {
      name: "Rebuild context",
    });
    expect(rebuildButton).toBeEnabled();

    await user.type(textarea, "keep this draft");
    expect(screen.getByLabelText("Send message")).toBeDisabled();
    await user.keyboard("{Enter}");
    expect(fakeOnSubmit).not.toHaveBeenCalled();
    expect(textarea).toHaveValue("keep this draft");

    await user.click(rebuildButton);
    await screen.findByText(/Rebuilds the active conversation context/);
    const previewButton = screen.getByRole("button", { name: "Preview" });
    await waitFor(() => expect(previewButton).toBeEnabled());
    await user.click(previewButton);
    await waitFor(() => expect(previewRequests).toHaveLength(1));
    expect(previewRequests[0]).toEqual({
      path: `/v1/chats/${threadId}/trajectory/llm-compress/preview`,
      body: { options: {} },
    });
    expect(textarea).toHaveValue("keep this draft");

    store.dispatch(
      applyChatEvent(rebuiltSnapshot(threadId, validRebuiltMessages)),
    );

    await waitFor(() =>
      expect(screen.getByLabelText("Send message")).toBeEnabled(),
    );
    expect(
      screen.queryByRole("button", { name: "Rebuild context" }),
    ).not.toBeInTheDocument();
    expect(textarea).toHaveValue("keep this draft");
    await user.click(screen.getByLabelText("Send message"));
    expect(fakeOnSubmit).toHaveBeenCalledWith(
      "keep this draft\n",
      "after_flow",
    );
  });

  test("a paused goal does not disable manual sends", async () => {
    const fakeOnSubmit = vi.fn();
    const chat = chatStateWithThread({
      model: "openai/gpt-4o",
      messages: [{ role: "user", content: "earlier" }],
      goal: pausedGoal,
    });
    const { user } = render(<App onSubmit={fakeOnSubmit} />, {
      preloadedState: { chat, ...engineConfigState },
    });

    const textarea = screen.getByTestId("chat-form-textarea");
    await user.type(textarea, "manual nudge");
    expect(screen.getByLabelText("Send message")).toBeEnabled();
    expect(
      screen.queryByRole("button", { name: "Rebuild context" }),
    ).not.toBeInTheDocument();
    await user.click(screen.getByLabelText("Send message"));
    expect(fakeOnSubmit).toHaveBeenCalledWith("manual nudge\n", "after_flow");
  });

  test("auto-compact toggle stays enabled while waiting and disables once generating", async () => {
    const chat = chatStateWithThread({
      model: "openai/gpt-4o",
      messages: [{ role: "user", content: "earlier" }],
    });
    const threadId = chat.current_thread_id;
    const { user, store } = render(<App />, {
      preloadedState: { chat, ...engineConfigState },
    });

    await user.click(screen.getByTestId("chat-form-textarea"));
    store.dispatch(
      applyChatEvent({
        chat_id: threadId,
        seq: "1",
        type: "runtime_updated",
        state: "executing_tools",
      }),
    );
    const runtime = store.getState().chat.threads[threadId];
    expect(runtime?.waiting_for_response).toBe(true);
    expect(runtime?.streaming).toBe(false);
    await waitFor(() =>
      expect(screen.getByTestId("auto-compact-toggle")).toBeEnabled(),
    );

    store.dispatch(
      applyChatEvent({
        chat_id: threadId,
        seq: "2",
        type: "runtime_updated",
        state: "generating",
      }),
    );
    await waitFor(() =>
      expect(screen.getByTestId("auto-compact-toggle")).toBeDisabled(),
    );
    const pressedBefore = screen
      .getByTestId("auto-compact-toggle")
      .getAttribute("aria-pressed");
    await user.click(screen.getByTestId("auto-compact-toggle"));
    expect(
      screen.getByTestId("auto-compact-toggle").getAttribute("aria-pressed"),
    ).toBe(pressedBefore);
  });

  test.each([
    "{Shift>}{enter>}{/enter}{/Shift}", // hold shift, hold enter, release enter, release shift,
    "{Shift>}{enter>}{/Shift}{/enter}", // hold shift, hold enter, release enter, release shift,
  ])("when pressing %s, it should not submit", async (a) => {
    const fakeOnSubmit = vi.fn();

    const { user, ...app } = render(<App onSubmit={fakeOnSubmit} />, {
      preloadedState: engineConfigState,
    });
    const textarea = app.container.querySelector("textarea");
    expect(textarea).not.toBeNull();
    if (textarea) {
      await user.type(textarea, "hello");
      await user.type(textarea, a);
    }
    expect(fakeOnSubmit).not.toHaveBeenCalled();
  });
});
