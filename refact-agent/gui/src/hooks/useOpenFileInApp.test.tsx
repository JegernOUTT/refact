import React from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { http, HttpResponse } from "msw";

import { render, screen, waitFor } from "../utils/test-utils";
import { server } from "../utils/mockServer";
import { useOpenFileInApp, type OpenFileInAppTarget } from "./useOpenFileInApp";
import { useGoToLink } from "./useGoToLink";
import type { Config } from "../features/Config/configSlice";
import { createDefaultChatState } from "../utils/test-utils";
import { setUpStore } from "../app/store";
import { ChatThreadProvider } from "../features/Chat/Thread/ChatThreadContext";

const filePath = "/workspace/src/main.ts";

function makeConfig(host: Config["host"], overrides?: Config["capabilities"]) {
  const config: Config = {
    host,
    lspPort: 8001,
    themeProps: { appearance: "dark" },
  };
  if (overrides) config.capabilities = overrides;
  return config;
}

const OpenFileHarness: React.FC<{ target: OpenFileInAppTarget }> = ({
  target,
}) => {
  const { canOpen, openFile } = useOpenFileInApp();
  return (
    <button data-can-open={canOpen} onClick={() => openFile(target)}>
      open
    </button>
  );
};

const GoToLinkHarness: React.FC = () => {
  const { handleGoTo } = useGoToLink();
  return (
    <button onClick={() => handleGoTo({ goto: `editor:${filePath}` })}>
      go
    </button>
  );
};

function useFullPathHandler(resolvedPath: string) {
  server.use(
    http.post("*/v1/fullpath", () =>
      HttpResponse.json({ fullpath: resolvedPath, is_directory: false }),
    ),
  );
}

afterEach(() => {
  vi.restoreAllMocks();
});

