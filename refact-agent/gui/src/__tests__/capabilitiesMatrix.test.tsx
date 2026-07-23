import React from "react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { type AppStore, setUpStore } from "../app/store";
import { createChatWithId } from "../features/Chat/Thread";
import type { Capabilities, Config } from "../features/Config/configSlice";
import { TabBar } from "../features/Workspace/TabBar";
import { makeSurfaceKey } from "../features/Workspace/surfaceKey";
import {
  hydrateWorkspace,
  openTab,
  reconcileWorkspace,
  setActiveTab,
} from "../features/Workspace/workspaceSlice";
import {
  useOpenFileInApp,
  type OpenFileInAppTarget,
} from "../hooks/useOpenFileInApp";
import { resolveCapabilities } from "../utils/capabilities";
import { render, screen } from "../utils/test-utils";

const WEB_CAPABILITIES: Capabilities = {
  filesPanel: true,
  gitPanel: true,
  terminalPanel: true,
  openFileInApp: true,
  openFileInIde: false,
  ideDiffPasteBack: false,
  folderPicker: true,
};

const IDE_CAPABILITIES: Capabilities = {
  filesPanel: false,
  gitPanel: false,
  terminalPanel: false,
  openFileInApp: false,
  openFileInIde: true,
  ideDiffPasteBack: true,
  folderPicker: false,
};

function makeConfig(
  host: Config["host"],
  overrides?: Partial<Capabilities>,
): Config {
  const config: Config = {
    host,
    lspPort: 8001,
    themeProps: { appearance: "dark" },
  };
  if (overrides) config.capabilities = overrides;
  return config;
}

const chat = (id: string) => makeSurfaceKey("chat", id);

