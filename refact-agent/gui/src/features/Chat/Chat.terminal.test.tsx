import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, screen } from "../../utils/test-utils";
import { act } from "react-dom/test-utils";
import { setUpStore } from "../../app/store";
import { Chat } from "./Chat";
import { createDefaultChatState } from "../../utils/test-utils";
import { setDockOpen } from "../Workspace/workspaceSlice";

const viewport = vi.hoisted(() => {
  vi.resetModules();
  return { narrow: true, buddy: false };
});
vi.mock("../../components/ui", async (importOriginal) => ({
  ...(await importOriginal<object>()),
  useMediaQuery: () => viewport.narrow,
}));
vi.mock("./Thread", async (importOriginal) => ({
  ...(await importOriginal<object>()),
  selectIsBuddyChat: () => viewport.buddy,
}));
vi.mock("../../components/ChatContent", () => ({ ChatContent: () => null }));
vi.mock("../../components/ChatForm", () => ({ ChatForm: () => null }));
vi.mock("../../components/ChatForm/useComposerActivity", () => ({
  useComposerActivity: () => undefined,
}));
vi.mock("../../components/TaskProgressWidget", () => ({
  TaskProgressWidget: () => null,
}));
vi.mock("../../hooks/useChatActions", () => ({ useChatActions: () => ({}) }));
vi.mock("../../hooks/useCheckpoints", () => ({ useCheckpoints: () => ({}) }));
vi.mock("../Workspace/TerminalPanel", () => ({
  TerminalPanel: () => <div data-testid="terminal-workbench" />,
}));

afterEach(() => {
  cleanup();
  viewport.narrow = true;
  viewport.buddy = false;
});

function setup(
  host: "web" | "vscode" = "web",
  terminalPanel = true,
  filesPanel = true,
) {
  const defaults = setUpStore().getState();
  const store = setUpStore({
    chat: createDefaultChatState(),
    config: {
      ...defaults.config,
      host,
      capabilities: {
        ...defaults.config.capabilities,
        filesPanel,
        gitPanel: false,
        terminalPanel,
      },
    },
  });
  store.dispatch(setDockOpen(true));
  const view = render(
    <Chat host={host} tabbed={false} backFromChat={() => undefined} />,
    { store },
  );
  return { ...view, store };
}

describe("Chat terminal sidebar independence", () => {
  it("keeps the same terminal mounted when the narrow sidebar closes", () => {
    const { store } = setup();
    const terminal = screen.getByTestId("terminal-workbench");
    act(() => {
      store.dispatch(setDockOpen(false));
    });
    expect(screen.getByTestId("terminal-workbench")).toBe(terminal);
  });

  it("preserves desktop closed-dock terminal hiding", () => {
    viewport.narrow = false;
    const { store } = setup();
    expect(screen.getByTestId("terminal-workbench")).toBeInTheDocument();
    act(() => {
      store.dispatch(setDockOpen(false));
    });
    expect(screen.queryByTestId("terminal-workbench")).not.toBeInTheDocument();
  });

  it("preserves desktop terminal-only availability with no sidebar", () => {
    viewport.narrow = false;
    const { store } = setup("web", true, false);
    act(() => {
      store.dispatch(setDockOpen(false));
    });
    expect(screen.getByTestId("terminal-workbench")).toBeInTheDocument();
  });

  it.each(["capability", "host", "buddy"])(
    "preserves the %s guard on narrow screens",
    (guard) => {
      viewport.buddy = guard === "buddy";
      setup(guard === "host" ? "vscode" : "web", guard !== "capability");
      expect(
        screen.queryByTestId("terminal-workbench"),
      ).not.toBeInTheDocument();
    },
  );
});