describe("useOpenFileInApp", () => {
  it("resolves against the source chat, not the currently focused chat", async () => {
    const chat = createDefaultChatState();
    const source = createDefaultChatState();
    const sourceId = source.current_thread_id;
    source.threads[sourceId].thread.worktree = {
      id: "source-worktree",
      kind: "chat",
      root: "/source-worktree",
      source_workspace_root: "/project",
      repo_root: "/project",
      enforce: true,
    };
    chat.threads = { ...chat.threads, ...source.threads };
    const store = setUpStore({ chat, config: makeConfig("web") });
    const { user } = render(
      <ChatThreadProvider chatId={sourceId}>
        <OpenFileHarness
          target={{ path: "refact-agent/engine/src/caps/caps.rs", line: 42 }}
        />
      </ChatThreadProvider>,
      { store },
    );
    await user.click(screen.getByRole("button", { name: "open" }));
    const absolute = "/source-worktree/refact-agent/engine/src/caps/caps.rs";
    expect(store.getState().filesPanel.viewerTarget).toEqual({
      path: absolute,
      line: 42,
    });
    expect(store.getState().workspace.tabs).toContain(`file:${absolute}`);
    expect(store.getState().workspace.tabs).not.toContain(
      "file:refact-agent/engine/src/caps/caps.rs",
    );
  });

  it("resolves a sole project root before dispatch", async () => {
    const initial = setUpStore().getState();
    const { user, store } = render(
      <OpenFileHarness target={{ path: "src/main.ts", line: 8 }} />,
      {
        preloadedState: {
          config: makeConfig("web"),
          current_project: {
            ...initial.current_project,
            workspaceRoots: ["/workspace"],
          },
        },
      },
    );
    await user.click(screen.getByRole("button", { name: "open" }));
    expect(store.getState().filesPanel.viewerTarget).toEqual({
      path: filePath,
      line: 8,
    });
  });

  it.each(["src/main.ts", "../secret", "%2e%2e/secret"])(
    "rejects ambiguous or unsafe %s without creating a surface",
    async (path) => {
      const initial = setUpStore().getState();
      const { user, store } = render(
        <OpenFileHarness target={{ path, resolved: true }} />,
        {
          preloadedState: {
            config: makeConfig("web"),
            current_project: {
              ...initial.current_project,
              workspaceRoots: ["/one", "/two"],
            },
          },
        },
      );
      await user.click(screen.getByRole("button", { name: "open" }));
      expect(store.getState().filesPanel.viewerTarget).toBeNull();
      expect(store.getState().error.message).toMatch(/Cannot open/);
    },
  );

  it("opens a file viewer tab on the web host", async () => {
    const { user, store } = render(
      <OpenFileHarness target={{ path: filePath, line: 7 }} />,
      { preloadedState: { config: makeConfig("web") } },
    );

    expect(screen.getByRole("button", { name: "open" })).toHaveAttribute(
      "data-can-open",
      "true",
    );

    await user.click(screen.getByRole("button", { name: "open" }));

    expect(store.getState().workspace.tabs).toContain(`file:${filePath}`);
    expect(store.getState().filesPanel.viewerTarget).toEqual({
      path: filePath,
      line: 7,
    });
  });

  it("resolves the path then posts ide/openFile on the vscode host", async () => {
    useFullPathHandler("/resolved/src/main.ts");
    const postMessageSpy = vi.spyOn(window, "postMessage");
    const { user, store } = render(
      <OpenFileHarness target={{ path: filePath, line: 7 }} />,
      { preloadedState: { config: makeConfig("vscode") } },
    );

    await user.click(screen.getByRole("button", { name: "open" }));

    await waitFor(() => {
      expect(postMessageSpy).toHaveBeenCalledWith(
        expect.objectContaining({
          type: "ide/openFile",
          payload: { file_path: "/resolved/src/main.ts", line: 7 },
        }),
        "*",
      );
    });
    expect(store.getState().filesPanel.viewerTarget).toBeNull();
  });

  it("posts ide/openFile directly for resolved targets on the vscode host", async () => {
    const postMessageSpy = vi.spyOn(window, "postMessage");
    const { user } = render(
      <OpenFileHarness target={{ path: filePath, line: 3, resolved: true }} />,
      { preloadedState: { config: makeConfig("vscode") } },
    );

    await user.click(screen.getByRole("button", { name: "open" }));

    expect(postMessageSpy).toHaveBeenCalledWith(
      expect.objectContaining({
        type: "ide/openFile",
        payload: { file_path: filePath, line: 3 },
      }),
      "*",
    );
  });

  it("prefers the in-app file viewer when both capabilities are enabled", async () => {
    const postMessageSpy = vi.spyOn(window, "postMessage");
    const { user, store } = render(
      <OpenFileHarness target={{ path: filePath }} />,
      {
        preloadedState: {
          config: makeConfig("vscode", { openFileInApp: true }),
        },
      },
    );

    await user.click(screen.getByRole("button", { name: "open" }));

    expect(store.getState().filesPanel.viewerTarget).toEqual({
      path: filePath,
      line: undefined,
    });
    expect(postMessageSpy).not.toHaveBeenCalledWith(
      expect.objectContaining({ type: "ide/openFile" }),
      "*",
    );
  });

  it("reports canOpen false and stays inert without any open capability", async () => {
    const postMessageSpy = vi.spyOn(window, "postMessage");
    const { user, store } = render(
      <OpenFileHarness target={{ path: filePath }} />,
      {
        preloadedState: {
          config: makeConfig("web", {
            openFileInApp: false,
            openFileInIde: false,
          }),
        },
      },
    );

    expect(screen.getByRole("button", { name: "open" })).toHaveAttribute(
      "data-can-open",
      "false",
    );

    await user.click(screen.getByRole("button", { name: "open" }));

    expect(store.getState().filesPanel.viewerTarget).toBeNull();
    expect(postMessageSpy).not.toHaveBeenCalledWith(
      expect.objectContaining({ type: "ide/openFile" }),
      "*",
    );
  });
});

describe("useGoToLink editor links", () => {
  it("routes editor: links into a file viewer tab on the web host", async () => {
    const { user, store } = render(<GoToLinkHarness />, {
      preloadedState: { config: makeConfig("web") },
    });

    await user.click(screen.getByRole("button", { name: "go" }));

    expect(store.getState().workspace.tabs).toContain(`file:${filePath}`);
    expect(store.getState().filesPanel.viewerTarget).toEqual({
      path: filePath,
      line: undefined,
    });
  });

  it("routes editor: links to the IDE on the vscode host", async () => {
    useFullPathHandler(filePath);
    const postMessageSpy = vi.spyOn(window, "postMessage");
    const { user, store } = render(<GoToLinkHarness />, {
      preloadedState: { config: makeConfig("vscode") },
    });

    await user.click(screen.getByRole("button", { name: "go" }));

    await waitFor(() => {
      expect(postMessageSpy).toHaveBeenCalledWith(
        expect.objectContaining({
          type: "ide/openFile",
          payload: { file_path: filePath, line: undefined },
        }),
        "*",
      );
    });
    expect(store.getState().filesPanel.viewerTarget).toBeNull();
  });
});
