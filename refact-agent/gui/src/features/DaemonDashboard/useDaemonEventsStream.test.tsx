import { renderHook, waitFor } from "@testing-library/react";
import type { PropsWithChildren } from "react";
import { Provider } from "react-redux";
import { act } from "react-dom/test-utils";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { setUpStore, type AppStore } from "../../app/store";
import { useDaemonEventsStream } from "../../hooks/useDaemonEventsStream";
import { selectDaemonEvents } from "./dashboardSlice";

class MockEventSource {
  static readonly CONNECTING = 0;
  static readonly OPEN = 1;
  static readonly CLOSED = 2;
  static instances: MockEventSource[] = [];
  readonly CONNECTING = 0;
  readonly OPEN = 1;
  readonly CLOSED = 2;
  onerror: ((event: Event) => void) | null = null;
  onmessage: ((event: MessageEvent<string>) => void) | null = null;
  onopen: ((event: Event) => void) | null = null;
  readyState = MockEventSource.CONNECTING;
  url: string;
  private listeners = new Map<string, Set<EventListener>>();

  constructor(url: string | URL) {
    this.url = String(url);
    MockEventSource.instances.push(this);
  }

  addEventListener(type: string, listener: EventListener) {
    const listeners = this.listeners.get(type) ?? new Set<EventListener>();
    listeners.add(listener);
    this.listeners.set(type, listeners);
  }

  removeEventListener(type: string, listener: EventListener) {
    this.listeners.get(type)?.delete(listener);
  }

  close() {
    this.readyState = MockEventSource.CLOSED;
  }

  dispatchEvent(event: Event) {
    for (const listener of this.listeners.get(event.type) ?? []) {
      listener(event);
    }
    return true;
  }

  open() {
    this.readyState = MockEventSource.OPEN;
    this.onopen?.(new Event("open"));
  }

  emit(event: Record<string, unknown>) {
    const message = new MessageEvent<string>("message", {
      data: JSON.stringify(event),
    });
    this.onmessage?.(message);
  }
}

function wrapper(store: AppStore) {
  function StoreWrapper({ children }: PropsWithChildren) {
    return <Provider store={store}>{children}</Provider>;
  }
  return StoreWrapper;
}

function event(seq: number) {
  return {
    seq,
    ts_ms: seq,
    kind: "worker_status",
    project_id: "p1",
    payload: {},
  };
}

describe("useDaemonEventsStream", () => {
  beforeEach(() => {
    MockEventSource.instances = [];
    vi.stubGlobal("EventSource", MockEventSource);
  });

  afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  it("connects in follow mode and stores consecutive events", async () => {
    const store = setUpStore({
      config: {
        host: "web",
        lspPort: 8488,
        lspUrl: "https://daemon.example.test",
        themeProps: {},
      },
    });
    renderHook(() => useDaemonEventsStream(), { wrapper: wrapper(store) });

    expect(MockEventSource.instances[0].url).toBe(
      "https://daemon.example.test/daemon/v1/events?after_seq=0&follow=true",
    );
    act(() => {
      MockEventSource.instances[0].open();
      MockEventSource.instances[0].emit(event(1));
    });

    await waitFor(() => {
      expect(selectDaemonEvents(store.getState())).toEqual([event(1)]);
    });
  });

  it("reconnects immediately from the last sequence when a gap appears", () => {
    vi.useFakeTimers();
    const store = setUpStore({
      config: {
        host: "web",
        lspPort: 8488,
        lspUrl: "https://daemon.example.test",
        themeProps: {},
      },
    });
    const view = renderHook(() => useDaemonEventsStream(), {
      wrapper: wrapper(store),
    });
    const first = MockEventSource.instances[0];

    act(() => {
      first.emit(event(1));
      first.emit(event(3));
      vi.runOnlyPendingTimers();
    });

    expect(MockEventSource.instances).toHaveLength(2);
    expect(MockEventSource.instances[1].url).toContain("after_seq=1");
    expect(selectDaemonEvents(store.getState())).toEqual([event(1)]);

    view.unmount();
    expect(MockEventSource.instances[1].readyState).toBe(
      MockEventSource.CLOSED,
    );
  });
});
