import { screen, waitFor } from "@testing-library/react";
import { http, HttpResponse } from "msw";
import { beforeEach, describe, expect, test, vi } from "vitest";

import { render } from "../../../utils/test-utils";
import { server } from "../../../utils/mockServer";
import { TerminalPanel } from "./TerminalPanel";

class FakeEventSource {
  static instances: FakeEventSource[] = [];

  onopen: ((event: Event) => void) | null = null;
  onerror: ((event: Event) => void) | null = null;
  close = vi.fn();
  private readonly listeners = new Map<string, EventListener[]>();

  constructor(_url: string | URL) {
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
}

const CONFIG_STATE = {
  config: {
    host: "web" as const,
    lspPort: 8001,
    apiKey: null,
    themeProps: {},
  },
};

describe("TerminalPanel", () => {
  beforeEach(() => {
    FakeEventSource.instances = [];
    vi.stubGlobal("EventSource", FakeEventSource);
    vi.spyOn(window, "confirm").mockReturnValue(true);
  });

  test("reattaches running PTYs and seeds backfill before streaming", async () => {
    server.use(
      http.get("*/v1/exec/list", () =>
        HttpResponse.json({
          processes: [
            {
              process_id: "reattach-123456",
              status: "running",
              command_preview: "bash -l",
              created_at_ms: 1,
              tty: true,
              service_name: null,
            },
            {
              process_id: "background",
              status: "running",
              command_preview: "task",
              created_at_ms: 2,
              tty: false,
              service_name: null,
            },
          ],
        }),
      ),
      http.get("*/v1/exec/reattach-123456/read", () =>
        HttpResponse.json({
          chunks: [{ seq: 0, stream: "combined", text: "history" }],
          next_seq: 1,
          status: "running",
        }),
      ),
      http.post("*/v1/exec/reattach-123456/resize", () =>
        HttpResponse.json({}),
      ),
    );

    render(<TerminalPanel />, { preloadedState: CONFIG_STATE });

    expect(
      await screen.findByRole("tab", { name: /bash · reattach/i }),
    ).toBeVisible();
    await waitFor(() => expect(FakeEventSource.instances).toHaveLength(1));
    expect(screen.queryByText("background")).not.toBeInTheDocument();
  });

  test("keeps terminal sessions mounted while switching internal tabs", async () => {
    server.use(
      http.get("*/v1/exec/list", () =>
        HttpResponse.json({
          processes: ["first-123456", "second-12345"].map((process_id) => ({
            process_id,
            status: "running",
            command_preview: "bash -l",
            created_at_ms: 1,
            tty: true,
            service_name: null,
          })),
        }),
      ),
      http.get("*/v1/exec/:processId/read", () =>
        HttpResponse.json({ chunks: [], next_seq: 0, status: "running" }),
      ),
      http.post("*/v1/exec/:processId/resize", () => HttpResponse.json({})),
    );

    const { container, user } = render(<TerminalPanel />, {
      preloadedState: CONFIG_STATE,
    });
    await screen.findByRole("tab", { name: /bash · first/i });
    const first = container.querySelector(
      '[data-terminal-process-id="first-123456"]',
    );
    const second = container.querySelector(
      '[data-terminal-process-id="second-12345"]',
    );
    expect(first).toBeInTheDocument();
    expect(second).toBeInTheDocument();

    await user.click(screen.getByRole("tab", { name: /bash · first/i }));
    expect(
      container.querySelector('[data-terminal-process-id="first-123456"]'),
    ).toBe(first);
    expect(
      container.querySelector('[data-terminal-process-id="second-12345"]'),
    ).toBe(second);
  });

  test("spawns a login shell and kills a running session when closed", async () => {
    const spawnBodies: unknown[] = [];
    let killed = false;
    server.use(
      http.get("*/v1/exec/list", () => HttpResponse.json({ processes: [] })),
      http.post("*/v1/exec/spawn", async ({ request }) => {
        spawnBodies.push(await request.json());
        return HttpResponse.json({
          process_id: "spawned-1234",
          status: "running",
        });
      }),
      http.get("*/v1/exec/spawned-1234/read", () =>
        HttpResponse.json({ chunks: [], next_seq: 0, status: "running" }),
      ),
      http.post("*/v1/exec/spawned-1234/resize", () => HttpResponse.json({})),
      http.post("*/v1/exec/spawned-1234/kill", () => {
        killed = true;
        return HttpResponse.json({
          process_id: "spawned-1234",
          status: "killed",
        });
      }),
    );

    const { user } = render(<TerminalPanel />, {
      preloadedState: CONFIG_STATE,
    });
    await user.click(
      await screen.findByRole("button", { name: "New terminal" }),
    );

    expect(
      await screen.findByRole("tab", { name: /bash · spawned/i }),
    ).toBeVisible();
    expect(spawnBodies).toEqual([
      { command: "bash -l", pty: true, rows: 24, cols: 80 },
    ]);

    await user.click(
      screen.getByRole("button", { name: /Close bash · spawned/i }),
    );
    await waitFor(() => expect(killed).toBe(true));
    expect(window.confirm).toHaveBeenCalled();
    await waitFor(() =>
      expect(
        screen.queryByRole("tab", { name: /bash · spawned/i }),
      ).not.toBeInTheDocument(),
    );
  });

  test("shows an honest disabled state for a 403 spawn response", async () => {
    server.use(
      http.get("*/v1/exec/list", () => HttpResponse.json({ processes: [] })),
      http.post("*/v1/exec/spawn", () =>
        HttpResponse.text("exec HTTP is disabled", { status: 403 }),
      ),
    );

    const { user } = render(<TerminalPanel />, {
      preloadedState: CONFIG_STATE,
    });
    await user.click(
      await screen.findByRole("button", { name: "New terminal" }),
    );

    expect(await screen.findByText("Browser terminal disabled")).toBeVisible();
    expect(screen.getByText(/REFACT_DISABLE_EXEC_HTTP policy/i)).toBeVisible();
  });
});