function createStoreWithChat(config: Config): AppStore {
  const store = setUpStore({ config });
  store.dispatch(
    createChatWithId({ id: "chat-a", title: "Chat Alpha", mode: "agent" }),
  );
  store.dispatch(openTab(chat("chat-a")));
  store.dispatch(setActiveTab(chat("chat-a")));
  return store;
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

afterEach(() => {
  vi.restoreAllMocks();
});

describe("resolveCapabilities matrix", () => {
  const matrix: readonly (readonly [
    string,
    Config["host"],
    Partial<Capabilities> | undefined,
    Capabilities,
  ])[] = [
    ["web defaults", "web", undefined, WEB_CAPABILITIES],
    ["ide defaults", "ide", undefined, IDE_CAPABILITIES],
    ["vscode defaults", "vscode", undefined, IDE_CAPABILITIES],
    ["jetbrains defaults", "jetbrains", undefined, IDE_CAPABILITIES],
    [
      "web with panels revoked",
      "web",
      { filesPanel: false, gitPanel: false, terminalPanel: false },
      {
        ...WEB_CAPABILITIES,
        filesPanel: false,
        gitPanel: false,
        terminalPanel: false,
      },
    ],
    [
      "web with ide open enabled",
      "web",
      { openFileInIde: true },
      { ...WEB_CAPABILITIES, openFileInIde: true },
    ],
    [
      "vscode with files panel granted",
      "vscode",
      { filesPanel: true, openFileInApp: true },
      { ...IDE_CAPABILITIES, filesPanel: true, openFileInApp: true },
    ],
    [
      "jetbrains with paste-back revoked",
      "jetbrains",
      { ideDiffPasteBack: false },
      { ...IDE_CAPABILITIES, ideDiffPasteBack: false },
    ],
  ];

  it.each(matrix)("resolves %s", (_name, host, overrides, expected) => {
    expect(resolveCapabilities(host, overrides)).toEqual(expected);
  });
});

describe("TabBar panel launcher reflects capabilities", () => {
  it("offers the dock without a center-panel launcher on web defaults", () => {
    const store = createStoreWithChat(makeConfig("web"));
    render(<TabBar />, { store });
    expect(
      screen.getByRole("button", { name: "Toggle workspace dock" }),
    ).toBeInTheDocument();

    expect(
      screen.queryByRole("button", { name: "Open workspace panel" }),
    ).toBeNull();
  });

  it("keeps the dock toggle when the only center panel is revoked", () => {
    const store = createStoreWithChat(makeConfig("web", { gitPanel: false }));
    render(<TabBar />, { store });

    expect(
      screen.getByRole("button", { name: "Toggle workspace dock" }),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Open workspace panel" }),
    ).toBeNull();
  });

  it("renders no launcher for IDE host defaults", () => {
    const store = createStoreWithChat(makeConfig("vscode"));
    render(<TabBar />, { store });

    expect(
      screen.queryByRole("button", { name: "Open workspace panel" }),
    ).toBeNull();
    expect(screen.getByRole("tab", { name: /Chat Alpha/ })).toBeInTheDocument();
  });

  it("renders no launcher when every panel capability is revoked on web", () => {
    const store = createStoreWithChat(
      makeConfig("web", {
        filesPanel: false,
        gitPanel: false,
        terminalPanel: false,
      }),
    );
    render(<TabBar />, { store });

    expect(
      screen.queryByRole("button", { name: "Open workspace panel" }),
    ).toBeNull();
  });

  it("keeps panel chrome hidden for IDE hosts with overrides", () => {
    const store = createStoreWithChat(
      makeConfig("vscode", { filesPanel: true }),
    );
    render(<TabBar />, { store });

    expect(
      screen.queryByRole("button", { name: "Toggle workspace dock" }),
    ).toBeNull();
    expect(
      screen.queryByRole("button", { name: "Open workspace panel" }),
    ).toBeNull();
  });
});

describe("useOpenFileInApp precedence: app > ide > none", () => {
  const filePath = "/workspace/src/main.ts";

  it("prefers the in-app Files panel when both capabilities are set", async () => {
    const postMessageSpy = vi.spyOn(window, "postMessage");
    const { user, store } = render(
      <OpenFileHarness target={{ path: filePath, line: 4 }} />,
      {
        preloadedState: {
          config: makeConfig("vscode", {
            openFileInApp: true,
            filesPanel: true,
          }),
        },
      },
    );

    await user.click(screen.getByRole("button", { name: "open" }));

    expect(store.getState().workspace.tabs).toContain(`file:${filePath}`);
    expect(store.getState().filesPanel.viewerTarget).toEqual({
      path: filePath,
      line: 4,
    });
    expect(postMessageSpy).not.toHaveBeenCalledWith(
      expect.objectContaining({ type: "ide/openFile" }),
      "*",
    );
  });

  it("falls back to the IDE bridge when only openFileInIde is set", async () => {
    const postMessageSpy = vi.spyOn(window, "postMessage");
    const { user, store } = render(
      <OpenFileHarness target={{ path: filePath, line: 4, resolved: true }} />,
      { preloadedState: { config: makeConfig("vscode") } },
    );

    await user.click(screen.getByRole("button", { name: "open" }));

    expect(postMessageSpy).toHaveBeenCalledWith(
      expect.objectContaining({
        type: "ide/openFile",
        payload: { file_path: filePath, line: 4 },
      }),
      "*",
    );
    expect(store.getState().filesPanel.viewerTarget).toBeNull();
  });

  it("stays inert with canOpen false when no capability is set", async () => {
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

describe("persisted panel tabs respect capability revocation", () => {
  function createStoreWithOpenThread(): AppStore {
    const store = setUpStore();
    store.dispatch(
      createChatWithId({ id: "chat-a", title: "Chat Alpha", mode: "agent" }),
    );
    return store;
  }

  it("hydrates enabled panel tabs and drops revoked ones", () => {
    const store = createStoreWithOpenThread();

    store.dispatch(
      hydrateWorkspace({
        tabs: [chat("chat-a"), "git:main", "files:main"],
        activeTabId: "git:main",
        groups: {},
        workspaceCapabilities: resolveCapabilities("web", { gitPanel: false }),
      }),
    );

    expect(store.getState().workspace.tabs).toEqual([chat("chat-a")]);
    expect(store.getState().workspace.activeTabId).toBe(chat("chat-a"));
  });

  it("keeps center panel tabs and migrates Terminal into the drawer", () => {
    const store = createStoreWithOpenThread();

    store.dispatch(
      hydrateWorkspace({
        tabs: [chat("chat-a"), "git:main", "terminal:main"],
        activeTabId: "git:main",
        groups: {},
        workspaceCapabilities: resolveCapabilities("web"),
      }),
    );

    expect(store.getState().workspace.tabs).toEqual([
      chat("chat-a"),
      "git:main",
    ]);
    expect(store.getState().workspace.activeTabId).toBe("git:main");
    expect(store.getState().workspace.drawer?.open).toBe(true);
  });

  it("drops every panel tab when hydrating with IDE capabilities", () => {
    const store = createStoreWithOpenThread();

    store.dispatch(
      hydrateWorkspace({
        tabs: [chat("chat-a"), "files:main", "git:main", "terminal:main"],
        activeTabId: "files:main",
        groups: {},
        workspaceCapabilities: resolveCapabilities("vscode"),
      }),
    );

    expect(store.getState().workspace.tabs).toEqual([chat("chat-a")]);
    expect(store.getState().workspace.activeTabId).toBe(chat("chat-a"));
  });

  it("reconciles away panel tabs whose capability was revoked", () => {
    const store = createStoreWithOpenThread();

    store.dispatch(
      hydrateWorkspace({
        tabs: [chat("chat-a"), "terminal:main"],
        activeTabId: "terminal:main",
        groups: {},
        workspaceCapabilities: resolveCapabilities("web"),
      }),
    );
    store.dispatch(
      reconcileWorkspace({
        openThreadIds: ["chat-a"],
        workspaceCapabilities: resolveCapabilities("web", {
          terminalPanel: false,
        }),
      }),
    );

    expect(store.getState().workspace.tabs).toEqual([chat("chat-a")]);
    expect(store.getState().workspace.activeTabId).toBe(chat("chat-a"));
  });
});
