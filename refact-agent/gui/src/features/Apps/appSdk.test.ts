import { afterEach, describe, expect, it, vi } from "vitest";
import {
  AppBridgeHost,
  createAppChannel,
  makeAppEnvelope,
  parseAppMessage,
  sanitizeChildRequest,
  type AppEnvelope,
} from "./appSdk";
import { buildAppDocument } from "./appDocument";

const binding = { channel: "abcdef0123456789", generation: 3 };

function env(fields: Partial<AppEnvelope>): Record<string, unknown> {
  return { ...makeAppEnvelope(binding, { kind: "request" }), ...fields };
}

describe("parseAppMessage", () => {
  it("accepts a valid request", () => {
    const e = parseAppMessage(
      env({ id: "c1", method: "ready", params: {} }),
      binding,
    );
    expect(e?.method).toBe("ready");
  });
  it("rejects stale channel, generation, type and bad ids", () => {
    expect(
      parseAppMessage(
        env({ id: "c1", method: "ready", channel: "x" }),
        binding,
      ),
    ).toBeNull();
    expect(
      parseAppMessage(
        env({ id: "c1", method: "ready", generation: 2 }),
        binding,
      ),
    ).toBeNull();
    expect(
      parseAppMessage(
        { ...env({ id: "c1", method: "ready" }), type: "other" },
        binding,
      ),
    ).toBeNull();
    expect(
      parseAppMessage(env({ id: "x1", method: "ready" }), binding),
    ).toBeNull();
    expect(
      parseAppMessage(env({ kind: "response", id: "h1" }), binding),
    ).toBeNull();
    expect(parseAppMessage("str", binding)).toBeNull();
  });
});

describe("sanitizeChildRequest", () => {
  it("strips unknown params so child cannot self-grant host ids", () => {
    const e = parseAppMessage(
      env({
        id: "c2",
        method: "context.stage",
        params: { text: "hi", chatId: "evil", viewId: "v" },
      }),
      binding,
    );
    expect(e && sanitizeChildRequest(e)).toEqual({
      kind: "request",
      id: "c2",
      method: "context.stage",
      params: { text: "hi" },
    });
  });
  it("rejects unknown methods and host-prefixed ids", () => {
    const a = parseAppMessage(env({ id: "c3", method: "host.exec" }), binding);
    expect(a && sanitizeChildRequest(a)).toBeNull();
    const b = parseAppMessage(env({ id: "h3", method: "ready" }), binding);
    expect(b && sanitizeChildRequest(b)).toBeNull();
    const c = parseAppMessage(
      env({ id: "c4", method: "event.emit", params: { name: "bad name" } }),
      binding,
    );
    expect(c && sanitizeChildRequest(c)).toBeNull();
  });
});

