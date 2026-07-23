import type { FitAddon } from "@xterm/addon-fit";
import type { Terminal } from "@xterm/xterm";
import { render, waitFor } from "@testing-library/react";
import { act } from "react-dom/test-utils";
import { http, HttpResponse } from "msw";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";

import type { Config } from "../../Config/configSlice";
import type { ExecStatus } from "../../../services/refact/exec";
import { server } from "../../../utils/mockServer";
import { useExecSession } from "./useExecSession";

const CONFIG: Config = {
  host: "web",
  lspPort: 8001,
  apiKey: null,
  themeProps: {},
};

class FakeEventSource {
  static instances: FakeEventSource[] = [];

  readonly url: string;
  onopen: ((event: Event) => void) | null = null;
  onerror: ((event: Event) => void) | null = null;
  close = vi.fn();
  private readonly listeners = new Map<string, EventListener[]>();

  constructor(url: string | URL) {
    this.url = String(url);
    FakeEventSource.instances.push(this);
  }

  addEventListener(type: string, listener: EventListener) {
    const listeners = this.listeners.get(type) ?? [];
    listeners.push(listener);
    this.listeners.set(type, listeners);
  }

  emit(type: string, data: unknown) {
    const event = new MessageEvent(type, { data: JSON.stringify(data) });
    for (const listener of this.listeners.get(type) ?? []) listener(event);
  }

  fail() {
    this.onerror?.(new Event("error"));
  }
}

class FakeResizeObserver {
  static disconnect = vi.fn();
  private readonly callback: ResizeObserverCallback;

  constructor(callback: ResizeObserverCallback) {
    this.callback = callback;
  }

  observe(target: Element) {
    void target;
    void this.callback;
  }
  unobserve(target: Element) {
    void target;
  }
  disconnect() {
    FakeResizeObserver.disconnect();
  }
}

type RuntimeFixture = {
  runtime: {
    terminal: Terminal;
    fitAddon: FitAddon;
    container: HTMLElement;
  };
  write: ReturnType<typeof vi.fn>;
  emitData: (value: string) => void;
  disposeInput: ReturnType<typeof vi.fn>;
  fit: ReturnType<typeof vi.fn>;
};

function makeRuntime(): RuntimeFixture {
  let dataHandler: ((value: string) => void) | null = null;
  const write = vi.fn();
  const disposeInput = vi.fn();
  const fit = vi.fn();
  const terminal = {
    rows: 40,
    cols: 120,
    write,
    onData: vi.fn((handler: (value: string) => void) => {
      dataHandler = handler;
      return { dispose: disposeInput };
    }),
  } as unknown as Terminal;

  return {
    runtime: {
      terminal,
      fitAddon: { fit } as unknown as FitAddon,
      container: document.createElement("div"),
    },
    write,
    emitData: (value) => dataHandler?.(value),
    disposeInput,
    fit,
  };
}

function Harness({
  runtime,
  onStatusChange,
  onResize,
}: {
  runtime: RuntimeFixture["runtime"];
  onStatusChange: (status: ExecStatus) => void;
  onResize?: (rows: number, cols: number) => void;
}) {
  const state = useExecSession({
    processId: "proc-1",
    runtime,
    connection: CONFIG,
    apiKey: undefined,
    onStatusChange,
    onResize,
  });
  return <div data-reconnecting={state.reconnecting}>{state.error}</div>;
}

