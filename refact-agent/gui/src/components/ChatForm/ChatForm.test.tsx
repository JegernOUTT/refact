import React from "react";
import { beforeEach, describe, expect, test, vi } from "vitest";
import { http, HttpResponse } from "msw";
import { fireEvent, screen } from "@testing-library/react";

import { render, waitFor } from "../../utils/test-utils";
import { ChatForm, ChatFormProps } from "./ChatForm";
import { createDefaultChatState } from "../../utils/test-utils";

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

server.use(...handlers);

const App: React.FC<Partial<ChatFormProps>> = ({ ...props }) => {
  const defaultProps: ChatFormProps = {
    onSubmit: (_str: string) => ({}),
    ...props,
  };

  return <ChatForm {...defaultProps} />;
};

describe("ChatForm", () => {
  beforeEach(() => {
    server.use(...handlers);
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

    const label = app.queryByText(/Selected \d* lines/);
    expect(label).not.toBeNull();
    // eslint-disable-next-line @typescript-eslint/no-non-null-assertion
    const textarea = app.container.querySelector("textarea")!;
    await user.type(textarea, "foo");
    await user.keyboard("{Enter}");
    const markdown = "```python\nprint(1)\n```\n";
    const expected = `${markdown}\n@file foo.txt:3\nfoo\n`;
    expect(fakeOnSubmit).toHaveBeenCalledWith(expected, "after_flow");
  });

  test("submits the accepted slash command instead of the typed prefix", async () => {
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
      expect(fakeOnSubmit).toHaveBeenCalledWith("/review\n", "after_flow");
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

    expect(prevented).toBe(true);
    expect(
      app.store.getState().chat.threads[
        app.store.getState().chat.current_thread_id
      ]?.attached_images,
    ).toHaveLength(0);
  });

  test("composer expands on focus and stays expanded while settings popover is open", async () => {
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
    await user.click(textarea);
    expect(form.className).toContain("chatFormExpanded");

    const settingsButton = app.container.querySelector(
      'button[class*="trigger"]',
    );
    expect(settingsButton).not.toBeNull();
    if (!settingsButton) return;

    await user.click(settingsButton);
    await waitFor(() =>
      expect(app.getByPlaceholderText("Search models")).toBeInTheDocument(),
    );
    fireEvent.blur(textarea, { relatedTarget: null });

    await waitFor(() => {
      expect(form.className).toContain("chatFormExpanded");
    });
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
