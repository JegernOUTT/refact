import React from "react";
import { createRoot } from "react-dom/client";
import { Provider } from "react-redux";
import { http, HttpResponse } from "msw";
import { setupWorker } from "msw/browser";
import { setUpStore } from "../../src/app/store";
import { Theme } from "../../src/components/Theme";
import { DaemonDashboardShell } from "../../src/features/DaemonDashboard/DaemonDashboardShell";
import { ActivityRail } from "../../src/features/Workspace/ActivityRail/ActivityRail";
import { navigateDashboard } from "../../src/features/DaemonDashboard/dashboardSlice";
import type { EngineRow } from "../../src/services/refact/daemon";
import "../../src/lib/render/web.css";
declare global {
  interface Window {
    daemonFixtureLogLines: number;
  }
}
window.daemonFixtureLogLines = 0;
const now = Date.now();
const workers = ["refact", "design-system", "research-notes"].map(
  (slug, i) => ({
    project_id: `p${i}`,
    slug,
    root: `/home/developer/projects/${slug}`,
    root_exists: true,
    pinned: i === 0,
    last_active_ms: now,
    state: i === 2 ? "stopped" : "ready",
    pid: i === 2 ? null : 1234 + i,
    http_port: i === 2 ? null : 8001 + i,
    lsp_port: null,
    lsp_clients: i === 0 ? 1 : 0,
    busy_chats: i === 0 ? 2 : 0,
    exec_running: 0,
    live_proxy_streams: 0,
    cron_next_fire_ms: null,
    idle_deadline_ms: null,
    last_status_report_ms: now,
    last_error: null,
  }),
);
let engines: EngineRow[] = workers.map((worker) => ({
  id: `managed:${worker.project_id}`,
  kind: "managed",
  label: worker.slug,
  project_id: worker.project_id,
  root: worker.root,
  state: worker.state,
  instance_id: null,
  http_port: worker.http_port,
  observe_only: false,
  log_stream: true,
  logs_unavailable_reason: null,
  observed_at_ms: now,
  error: null,
}));
engines.push({
  ...engines[0],
  id: "attached:fixture",
  kind: "attached",
  label: "IDE engine",
  project_id: null,
  root: "/home/developer/sandbox",
  observe_only: true,
  log_stream: false,
  logs_unavailable_reason: "Live logs unavailable",
});
function streamResponse(kind: string, initial: string) {
  let timer: ReturnType<typeof setInterval>;
  let line = 0;
  const encoder = new TextEncoder();
  const stream = new ReadableStream<Uint8Array>({
    start(controller) {
      controller.enqueue(encoder.encode(initial));
      timer = setInterval(() => {
        line += 1;
        if (kind === "log") window.daemonFixtureLogLines += 1;
        controller.enqueue(
          encoder.encode(
            kind === "log"
              ? `data: fixture heartbeat ${line}\n\n`
              : `event: daemon\ndata: ${JSON.stringify({
                  seq: line + 1,
                  ts_ms: Date.now(),
                  kind: "worker_status",
                  project_id: "p0",
                  payload: { message: "Worker observation refreshed" },
                })}\n\n`,
          ),
        );
      }, 3000);
    },
    cancel() {
      clearInterval(timer);
    },
  });
  return new HttpResponse(stream, {
    headers: {
      "Content-Type": "text/event-stream",
      "Cache-Control": "no-cache",
    },
  });
}
const worker = setupWorker(
  http.get("*/v1/apps", () => HttpResponse.json({ apps: [] })),
  http.get("*/v1/apps/subscribe", () => streamResponse("event", ": ready\n\n")),
  http.get("*/v1/git/status", () => HttpResponse.json({ roots: [] })),
  http.get("*/v1/chats/activity", () => HttpResponse.json({ chats: [] })),
  http.get("*/daemon/v1/status", () =>
    HttpResponse.json({
      pid: 42,
      port: 8488,
      version: "8.7.0",
      uptime_secs: 3600,
      started_at_ms: now - 3600000,
      workers: 2,
      cron_pending: {},
    }),
  ),
  http.get("*/daemon/v1/workers", () => HttpResponse.json(workers)),
  http.get("*/daemon/v1/engines", () =>
    HttpResponse.json({
      schema_version: 1,
      observed_at_ms: Date.now(),
      engines,
    }),
  ),
  http.post("*/daemon/v1/engines/attach", async ({ request }) => {
    const body = (await request.json()) as { label?: string; port: number };
    const engine = {
      ...engines[3],
      id: `attached:${Date.now()}`,
      label: body.label || "Local engine",
      http_port: body.port,
    };
    engines.push(engine);
    return HttpResponse.json(engine);
  }),
  http.delete("*/daemon/v1/engines/:id", ({ params }) => {
    engines = engines.filter((engine) => engine.id !== params.id);
    return HttpResponse.json({ detached: true });
  }),
  http.get("*/daemon/v1/chats/live", () =>
    HttpResponse.json({
      schema_version: 1,
      observed_at_ms: Date.now(),
      engines: engines.map((engine, i) => ({
        engine_id: engine.id,
        status: i === 2 ? "stopped" : i === 3 ? "unsupported" : "available",
        coverage: "loaded_sessions",
        observed_at_ms: now,
        error:
          i === 3
            ? "Engine version does not support live session observation"
            : null,
        chats:
          i < 2
            ? [
                {
                  id: "same-id-across-engines",
                  title:
                    i === 0
                      ? "Improve daemon console"
                      : "Review component tokens",
                  mode: "agent",
                  session_state: i === 0 ? "generating" : "waiting_user_input",
                  paused: false,
                  project_root: engine.root,
                  updated_at_ms: now,
                  kind: "chat",
                },
              ]
            : [],
      })),
    }),
  ),
  http.get("*/daemon/v1/events", ({ request }) => {
    const initial = `event: daemon\ndata: ${JSON.stringify({
      seq: 1,
      ts_ms: now,
      kind: "worker_ready",
      project_id: "p0",
      payload: { message: "Refact worker ready" },
    })}\n\n`;
    return new URL(request.url).searchParams.get("follow") === "true"
      ? streamResponse("event", initial)
      : HttpResponse.text(initial, {
          headers: { "Content-Type": "text/event-stream" },
        });
  }),
  http.get("*/daemon/v1/logs/stream", () =>
    streamResponse("log", "data: daemon console fixture connected\n\n"),
  ),
  http.get("*/daemon/v1/logs", () =>
    HttpResponse.text("daemon console fixture log\n"),
  ),
);
await worker.start({ onUnhandledRequest: "error", quiet: true });
const params = new URLSearchParams(location.search);
const showRail = params.get("rail") === "true";
if (showRail) window.history.replaceState(null, "", "/p/fixture/?rail=true");
const store = setUpStore({
  config: {
    host: "web",
    dev: true,
    engineServed: showRail,
    lspUrl: location.origin,
    lspPort: 4173,
    themeProps: {
      appearance: params.get("theme") === "light" ? "light" : "dark",
    },
  },
});
const page = params.get("page");
if (page === "engines" || page === "chats" || page === "activity")
  store.dispatch(navigateDashboard({ page, params: {} }));
createRoot(document.getElementById("refact-chat")!).render(
  <Provider store={store}>
    <Theme>{showRail ? <ActivityRail /> : <DaemonDashboardShell />}</Theme>
  </Provider>,
);
