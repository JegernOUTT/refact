import { describe, expect, it } from "vitest";
import {
  APP_TOOL_NAMES,
  buildAppToolModel,
  sanitizeAppValue,
} from "./appToolModel";

// Shapes serialized by apps/commands.rs, apps/protocol.rs and tool_app.rs.
const app = {
  slug: "weather",
  manifest: {
    app_id: "a",
    name: "Weather",
    description: "Forecast",
    entrypoint: "ui/index.html",
    schema_version: 1,
  },
  revision: "draft-2",
  active_revision: "published-1",
  files: ["app.json", "ui/index.html"],
};
const workspace = {
  revision: 2,
  bindings: [
    { app_id: "a", binding_id: "b", params: { city: "London" }, state: {} },
  ],
  layout: {
    kind: "split",
    direction: "horizontal",
    sizes: [1, 1],
    children: [
      { kind: "leaf", tabs: [{ kind: "chat", ref: "self" }], active: 0 },
      { kind: "leaf", tabs: [{ kind: "app", binding_id: "b" }], active: 0 },
    ],
  },
};
const view = {
  view_id: "v",
  app_id: "a",
  revision: "published-1",
  generation: "g",
  host: "vscode",
  visible: true,
  ready: false,
  focused: false,
  stale: false,
};
const fixtures: Record<(typeof APP_TOOL_NAMES)[number], unknown> = {
  app,
  app_build: { app_id: "a", revision: "published-2", manifest: app.manifest },
  app_state: { revision: 2, value: { count: 3 } },
  app_binding: workspace,
  app_layout: workspace,
  app_view: [view],
  app_source: {
    source: {
      source_id: "weather",
      channel: "weather",
      driver: {
        type: "http_poll",
        url: "https://example.com/forecast?token=HIDDEN",
        every_ms: 60000,
      },
      permissions: [],
    },
    revision: "draft-2",
    source_records: [],
  },
  app_event: [
    {
      generation: "g",
      seq: 2,
      kind: "app_event",
      data: { app_id: "a", channel: "weather", data: { temperature: 21 } },
    },
  ],
  app_inspect: {
    app,
    sources: [],
    views: [view],
    events: [],
    diagnostics_version: 1,
    limitations: ["Viewer presence does not prove SDK readiness"],
  },
  app_screenshot: [
    {
      m_type: "text",
      m_content: JSON.stringify({
        app_id: "a",
        view_id: "v",
        revision: "published-1",
        generation: "g",
        status: "ok",
        trust: "app_reported",
        method: "dom_raster",
        metadata: { width: 800, height: 600 },
      }),
    },
    { m_type: "image/png", m_content: "iVBORw0KGgo=" },
  ],
};
const model = (
  name: string,
  content: unknown,
  action = "get",
  failed = false,
) =>
  buildAppToolModel(
    name,
    JSON.stringify({ action, password: "ARG_SECRET" }),
    typeof content === "string" || Array.isArray(content)
      ? content
      : JSON.stringify(content),
    true,
    failed,
  );

describe("App result adapter", () => {
  it("presents all ten real result shapes and t_ aliases without raw header JSON", () => {
    for (const name of APP_TOOL_NAMES) {
      for (const alias of [name, `t_${name}`]) {
        const result = model(alias, fixtures[name]);
        expect(result.status, alias).toBe("done");
        expect(
          result.sections.length + result.facts.length,
          alias,
        ).toBeGreaterThan(0);
        expect(result.title).not.toContain("ARG_SECRET");
        expect(JSON.stringify(result)).not.toContain("HIDDEN");
      }
    }
  });
  it("distinguishes request, publication, source configuration and last-good-data errors", () => {
    expect(model("app", app, "clone")).toMatchObject({
      title: "App · Clone",
      status: "done",
    });
    expect(
      model("app_view", { status: "requested", request_id: "r" }, "open")
        .outcome,
    ).toBe("Requested · not confirmed");
    expect(model("app_build", fixtures.app_build, "publish").outcome).toBe(
      "Published · not runtime verified",
    );
    expect(model("app_source", fixtures.app_source, "configure").outcome).toBe(
      "Configured · health unknown",
    );
    expect(
      model("app_source", {
        source_id: "weather",
        status: "error",
        last_error: "upstream timeout",
        last_good_data: { temperature: 21 },
      }).status,
    ).toBe("failed");
    const visible = model("app_view", [view]);
    expect(visible.outcome).not.toMatch(/opened|focused|ready/i);
    expect(visible.sections.flatMap((section) => section.rows)).toContainEqual({
      label: "Ready",
      value: "false",
    });
  });
  it("keeps unknown, malformed, partial, missing and failed results truthful", () => {
    for (const input of [
      "{broken",
      "future result",
      null,
      42,
      { future: true },
    ])
      expect(model("app", input).status).toBe("unknown");
    expect(model("app", { revision: "r", partial: true }).outcome).toBe(
      "Partial result",
    );
    expect(
      model("app_screenshot", "capture timed out", "capture", true).outcome,
    ).toBe("Tool failed");
    expect(buildAppToolModel("app", "{", undefined, false, false).outcome).toBe(
      "Not recorded",
    );
  });
  it("retains actual raster image parts and treats SVG fallback as artifact, never a URL preview", () => {
    expect(model("app_screenshot", fixtures.app_screenshot).images).toEqual([
      "data:image/png;base64,iVBORw0KGgo=",
    ]);
    const svg = model(
      "app_screenshot",
      '{"status":"svg_fallback","trust":"app_reported","revision":"r"}\nCapture artifact (original media type, not a PNG): data:image/svg+xml;base64,PHN2Zz4=',
    );
    expect(svg.outcome).toBe("SVG artifact · not PNG");
    expect(svg.images).toEqual([]);
    for (const input of [
      { data_url: "https://evil.test/a.png?token=SECRET" },
      [{ m_type: "image/svg+xml", m_content: "PHN2Zz4=" }],
      [{ m_type: "image/png", m_content: "https://evil.test" }],
    ])
      expect(model("app_screenshot", input).images).toEqual([]);
  });
  it("redacts sensitive keys recursively and strips endpoint paths, userinfo and query tokens", () => {
    const clean = sanitizeAppValue({
      view_token: "HOST_SECRET",
      state: { api_key: "API_SECRET", nested: [{ password: "PASS_SECRET" }] },
      driver: {
        url: "https://user:password@example.com/private?q=QUERY_SECRET#frag",
      },
    });
    expect(JSON.stringify(clean)).not.toMatch(
      /HOST_SECRET|API_SECRET|PASS_SECRET|QUERY_SECRET|user:password|private/,
    );
    expect(JSON.stringify(clean)).toContain("https://example.com");
    const summary = model("app_state", {
      revision: 1,
      value: { secret: "STATE_SECRET", count: 2 },
    });
    expect(JSON.stringify(summary)).not.toContain("STATE_SECRET");
  });
  it("keeps malformed actions unknown and app-state payloads neutral", () => {
    expect(
      buildAppToolModel("app", "{", '{"revision":"r1"}', true, false).status,
    ).toBe("unknown");
    expect(
      model("app_state", {
        revision: 1,
        value: { status: "failed", error: "user note" },
      }).status,
    ).toBe("done");
    expect(
      model("app_inspect", {
        diagnostics_version: 1,
        diagnostics: { info: "normal" },
      }).status,
    ).toBe("done");
    const nested = model("app", {
      revision: "r",
      content: JSON.stringify({ host_view_token: "DO_NOT_SHOW" }),
    });
    expect(JSON.stringify(nested)).not.toContain("DO_NOT_SHOW");
  });
});