describe("AppBridgeHost", () => {
  afterEach(() => vi.useRealTimers());

  function setup(handlers = {}) {
    const posted: unknown[] = [];
    const contentWindow = {
      postMessage: (m: unknown) => posted.push(m),
    } as unknown as Window;
    const frame = { contentWindow } as HTMLIFrameElement;
    const target = new EventTarget() as unknown as Window;
    const rejected: string[] = [];
    const host = new AppBridgeHost({
      ...binding,
      getFrame: () => frame,
      handlers: {
        ready: () => ({ appearance: "light", state: null }),
        ...handlers,
      },
      onRejected: (r) => rejected.push(r),
    });
    host.attach(target);
    const send = (
      data: unknown,
      source: unknown = contentWindow,
      origin = "null",
    ) =>
      target.dispatchEvent(
        new MessageEvent("message", { data, source: source as Window, origin }),
      );
    return { host, posted, send, rejected, contentWindow };
  }
  const flush = () => new Promise((r) => setTimeout(r, 0));

  it("ignores foreign sources and non-null origins", async () => {
    const { send, posted, rejected } = setup();
    send(env({ id: "c1", method: "ready" }), {});
    send(env({ id: "c1", method: "ready" }), undefined, "https://evil");
    await flush();
    expect(posted).toEqual([]);
    expect(rejected).toEqual(["origin"]);
  });

  it("gates methods until ready and dedupes ids", async () => {
    const stage = vi.fn(() => "ok");
    const { send, posted, rejected, host } = setup({ "context.stage": stage });
    send(env({ id: "c1", method: "context.stage", params: { text: "a" } }));
    await flush();
    expect((posted[0] as AppEnvelope).error?.code).toBe("not_ready");
    send(env({ id: "c2", method: "ready", params: {} }));
    await flush();
    expect(host.isReady).toBe(true);
    send(env({ id: "c3", method: "context.stage", params: { text: "a" } }));
    send(env({ id: "c3", method: "context.stage", params: { text: "a" } }));
    await flush();
    expect(stage).toHaveBeenCalledTimes(1);
    expect(rejected).toContain("duplicate_id");
  });

  it("drops late handler results after detach", async () => {
    let release: (v: unknown) => void = () => undefined;
    const { send, posted, host } = setup({
      "state.get": () => new Promise((r) => (release = r)),
    });
    send(env({ id: "c1", method: "ready" }));
    await flush();
    send(env({ id: "c2", method: "state.get" }));
    await flush();
    host.detach();
    release({ state: 1 });
    await flush();
    expect(posted.filter((m) => (m as AppEnvelope).id === "c2")).toEqual([]);
  });

  it("correlates host requests by id and rejects unknown responses", async () => {
    const { send, posted, host, rejected } = setup();
    await expect(host.request("capture", {})).rejects.toMatchObject({
      code: "not_ready",
    });
    send(env({ id: "c1", method: "ready" }));
    await flush();
    const p = host.request("capture", {});
    const req = posted.at(-1) as AppEnvelope;
    expect(req.id).toMatch(/^h\d+$/);
    send(env({ kind: "response", id: "h999", ok: true, result: 1 }));
    send(
      env({
        kind: "response",
        id: req.id,
        ok: true,
        result: { status: "error" },
      }),
    );
    await expect(p).resolves.toEqual({ status: "error" });
    expect(rejected).toContain("unknown_response");
  });

  it("times out host requests", async () => {
    const { send, host } = setup();
    send(env({ id: "c1", method: "ready" }));
    await flush();
    vi.useFakeTimers();
    const p = host.request("capture", {}, 50);
    vi.advanceTimersByTime(60);
    await expect(p).rejects.toMatchObject({ code: "timeout" });
  });

  it("creates distinct channels", () => {
    expect(createAppChannel()).not.toBe(createAppChannel());
    expect(createAppChannel()).toMatch(/^[0-9a-f]{32}$/);
  });
});

