import { act as rtlAct, renderHook, waitFor } from "@testing-library/react";
import type { Terminal } from "@xterm/xterm";
import type { FitAddon } from "@xterm/addon-fit";
import type { ExecReadResponse } from "../../../services/refact/exec";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";

import { terminalKeyHandler } from "./terminalKeys";

const act = rtlAct as typeof import("react-dom/test-utils").act;

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

describe("useExecSession attachment lifecycle", () => {
  let useExecSession: typeof import("./useExecSession").useExecSession;
  let ExecHttpError: typeof import("../../../services/refact/exec").ExecHttpError;
  const read = vi.fn<typeof import("../../../services/refact/exec").readExec>();
  const resize =
    vi.fn<typeof import("../../../services/refact/exec").resizeExec>();
  const stdin =
    vi.fn<typeof import("../../../services/refact/exec").writeProcessStdin>();
  const terminalWrite = vi.fn<(text: string) => void>();
  let sendData: (chars: string) => void;
  let notifyResize: () => void;
  let options: Parameters<typeof useExecSession>[0];
  let streams: TestEventSource[];

  class TestEventSource extends EventTarget {
    onopen: (() => void) | null = null;
    onerror: (() => void) | null = null;
    close = vi.fn();
    constructor(readonly url: string) {
      super();
      streams.push(this);
    }
  }

  const running: ExecReadResponse = {
    chunks: [{ seq: 0, offset: 4, stream: "combined", text: "old" }],
    next_seq: 4,
    status: "running",
  };

  beforeEach(async () => {
    vi.useFakeTimers();
    vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
    streams = [];
    terminalWrite.mockClear();
    read.mockReset().mockResolvedValue(running);
    resize.mockReset().mockResolvedValue({});
    stdin.mockReset().mockResolvedValue({
      process_id: "proc",
      status: "running",
      bytes_written: 1,
      since_seq: 0,
      next_seq: 0,
      latest_seq: 0,
    });
    vi.doUnmock("./useExecSession");
    vi.doMock("../../../services/refact/exec", async (importOriginal) => ({
      ...(await importOriginal<
        typeof import("../../../services/refact/exec")
      >()),
      readExec: read,
      resizeExec: resize,
      writeProcessStdin: stdin,
    }));
    ({ ExecHttpError } = await import("../../../services/refact/exec"));
    ({ useExecSession } = await import("./useExecSession"));
    vi.stubGlobal("EventSource", TestEventSource);
    vi.stubGlobal(
      "ResizeObserver",
      class {
        constructor(callback: () => void) {
          notifyResize = callback;
        }
        observe = vi.fn();
        disconnect = vi.fn();
      },
    );
    options = {
      processId: "proc",
      chatId: "chat",
      connection: { host: "web", lspPort: 8001 },
      onStatusChange: vi.fn(),
      runtime: {
        terminal: {
          rows: 24,
          cols: 80,
          write: terminalWrite,
          onData: (callback: (chars: string) => void) => {
            sendData = callback;
            return { dispose: vi.fn() };
          },
        } as unknown as Terminal,
        fitAddon: { fit: vi.fn() } as unknown as FitAddon,
        container: document.createElement("div"),
      },
    };
  });

  afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
    vi.doUnmock("../../../services/refact/exec");
  });

  async function settle() {
    await act(async () => {
      await Promise.resolve();
    });
  }

  test("writes the original command before output once across reconnect and context changes", async () => {
    const command = "printf 'actual command'";
    read.mockResolvedValue({ ...running, command });
    const { rerender, unmount } = renderHook(() => useExecSession(options));
    await settle();
    const header = `\u001b[2m$ ${command}\u001b[22m\r\n`;
    expect(terminalWrite.mock.calls).toEqual([[header], ["old"]]);
    expect(stdin).not.toHaveBeenCalled();
    expect(
      new URL(streams[0].url, "http://localhost").searchParams.get("since_seq"),
    ).toBe("4");

    act(() => {
      streams[0].dispatchEvent(
        new MessageEvent("snapshot", {
          data: JSON.stringify({ ...running, command }),
        }),
      );
      streams[0].onerror?.();
    });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(250);
    });
    expect(read.mock.calls[1][1]).toBe(4);
    options = { ...options, chatId: "other-chat" };
    rerender();
    await settle();
    expect(read.mock.calls[2][1]).toBe(4);
    expect(terminalWrite.mock.calls).toEqual([[header], ["old"]]);
    unmount();
  });

  test("normalizes command newlines and renders terminal controls as inert text", async () => {
    read.mockResolvedValue({
      ...running,
      command:
        "echo one\r\necho two\recho three\n\u001b[2J\u009d52;c;bad\u0007\t\b",
    });
    const { unmount } = renderHook(() => useExecSession(options));
    await settle();
    expect(terminalWrite.mock.calls).toEqual([
      [
        "\u001b[2m$ echo one\r\necho two\r\necho three\r\n\\x1b[2J\\x9d52;c;bad\\x07\\x09\\x08\u001b[22m\r\n",
      ],
      ["old"],
    ]);
    expect(stdin).not.toHaveBeenCalled();
    unmount();
  });

  test("accepts a snapshot command before the first output and prefixes a fresh terminal history", async () => {
    read.mockResolvedValue({ ...running, chunks: [], next_seq: 0 });
    const { rerender, unmount } = renderHook(() => useExecSession(options));
    await settle();
    const snapshot = { ...running, command: "echo original" };
    act(() => {
      streams[0].dispatchEvent(
        new MessageEvent("snapshot", { data: JSON.stringify(snapshot) }),
      );
    });
    const header = "\u001b[2m$ echo original\u001b[22m\r\n";
    expect(terminalWrite.mock.calls).toEqual([[header], ["old"]]);

    const freshWrite = vi.fn();
    const runtime = options.runtime;
    if (!runtime) throw new Error("test requires a terminal runtime");
    read.mockResolvedValue(snapshot);
    options = {
      ...options,
      runtime: {
        ...runtime,
        terminal: {
          ...runtime.terminal,
          write: freshWrite,
        } as unknown as Terminal,
      },
    };
    rerender();
    await settle();
    expect(freshWrite).toHaveBeenCalledExactlyOnceWith(header);
    expect(read.mock.calls[1][1]).toBe(4);
    unmount();
  });

  test("keeps older workers without command metadata compatible", async () => {
    const { unmount } = renderHook(() => useExecSession(options));
    await settle();
    act(() => {
      streams[0].dispatchEvent(
        new MessageEvent("snapshot", {
          data: JSON.stringify({ ...running, command: "late metadata" }),
        }),
      );
    });
    expect(terminalWrite).toHaveBeenCalledExactlyOnceWith("old");
    expect(stdin).not.toHaveBeenCalled();
    unmount();
  });

  test.each([401, 403, 404])(
    "stops retry and queued IO after read HTTP %s without inventing process exit",
    async (status) => {
      read.mockRejectedValue(new ExecHttpError("attachment denied", status));
      const { result, unmount } = renderHook(() => useExecSession(options));
      await settle();
      expect(result.current).toEqual({
        error: "attachment denied",
        reconnecting: false,
      });
      sendData("x");
      notifyResize();
      await act(async () => {
        await vi.advanceTimersByTimeAsync(20_000);
      });
      expect(read).toHaveBeenCalledOnce();
      expect(resize).toHaveBeenCalledOnce();
      expect(stdin).not.toHaveBeenCalled();
      expect(streams).toHaveLength(0);
      expect(options.onStatusChange).not.toHaveBeenCalled();
      expect(terminalWrite).not.toHaveBeenCalled();
      unmount();
    },
  );

  test("resize denial blocks initial read", async () => {
    let rejectResize: (cause: Error) => void = () => undefined;
    resize.mockImplementationOnce(
      () =>
        new Promise((_, reject) => {
          rejectResize = reject;
        }),
    );
    const { result, unmount } = renderHook(() => useExecSession(options));
    await settle();
    act(() => {
      rejectResize(new ExecHttpError("resize denied", 403));
    });
    await settle();
    expect(result.current).toEqual({
      error: "resize denied",
      reconnecting: false,
    });
    expect(read).not.toHaveBeenCalled();
    expect(streams).toHaveLength(0);
    unmount();
  });

  test.each([new TypeError("network down")])(
    "reconnects transient failures, deduplicates output and clears stale errors",
    async () => {
      const cause = new TypeError("network down");
      read
        .mockRejectedValueOnce(cause)
        .mockRejectedValueOnce(new ExecHttpError("unavailable", 503));
      const { result, unmount } = renderHook(() => useExecSession(options));
      await settle();
      expect(result.current.reconnecting).toBe(true);
      await act(async () => {
        await vi.advanceTimersByTimeAsync(750);
      });
      expect(read).toHaveBeenCalledTimes(3);
      expect(streams).toHaveLength(1);
      act(() => streams[0].onopen?.());
      expect(result.current).toEqual({ error: null, reconnecting: false });
      act(() => streams[0].onerror?.());
      await act(async () => {
        await vi.advanceTimersByTimeAsync(250);
      });
      expect(read.mock.calls[3][1]).toBe(4);
      expect(terminalWrite).toHaveBeenCalledExactlyOnceWith("old");
      unmount();
      const calls = read.mock.calls.length;
      act(() => streams[0].onerror?.());
      await act(async () => {
        await vi.advanceTimersByTimeAsync(20_000);
      });
      expect(read).toHaveBeenCalledTimes(calls);
    },
  );

  test("late backfill and stream callbacks cannot overwrite stdin denial, and changed context can attach", async () => {
    const { result, rerender, unmount } = renderHook(() =>
      useExecSession(options),
    );
    await settle();
    act(() => streams[0].onopen?.());
    let resolveRead: (value: ExecReadResponse) => void = () => undefined;
    read.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          resolveRead = resolve;
        }),
    );
    act(() => streams[0].onerror?.());
    await act(async () => {
      await vi.advanceTimersByTimeAsync(250);
    });
    stdin.mockRejectedValueOnce(new ExecHttpError("stdin denied", 403));
    act(() => {
      sendData("x");
    });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(16);
    });
    act(() => {
      resolveRead({
        ...running,
        next_seq: 8,
        chunks: [{ seq: 4, text: "late", stream: "combined" }],
      });
    });
    await settle();
    await settle();
    act(() => streams[0].onopen?.());
    expect(result.current).toEqual({
      error: "stdin denied",
      reconnecting: false,
    });
    expect(streams).toHaveLength(1);
    expect(terminalWrite).not.toHaveBeenCalledWith("late");
    expect(streams[0].close).toHaveBeenCalled();
    const calls = stdin.mock.calls.length;
    act(() => {
      sendData("blocked");
      notifyResize();
    });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(20_000);
    });
    expect(stdin).toHaveBeenCalledTimes(calls);
    options = { ...options, chatId: "other-chat" };
    rerender();
    await settle();
    expect(streams).toHaveLength(2);
    act(() => streams[1].onopen?.());
    expect(result.current).toEqual({ error: null, reconnecting: false });
    unmount();
  });
});