describe("useExecSession", () => {
  beforeEach(() => {
    FakeEventSource.instances = [];
    FakeResizeObserver.disconnect.mockClear();
    vi.stubGlobal("EventSource", FakeEventSource);
    vi.stubGlobal("ResizeObserver", FakeResizeObserver);
  });

  afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  test("backfills, streams, batches stdin, resizes, and renders exit", async () => {
    const stdinBodies: unknown[] = [];
    const resizeBodies: unknown[] = [];
    server.use(
      http.get("*/v1/exec/proc-1/read", () =>
        HttpResponse.json({
          chunks: [{ seq: 0, stream: "combined", text: "backfill" }],
          next_seq: 1,
          status: "running",
        }),
      ),
      http.post("*/v1/exec/proc-1/stdin", async ({ request }) => {
        stdinBodies.push(await request.json());
        return HttpResponse.json({
          process_id: "proc-1",
          status: "running",
          bytes_written: 2,
          since_seq: 0,
          next_seq: 1,
          latest_seq: 1,
        });
      }),
      http.post("*/v1/exec/proc-1/resize", async ({ request }) => {
        resizeBodies.push(await request.json());
        return HttpResponse.json({});
      }),
    );
    const fixture = makeRuntime();
    const onStatusChange = vi.fn();
    const view = render(
      <Harness runtime={fixture.runtime} onStatusChange={onStatusChange} />,
    );

    await waitFor(() => expect(FakeEventSource.instances).toHaveLength(1));
    expect(fixture.write).toHaveBeenCalledWith("backfill");
    const source = FakeEventSource.instances[0];
    expect(source.url).toContain("since_seq=1");

    act(() => {
      source.emit("snapshot", {
        status: "running",
        chunks: [{ seq: 0, stream: "combined", text: "duplicate" }],
        next_seq: 1,
      });
      source.emit("output", { seq: 1, stream: "combined", text: "live" });
      fixture.emitData("a");
      fixture.emitData("b");
    });

    await waitFor(() => expect(stdinBodies).toEqual([{ chars: "ab" }]));
    await waitFor(() =>
      expect(resizeBodies).toEqual([{ rows: 40, cols: 120 }]),
    );
    expect(fixture.write).not.toHaveBeenCalledWith("duplicate");
    expect(fixture.write).toHaveBeenCalledWith("live");
    expect(fixture.fit).toHaveBeenCalled();

    act(() => {
      source.emit("exit", { process_id: "proc-1", status: "exited" });
    });
    expect(fixture.write).toHaveBeenCalledWith(
      "\r\n[process exited: exited]\r\n",
    );
    expect(onStatusChange).toHaveBeenLastCalledWith("exited");
    expect(source.close).toHaveBeenCalled();

    view.unmount();
    expect(fixture.disposeInput).toHaveBeenCalled();
    expect(FakeResizeObserver.disconnect).toHaveBeenCalled();
  });

  test("syncs the fitted PTY size before painting backfill on attach", async () => {
    const calls: string[] = [];
    server.use(
      http.get("*/v1/exec/proc-1/read", () => {
        calls.push("read");
        return HttpResponse.json({
          chunks: [{ seq: 0, stream: "combined", text: "backfill" }],
          next_seq: 1,
          status: "running",
        });
      }),
      http.post("*/v1/exec/proc-1/resize", () => {
        calls.push("resize");
        return HttpResponse.json({});
      }),
    );
    const fixture = makeRuntime();
    const onResize = vi.fn();
    render(
      <Harness
        runtime={fixture.runtime}
        onStatusChange={vi.fn()}
        onResize={onResize}
      />,
    );

    await waitFor(() => expect(FakeEventSource.instances).toHaveLength(1));
    expect(calls).toEqual(["resize", "read"]);
    expect(fixture.fit).toHaveBeenCalled();
    expect(onResize).toHaveBeenCalledWith(40, 120);
    expect(fixture.write).toHaveBeenCalledWith("backfill");
  });

  test("raw tty backfill preserves CRLF and advances the byte-offset cursor", async () => {
    const readUrls: string[] = [];
    server.use(
      http.get("*/v1/exec/proc-1/read", ({ request }) => {
        readUrls.push(request.url);
        return HttpResponse.json({
          chunks:
            readUrls.length === 1
              ? [{ seq: 0, stream: "combined", text: "tail\r\n", offset: 6 }]
              : [],
          next_seq: readUrls.length === 1 ? 6 : 11,
          status: "running",
        });
      }),
      http.post("*/v1/exec/proc-1/resize", () => HttpResponse.json({})),
    );
    const fixture = makeRuntime();
    const view = render(
      <Harness runtime={fixture.runtime} onStatusChange={vi.fn()} />,
    );

    await waitFor(() => expect(FakeEventSource.instances).toHaveLength(1));
    expect(readUrls[0]).toContain("raw=true");
    expect(readUrls[0]).toContain("since_seq=0");
    expect(fixture.write).toHaveBeenCalledWith("tail\r\n");
    const source = FakeEventSource.instances[0];
    expect(source.url).toContain("since_seq=6");

    act(() => {
      source.emit("output", {
        seq: 6,
        stream: "combined",
        text: "α\r\n",
        offset: 11,
      });
      source.emit("output", {
        seq: 6,
        stream: "combined",
        text: "stale-replay",
        offset: 9,
      });
    });
    expect(fixture.write).toHaveBeenCalledWith("α\r\n");
    expect(fixture.write).not.toHaveBeenCalledWith("stale-replay");

    act(() => {
      source.fail();
    });
    await waitFor(() => expect(readUrls.length).toBeGreaterThan(1), {
      timeout: 2_000,
    });
    expect(readUrls[1]).toContain("raw=true");
    expect(readUrls[1]).toContain("since_seq=11");
    await waitFor(() => expect(FakeEventSource.instances).toHaveLength(2));
    expect(FakeEventSource.instances[1].url).toContain("since_seq=11");

    view.unmount();
  });

  test("reconnects with sequence resume after an SSE error", async () => {
    let readCount = 0;
    server.use(
      http.get("*/v1/exec/proc-1/read", () => {
        readCount += 1;
        return HttpResponse.json({
          chunks:
            readCount === 1
              ? [{ seq: 0, stream: "combined", text: "first" }]
              : [{ seq: 2, stream: "combined", text: "recovered" }],
          next_seq: readCount === 1 ? 1 : 3,
          status: "running",
        });
      }),
      http.post("*/v1/exec/proc-1/resize", () => HttpResponse.json({})),
    );
    const fixture = makeRuntime();
    const view = render(
      <Harness runtime={fixture.runtime} onStatusChange={vi.fn()} />,
    );
    await waitFor(() => expect(FakeEventSource.instances).toHaveLength(1));

    FakeEventSource.instances[0].emit("output", {
      seq: 1,
      stream: "combined",
      text: "second",
    });
    FakeEventSource.instances[0].fail();

    await waitFor(() => expect(FakeEventSource.instances).toHaveLength(2), {
      timeout: 2_000,
    });
    expect(readCount).toBe(2);
    expect(fixture.write).toHaveBeenCalledWith("recovered");
    expect(FakeEventSource.instances[1].url).toContain("since_seq=3");

    view.unmount();
  });

  test("clears pending timers and closes the stream on unmount", async () => {
    server.use(
      http.get("*/v1/exec/proc-1/read", () =>
        HttpResponse.json({ chunks: [], next_seq: 0, status: "running" }),
      ),
      http.post("*/v1/exec/proc-1/resize", () => HttpResponse.json({})),
    );
    const fixture = makeRuntime();
    const view = render(
      <Harness runtime={fixture.runtime} onStatusChange={vi.fn()} />,
    );
    await waitFor(() => expect(FakeEventSource.instances).toHaveLength(1));

    vi.useFakeTimers();
    act(() => {
      fixture.emitData("pending");
      FakeEventSource.instances[0].fail();
    });
    expect(vi.getTimerCount()).toBeGreaterThan(0);
    view.unmount();
    expect(vi.getTimerCount()).toBe(0);
    expect(FakeEventSource.instances[0].close).toHaveBeenCalled();
  });
});
