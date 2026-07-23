import { waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";

type TerminalSessionComponent =
  typeof import("./TerminalSession").TerminalSession;
type RenderFn = typeof import("../../../utils/test-utils").render;

type FakeTerminalOptions = {
  cursorBlink?: boolean;
  fontFamily?: string;
  theme?: Record<string, string | undefined>;
};

class FakeTerminal {
  static instances: FakeTerminal[] = [];
  readonly constructorOptions: FakeTerminalOptions;
  options: FakeTerminalOptions = {};
  rows = 24;
  cols = 80;
  loadAddon = (): undefined => undefined;
  open = (): undefined => undefined;
  focus = (): undefined => undefined;
  dispose = (): undefined => undefined;
  onData = () => ({ dispose: (): undefined => undefined });
  write = (): undefined => undefined;

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
};

const LIGHT_TOKENS: Record<string, string> = {
  ...DARK_TOKENS,
  "--rf-bg": "#fcfcfd",
  "--rf-color-fg": "rgba(0, 0, 0, 0.88)",
};

const tokenState = { current: DARK_TOKENS };

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
  vi.doMock("@xterm/xterm", () => ({ Terminal: FakeTerminal }));
  vi.doMock("@xterm/addon-fit", () => ({
    FitAddon: class {
      fit = (): undefined => undefined;
    },
  }));
  vi.doMock("./useExecSession", () => ({
    useExecSession: () => ({ error: null, reconnecting: false }),
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
      <TerminalSession processId="proc-theme" onStatusChange={vi.fn()} />,
      { preloadedState: CONFIG_STATE },
    );

    await waitFor(() => expect(FakeTerminal.instances).toHaveLength(1));
    const constructed = FakeTerminal.instances[0].constructorOptions;
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
  });

  test("appearance switch updates options.theme without recreating the terminal", async () => {
    const view = render(
      <TerminalSession processId="proc-theme" onStatusChange={vi.fn()} />,
      { preloadedState: CONFIG_STATE },
    );

    await waitFor(() => expect(FakeTerminal.instances).toHaveLength(1));
    const terminal = FakeTerminal.instances[0];
    await waitFor(() =>
      expect(terminal.options.theme?.background).toBe("#0c0d0f"),
    );

    tokenState.current = LIGHT_TOKENS;
    view.rerender(
      <TerminalSession processId="proc-theme" onStatusChange={vi.fn()} />,
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
      <TerminalSession processId="proc-theme" onStatusChange={vi.fn()} />,
      { preloadedState: CONFIG_STATE },
    );

    await waitFor(() => expect(FakeTerminal.instances).toHaveLength(1));
    const constructed = FakeTerminal.instances[0].constructorOptions;
    expect(constructed.theme?.background).toBeUndefined();
    expect(constructed.theme?.foreground).toBeUndefined();
    expect(constructed.fontFamily).toBeUndefined();
  });
});
