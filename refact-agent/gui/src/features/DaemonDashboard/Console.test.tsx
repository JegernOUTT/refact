import { http, HttpResponse } from "msw";
import { describe, expect, it, vi } from "vitest";
import { setUpStore } from "../../app/store";
import { server } from "../../utils/mockServer";
import { render, screen, within } from "../../utils/test-utils";
import { EnginesPage } from "./EnginesPage";
import { ActiveChatsPage } from "./ActiveChatsPage";
import { QuickTour } from "./QuickTour";
import { selectDashboardNavigation } from "./dashboardSlice";
import { LogTailPane } from "./Activity/LogTailPane";

const config = {
  host: "web" as const,
  lspPort: 8488,
  lspUrl: "https://daemon.example.test",
  themeProps: {},
};
const engine = {
  id: "attached:one",
  kind: "attached",
  label: "IDE engine",
  project_id: null,
  root: "/workspace",
  state: "ready",
  instance_id: null,
  http_port: 8001,
  observe_only: true,
  log_stream: false,
  logs_unavailable_reason: "Unsupported",
  observed_at_ms: 1,
  error: null,
};
function enginesHandler() {
  return http.get("*/daemon/v1/engines", () =>
    HttpResponse.json({
      schema_version: 1,
      observed_at_ms: 1,
      engines: [engine],
    }),
  );
}

describe("daemon console observation", () => {
  it("offers loopback-only connect and disconnects only the attachment", async () => {
    const detached = vi.fn();
    const attached = vi.fn();
    server.use(
      enginesHandler(),
      http.delete("*/daemon/v1/engines/:id", ({ params }) => {
        detached(params.id);
        return HttpResponse.json({ detached: true });
      }),
      http.post("*/daemon/v1/engines/attach", async ({ request }) => {
        attached(await request.json());
        return HttpResponse.json(engine);
      }),
    );
    const view = render(<EnginesPage />, { store: setUpStore({ config }) });
    await screen.findByText("IDE engine");
    expect(
      screen.queryByRole("button", { name: /Restart|Stop/ }),
    ).not.toBeInTheDocument();
    await view.user.click(screen.getByRole("button", { name: "Disconnect" }));
    expect(detached).toHaveBeenCalledWith("attached:one");
    await view.user.click(
      screen.getByRole("button", { name: "Connect engine" }),
    );
    const dialog = screen.getByRole("dialog");
    expect(within(dialog).queryByLabelText(/token|password/i)).toBeNull();
    await view.user.clear(within(dialog).getByLabelText("Port"));
    await view.user.type(within(dialog).getByLabelText("Port"), "65536");
    expect(
      within(dialog).getByRole("button", { name: "Connect" }),
    ).toBeDisabled();
    await view.user.clear(within(dialog).getByLabelText("Port"));
    await view.user.type(within(dialog).getByLabelText("Port"), "8001");
    await view.user.click(
      within(dialog).getByRole("button", { name: "Connect" }),
    );
    expect(attached).toHaveBeenCalledWith({ host: "127.0.0.1", port: 8001 });
  });

  it("keeps same chat ids distinct across engines, filters states, and exposes unknown coverage", async () => {
    server.use(
      enginesHandler(),
      http.get("*/daemon/v1/chats/live", () =>
        HttpResponse.json({
          schema_version: 1,
          observed_at_ms: 1,
          engines: ["one", "two", "three"].map((id, i) => ({
            engine_id: id,
            status: i === 2 ? "unsupported" : "available",
            coverage: "loaded_sessions",
            observed_at_ms: 1,
            error: i === 2 ? "Coverage unknown" : null,
            chats:
              i < 2
                ? [
                    {
                      id: "same",
                      title: i === 0 ? "Working chat" : "Waiting chat",
                      mode: "agent",
                      session_state:
                        i === 0 ? "generating" : "waiting_user_input",
                      paused: false,
                      project_root: "/workspace",
                      updated_at_ms: 1,
                      kind: "chat",
                    },
                  ]
                : [],
          })),
        }),
      ),
    );
    const view = render(<ActiveChatsPage />, { store: setUpStore({ config }) });
    await screen.findByText("Working chat");
    expect(screen.getByText("Waiting chat")).toBeInTheDocument();
    expect(screen.getAllByText("same")).toHaveLength(2);
    expect(screen.getByText("Coverage unknown")).toBeInTheDocument();
    await view.user.click(screen.getByRole("button", { name: "Waiting" }));
    expect(screen.queryByText("Working chat")).toBeNull();
    expect(screen.getByText("Waiting chat")).toBeInTheDocument();
  });

  it("replays the manual tour, navigates back, skips and finishes without actions", async () => {
    const store = setUpStore({ config });
    const view = render(<QuickTour />, { store });
    expect(screen.queryByRole("dialog")).toBeNull();
    await view.user.click(screen.getByRole("button", { name: "Quick tour" }));
    expect(selectDashboardNavigation(store.getState()).page).toBe("projects");
    await view.user.click(screen.getByRole("button", { name: "Next" }));
    expect(selectDashboardNavigation(store.getState()).page).toBe("engines");
    await view.user.click(screen.getByRole("button", { name: "Back" }));
    expect(selectDashboardNavigation(store.getState()).page).toBe("projects");
    await view.user.click(screen.getByRole("button", { name: "Skip" }));
    expect(screen.queryByRole("dialog")).toBeNull();
    await view.user.click(screen.getByRole("button", { name: "Quick tour" }));
    for (let step = 0; step < 3; step++)
      await view.user.click(screen.getByRole("button", { name: "Next" }));
    expect(selectDashboardNavigation(store.getState()).page).toBe("chats");
    await view.user.click(screen.getByRole("button", { name: "Finish" }));
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("never requests logs for an attached target and closes sources on unmount", () => {
    server.use(enginesHandler());
    const closed = vi.fn();
    const opened = vi.fn();
    class Source {
      onopen = null;
      onmessage = null;
      onerror = null;
      constructor(url: string) {
        opened(url);
      }
      close() {
        closed();
      }
    }
    vi.stubGlobal("EventSource", Source);
    const view = render(
      <LogTailPane workers={[]} projectIdParam="attached:one" />,
      { store: setUpStore({ config }) },
    );
    expect(screen.getByText(/Live logs unavailable/)).toBeInTheDocument();
    expect(opened).not.toHaveBeenCalled();
    view.unmount();
    const managed = render(<LogTailPane workers={[]} />, {
      store: setUpStore({ config }),
    });
    expect(opened).toHaveBeenCalledTimes(1);
    managed.unmount();
    expect(closed).toHaveBeenCalledTimes(1);
    vi.unstubAllGlobals();
  });
});
