import { http, HttpResponse } from "msw";
import { describe, expect, it, vi } from "vitest";

import type { DaemonWorker } from "../../../services/refact/daemon";
import { server } from "../../../utils/mockServer";
import { render, screen, waitFor } from "../../../utils/test-utils";
import { FirstRunWizard } from "./FirstRunWizard";

function worker(state: string): DaemonWorker {
  return {
    project_id: "refact",
    slug: "refact",
    root: "/work/refact",
    pinned: false,
    last_active_ms: 1,
    state,
    pid: state === "ready" ? 1 : null,
    http_port: state === "ready" ? 8001 : null,
    lsp_port: state === "ready" ? 9001 : null,
    lsp_clients: 0,
    busy_chats: 0,
    exec_running: 0,
    live_proxy_streams: 0,
    cron_next_fire_ms: null,
    idle_deadline_ms: null,
    last_status_report_ms: null,
    last_error: null,
  };
}

describe("FirstRunWizard", () => {
  it("calls onDone exactly once when setup finishes", async () => {
    const onDone = vi.fn();
    server.use(
      http.get("http://daemon.test/p/refact/v1/providers", () =>
        HttpResponse.json({
          providers: [
            {
              status: "active",
            },
          ],
        }),
      ),
    );
    const view = render(
      <FirstRunWizard
        daemonBase="http://daemon.test"
        hasChats={false}
        onDone={onDone}
        onProjectOpened={vi.fn()}
        userRequested
        workers={[worker("ready")]}
      />,
    );

    await view.user.click(
      await screen.findByRole("link", { name: "Start first chat" }),
    );

    await waitFor(() => expect(onDone).toHaveBeenCalledTimes(1));
  });

  it("calls onDone exactly once when setup is skipped", async () => {
    const onDone = vi.fn();
    const view = render(
      <FirstRunWizard
        daemonBase="http://daemon.test"
        hasChats={false}
        onDone={onDone}
        onProjectOpened={vi.fn()}
        userRequested={false}
        workers={[]}
      />,
    );

    await view.user.click(screen.getByRole("button", { name: "Skip setup" }));

    await waitFor(() => expect(onDone).toHaveBeenCalledTimes(1));
  });
});