describe("appSdkRuntime theme tokens", () => {
  it("updates CSS variables in place and rejects unsafe values", async () => {
    const channel = "abcdef0123456789";
    const doc = buildAppDocument({
      channel,
      generation: 1,
      appearance: "dark",
      html: '<input id="draft" value="typed">',
      themeTokens: { "--rf-bg": "#0c0d0f" },
    });
    const frame = document.createElement("iframe");
    document.body.appendChild(frame);
    const win = frame.contentWindow;
    if (!win) throw new Error("no frame window");
    const posted = vi.spyOn(win.parent, "postMessage");
    win.document.open();
    win.document.write(doc);
    win.document.close();
    await new Promise((r) => setTimeout(r, 0));
    const draft = win.document.getElementById("draft") as HTMLInputElement;
    draft.value = "local edit";
    const style = win.document.getElementById("refact-app-theme-tokens");
    const sheet = (style as HTMLStyleElement | null)?.sheet;
    const rule = sheet?.cssRules[0] as CSSStyleRule | undefined;
    if (!rule) throw new Error("token rule missing");
    win.dispatchEvent(
      new MessageEvent("message", {
        source: win.parent,
        data: {
          type: "refact-app",
          v: 1,
          channel,
          generation: 1,
          kind: "notification",
          method: "theme",
          params: {
            appearance: "light",
            tokens: {
              "--rf-bg": "#fcfcfd",
              "--rf-color-accent": "#006adc",
              "--rf-border-strong": "#808080",
              "--rf-space-2": "8px",
              "--rf-color-fg": "red;}body{display:none",
              "--rf-color-muted": "url(x)",
            },
          },
        },
      }),
    );
    expect(rule.style.getPropertyValue("--rf-bg").trim()).toBe("#fcfcfd");
    expect(rule.style.getPropertyValue("--rf-color-accent").trim()).toBe(
      "#006adc",
    );
    expect(rule.style.getPropertyValue("--rf-border-strong").trim()).toBe(
      "#808080",
    );
    expect(rule.style.getPropertyValue("--rf-space-2").trim()).toBe("8px");
    expect(
      win.document.getElementById("refact-app-theme-base")?.textContent,
    ).toContain(
      "scrollbar-color:var(--rf-app-scrollbar-thumb) var(--rf-app-scrollbar-track)",
    );
    expect(rule.style.getPropertyValue("--rf-color-fg")).toBe("");
    expect(rule.style.getPropertyValue("--rf-color-muted")).toBe("");
    expect(
      win.document.documentElement.getAttribute("data-refact-appearance"),
    ).toBe("light");
    win.dispatchEvent(
      new MessageEvent("message", {
        source: win.parent,
        data: {
          type: "refact-app",
          v: 1,
          channel,
          generation: 1,
          kind: "notification",
          method: "theme",
          params: { appearance: "dark", tokens: { "--rf-bg": "#0c0d0f" } },
        },
      }),
    );
    expect(rule.style.getPropertyValue("--rf-bg").trim()).toBe("#0c0d0f");
    expect(rule.style.getPropertyValue("--rf-color-accent")).toBe("");
    expect(win.document.getElementById("draft")).toBe(draft);
    expect(draft.value).toBe("local edit");
    type Sdk = {
      ready: () => Promise<unknown>;
      getState: () => Promise<unknown>;
      setState: (state: unknown) => Promise<unknown>;
      readonly state: unknown;
      on: (name: string, listener: (payload: unknown) => void) => void;
    };
    const sdk = (win as unknown as { refactApp: Sdk }).refactApp;
    const stateChanged = vi.fn((payload: unknown) => {
      draft.value = String(
        (payload as { state: { count: number } }).state.count,
      );
    });
    sdk.on("state", stateChanged);
    const send = (fields: Record<string, unknown>) =>
      win.dispatchEvent(
        new MessageEvent("message", {
          source: win.parent,
          data: { type: "refact-app", v: 1, channel, generation: 1, ...fields },
        }),
      );
    const latestRequest = (method: string) => {
      const request = posted.mock.calls
        .map(([data]) => data as unknown)
        .reverse()
        .find(
          (data) =>
            typeof data === "object" &&
            data !== null &&
            (data as AppEnvelope).method === method,
        ) as AppEnvelope | undefined;
      if (!request) throw new Error(`missing ${method}`);
      return request.id;
    };
    send({
      kind: "notification",
      method: "state",
      params: { state: { count: 2 }, revision: 2 },
    });
    draft.value = "unsaved draft";
    send({
      kind: "notification",
      method: "state",
      params: { state: { count: 2 }, revision: 2 },
    });
    expect(draft.value).toBe("unsaved draft");
    expect(stateChanged).toHaveBeenCalledTimes(1);
    send({
      kind: "response",
      id: "c1",
      ok: true,
      result: { state: { count: 0 }, revision: 0 },
    });
    await sdk.ready();
    expect(sdk.state).toEqual({ count: 2 });
    const get = sdk.getState();
    const set = sdk.setState({ count: 3 });
    await new Promise((resolve) => setTimeout(resolve, 0));
    send({
      kind: "notification",
      method: "state",
      params: { state: { count: 4 }, revision: 4 },
    });
    send({
      kind: "response",
      id: latestRequest("state.set"),
      ok: true,
      result: { state: { count: 3 }, revision: 3 },
    });
    await expect(set).resolves.toEqual({ state: { count: 3 }, revision: 3 });
    send({
      kind: "response",
      id: latestRequest("state.get"),
      ok: true,
      result: { state: { count: 1 }, revision: 4 },
    });
    await expect(get).resolves.toEqual({ count: 4 });
    expect(sdk.state).toEqual({ count: 4 });
    expect(draft.value).toBe("4");
    expect(stateChanged).toHaveBeenCalledTimes(2);
    frame.remove();
    posted.mockRestore();
  });
});
