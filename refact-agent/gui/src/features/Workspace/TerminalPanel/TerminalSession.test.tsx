import { waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";

import { terminalKeyHandler } from "./terminalKeys";

type TerminalSessionComponent =
  typeof import("./TerminalSession").TerminalSession;
type RenderFn = typeof import("../../../utils/test-utils").render;

type FakeTerminalOptions = {
  convertEol?: boolean;
  cursorBlink?: boolean;
  cursorInactiveStyle?: string;
  disableStdin?: boolean;
  fontFamily?: string;
  fontSize?: number;
  lineHeight?: number;
  macOptionIsMeta?: boolean;
  scrollback?: number;
  theme?: Record<string, string | undefined>;
};

class FakeTerminal {
  static instances: FakeTerminal[] = [];
  readonly constructorOptions: FakeTerminalOptions;
  options: FakeTerminalOptions = {};
  rows = 24;
  cols = 80;
  unicode = { activeVersion: "6" };
  loadAddon = (): undefined => undefined;
  open = (): undefined => undefined;
  focus = vi.fn<() => undefined>();
  dispose = (): undefined => undefined;
  onData = () => ({ dispose: (): undefined => undefined });
  write = vi.fn<(data: string) => undefined>();
  attachCustomKeyEventHandler =
    vi.fn<(handler: (event: KeyboardEvent) => boolean) => undefined>();
  hasSelection = (): boolean => false;
  getSelection = (): string => "";
  clearSelection = (): undefined => undefined;

  constructor(options: FakeTerminalOptions) {
    this.constructorOptions = options;
    FakeTerminal.instances.push(this);
  }
}

const DARK_TOKENS: Record<string, string> = {
  "--rf-bg": "#0c0d0f",
  "--rf-color-fg": "rgba(255, 255, 255, 0.92)",
  "--rf-color-muted": "rgba(255, 255, 255, 0.48)",
  "--rf-color-faint": "rgba(255, 255, 255, 0.28)",
  "--rf-color-accent": "#7f93d8",
  "--rf-color-success": "#5fae8b",
  "--rf-color-warning": "#cda04e",
  "--rf-color-danger": "#d8736d",
  "--rf-chart-5": "#6cb6c9",
  "--rf-chart-6": "#b08ad1",
  "--rf-font-mono": "ui-monospace, monospace",
  "--rf-text-2": "13px",
};

const LIGHT_TOKENS: Record<string, string> = {
  ...DARK_TOKENS,
  "--rf-bg": "#fcfcfd",
  "--rf-color-fg": "rgba(0, 0, 0, 0.88)",
};

const tokenState = { current: DARK_TOKENS };
const useExecSessionMock = vi.fn(() => ({
  error: null,
  reconnecting: false,
}));

const CONFIG_STATE = {
  config: {
    host: "web" as const,
    lspPort: 8001,
    apiKey: null,
    themeProps: {},
  },
};

let TerminalSession: TerminalSessionComponent;
let render: RenderFn;

beforeEach(async () => {
  vi.resetModules();
  FakeTerminal.instances = [];
  tokenState.current = DARK_TOKENS;
  useExecSessionMock.mockClear();
  vi.doMock("@xterm/xterm", () => ({ Terminal: FakeTerminal }));
  vi.doMock("@xterm/addon-fit", () => ({
    FitAddon: class {
      fit = (): undefined => undefined;
    },
  }));
  vi.doMock("./useExecSession", () => ({
    useExecSession: useExecSessionMock,
  }));
  vi.doMock("../../../components/ui", async (importOriginal) => {
    const actual =
      await importOriginal<typeof import("../../../components/ui")>();
    return { ...actual, useTokens: () => tokenState.current };
  });
  ({ TerminalSession } = await import("./TerminalSession"));
  ({ render } = await import("../../../utils/test-utils"));
});

afterEach(() => {
  vi.doUnmock("@xterm/xterm");
  vi.doUnmock("@xterm/addon-fit");
  vi.doUnmock("./useExecSession");
  vi.doUnmock("../../../components/ui");
});

describe("TerminalSession", () => {
  test("constructs the terminal with a token-derived dark theme", async () => {
    render(
      <TerminalSession
        processId="proc-theme"
        chatId="chat-a"
        onStatusChange={vi.fn()}
      />,
      { preloadedState: CONFIG_STATE },
    );

    await waitFor(() => expect(FakeTerminal.instances).toHaveLength(1));
    expect(useExecSessionMock).toHaveBeenCalledWith(
      expect.objectContaining({ processId: "proc-theme", chatId: "chat-a" }),
    );
    const constructed = FakeTerminal.instances[0].constructorOptions;
    expect(FakeTerminal.instances[0].focus).toHaveBeenCalledOnce();
    expect(constructed.theme?.background).not.toBe("#ffffff");
    expect(constructed.theme?.background).toBe("#0c0d0f");
    expect(constructed.theme?.foreground).toBe("rgba(255, 255, 255, 0.92)");
    expect(constructed.theme?.cursor).toBe("#7f93d8");
    expect(constructed.theme?.red).toBe("#d8736d");
    expect(constructed.theme?.green).toBe("#5fae8b");
    expect(constructed.theme?.yellow).toBe("#cda04e");
    expect(constructed.theme?.cyan).toBe("#6cb6c9");
    expect(constructed.theme?.magenta).toBe("#b08ad1");
    expect(constructed.theme?.brightBlack).toBe("rgba(255, 255, 255, 0.28)");
    expect(constructed.fontFamily).toBe("ui-monospace, monospace");
    expect(constructed.fontSize).toBe(13);
    expect(constructed.lineHeight).toBeGreaterThan(1);
    expect(constructed.scrollback).toBeGreaterThan(1000);
    expect(constructed.convertEol).toBe(false);
    expect(constructed.macOptionIsMeta).toBe(true);
    expect(FakeTerminal.instances[0].unicode.activeVersion).toBe("11");
    expect(
      FakeTerminal.instances[0].attachCustomKeyEventHandler,
    ).toHaveBeenCalledOnce();
    expect(FakeTerminal.instances[0].write).not.toHaveBeenCalled();
  });

  test("opens the find bar from a search request and closes it with Escape", async () => {
    const view = render(
      <TerminalSession
        processId="proc-search"
        chatId="chat-a"
        searchRequest={0}
        onStatusChange={vi.fn()}
      />,
      { preloadedState: CONFIG_STATE },
    );
    await waitFor(() => expect(FakeTerminal.instances).toHaveLength(1));
    expect(view.queryByLabelText("Find in terminal")).toBeNull();

    view.rerender(
      <TerminalSession
        processId="proc-search"
        chatId="chat-a"
        searchRequest={1}
        onStatusChange={vi.fn()}
      />,
    );
    const input = await view.findByLabelText("Find in terminal");
    expect(input).toHaveFocus();
    await view.user.keyboard("{Escape}");
    expect(view.queryByLabelText("Find in terminal")).toBeNull();
    expect(FakeTerminal.instances[0].focus).toHaveBeenCalled();
  });

  test("terminal key handler copies, pastes and searches without leaking to the pty", () => {
    const copy = vi.fn();
    const openSearch = vi.fn();
    const selection = { current: "selected text" };
    const terminal = {
      hasSelection: () => selection.current.length > 0,
      getSelection: () => selection.current,
      clearSelection: vi.fn(),
    };
    const keydown = (init: KeyboardEventInit) =>
      new KeyboardEvent("keydown", init);

    const linux = terminalKeyHandler(terminal, { copy, openSearch }, false);
    expect(linux(keydown({ key: "c", ctrlKey: true }))).toBe(false);
    expect(copy).toHaveBeenLastCalledWith("selected text");
    expect(terminal.clearSelection).toHaveBeenCalled();
    selection.current = "";
    expect(linux(keydown({ key: "c", ctrlKey: true }))).toBe(true);
    expect(linux(keydown({ key: "C", ctrlKey: true, shiftKey: true }))).toBe(
      false,
    );
    expect(linux(keydown({ key: "v", ctrlKey: true }))).toBe(false);
    expect(linux(keydown({ key: "f", ctrlKey: true }))).toBe(true);
    expect(linux(keydown({ key: "F", ctrlKey: true, shiftKey: true }))).toBe(
      false,
    );
    expect(openSearch).toHaveBeenCalledOnce();
    expect(linux(keydown({ key: "Tab" }))).toBe(true);
    expect(linux(new KeyboardEvent("keyup", { key: "c", ctrlKey: true }))).toBe(
      true,
    );

    const mac = terminalKeyHandler(terminal, { copy, openSearch }, true);
    expect(mac(keydown({ key: "c", ctrlKey: true }))).toBe(true);
    expect(mac(keydown({ key: "c", metaKey: true }))).toBe(false);
    expect(mac(keydown({ key: "v", metaKey: true }))).toBe(false);
    expect(mac(keydown({ key: "f", metaKey: true }))).toBe(false);
  });

  test("appearance switch updates options.theme without recreating the terminal", async () => {
    const view = render(
      <TerminalSession
        processId="proc-theme"
        chatId="chat-a"
        onStatusChange={vi.fn()}
      />,
      { preloadedState: CONFIG_STATE },
    );

    await waitFor(() => expect(FakeTerminal.instances).toHaveLength(1));
    const terminal = FakeTerminal.instances[0];
    await waitFor(() =>
      expect(terminal.options.theme?.background).toBe("#0c0d0f"),
    );

    tokenState.current = LIGHT_TOKENS;
    view.rerender(
      <TerminalSession
        processId="proc-theme"
        chatId="chat-a"
        onStatusChange={vi.fn()}
      />,
    );

    await waitFor(() =>
      expect(terminal.options.theme?.background).toBe("#fcfcfd"),
    );
    expect(terminal.options.theme?.foreground).toBe("rgba(0, 0, 0, 0.88)");
    expect(FakeTerminal.instances).toHaveLength(1);
  });

  test("unresolvable tokens are omitted so xterm keeps its own dark defaults", async () => {
    tokenState.current = {
      "--rf-bg": "var(--missing)",
      "--rf-color-fg": "",
    };
    render(
      <TerminalSession
        processId="proc-theme"
        chatId="chat-a"
        onStatusChange={vi.fn()}
      />,
      { preloadedState: CONFIG_STATE },
    );

    await waitFor(() => expect(FakeTerminal.instances).toHaveLength(1));
    const constructed = FakeTerminal.instances[0].constructorOptions;
    expect(constructed.theme?.background).toBeUndefined();
    expect(constructed.theme?.foreground).toBeUndefined();
    expect(constructed.fontFamily).toBeUndefined();
    expect(constructed.fontSize).toBeUndefined();
  });

  test("focuses an existing terminal when focus is requested", async () => {
    const view = render(
      <TerminalSession
        processId="proc-focus"
        chatId="chat-a"
        focusRequest={0}
        onStatusChange={vi.fn()}
      />,
      { preloadedState: CONFIG_STATE },
    );
    await waitFor(() => expect(FakeTerminal.instances).toHaveLength(1));
    const terminal = FakeTerminal.instances[0];
    const callsAfterMount = terminal.focus.mock.calls.length;

    view.rerender(
      <TerminalSession
        processId="proc-focus"
        chatId="chat-a"
        focusRequest={1}
        onStatusChange={vi.fn()}
      />,
    );

    await waitFor(() =>
      expect(terminal.focus).toHaveBeenCalledTimes(callsAfterMount + 1),
    );
    expect(FakeTerminal.instances).toHaveLength(1);
  });

  test("renders non-TTY processes as read-only terminal mirrors", async () => {
    render(
      <TerminalSession
        processId="proc-read-only"
        chatId="chat-a"
        readOnly
        focusRequest={1}
        onStatusChange={vi.fn()}
      />,
      { preloadedState: CONFIG_STATE },
    );

    await waitFor(() => expect(FakeTerminal.instances).toHaveLength(1));
    expect(FakeTerminal.instances[0].constructorOptions).toMatchObject({
      convertEol: true,
      cursorBlink: false,
      cursorInactiveStyle: "none",
      disableStdin: true,
    });
    expect(FakeTerminal.instances[0].focus).not.toHaveBeenCalled();
    expect(FakeTerminal.instances[0].write).toHaveBeenCalledWith("\u001b[?25l");
    expect(useExecSessionMock).toHaveBeenCalledWith(
      expect.objectContaining({ interactive: false }),
    );
  });
});
