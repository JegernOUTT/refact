export const APP_MESSAGE_TYPE = "refact-app";
export const APP_PROTOCOL_VERSION = 1;
export const APP_SDK_VERSION = "1.0.0";
export const APP_MAX_CAPTURE_MESSAGE_CHARS = 12_000_000;
export const APP_MAX_INFLIGHT = 16;
export const APP_MAX_SEEN_IDS = 4096;

export type AppAppearance = "light" | "dark";
export type AppEnvelopeKind = "request" | "response" | "notification";
export type AppErrorPayload = { code: string; message: string };

export type AppEnvelope = {
  type: typeof APP_MESSAGE_TYPE;
  v: typeof APP_PROTOCOL_VERSION;
  channel: string;
  generation: number;
  kind: AppEnvelopeKind;
  id?: string;
  method?: string;
  params?: unknown;
  ok?: boolean;
  result?: unknown;
  error?: AppErrorPayload;
};

export type AppChildRequestParams = {
  ready: { sdkVersion: string };
  "state.get": Record<string, never>;
  "state.set": { state: unknown };
  "event.emit": { name: string; payload: unknown };
  "context.stage": { text: string; label?: string };
  "action.request": { name: string; payload: unknown };
};
export type AppChildMethod = keyof AppChildRequestParams;
export const APP_CHILD_METHODS: readonly AppChildMethod[] = [
  "ready",
  "state.get",
  "state.set",
  "event.emit",
  "context.stage",
  "action.request",
];

export type AppChildRequest = {
  [M in AppChildMethod]: {
    kind: "request";
    id: string;
    method: M;
    params: AppChildRequestParams[M];
  };
}[AppChildMethod];

export type AppReadyResult = {
  appearance: AppAppearance;
  state: unknown;
  revision?: number;
  data?: Record<string, unknown>;
};

export type AppHostNotificationParams = {
  theme: { appearance: AppAppearance; tokens?: Record<string, string> };
  state: { state: unknown; revision?: number };
  data: { name: string; payload: unknown };
};
export type AppHostNotificationMethod = keyof AppHostNotificationParams;

export type AppCaptureParams = {
  maxWidth?: number;
  maxHeight?: number;
  maxBytes?: number;
  redactSelectors?: string[];
};
export type AppHostRequestParams = { capture: AppCaptureParams };
export type AppHostRequestMethod = keyof AppHostRequestParams;

export type AppBinding = { channel: string; generation: number };

const NAME_RE = /^[A-Za-z0-9_.:-]{1,128}$/;
const ID_RE = /^[ch][0-9]{1,15}$/;
const MAX_TEXT = 200_000;
const MAX_LABEL = 200;

export function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isAppError(value: unknown): value is AppErrorPayload {
  return (
    isRecord(value) &&
    typeof value.code === "string" &&
    typeof value.message === "string"
  );
}

export function createAppChannel(): string {
  const bytes = new Uint8Array(16);
  crypto.getRandomValues(bytes);
  return Array.from(bytes, (b) => b.toString(16).padStart(2, "0")).join("");
}

export function parseAppMessage(
  data: unknown,
  binding: AppBinding,
): AppEnvelope | null {
  if (!isRecord(data)) return null;
  if (data.type !== APP_MESSAGE_TYPE || data.v !== APP_PROTOCOL_VERSION) {
    return null;
  }
  if (data.channel !== binding.channel) return null;
  if (data.generation !== binding.generation) return null;
  const kind = data.kind;
  if (kind !== "request" && kind !== "response" && kind !== "notification") {
    return null;
  }
  if (kind !== "notification") {
    if (typeof data.id !== "string" || !ID_RE.test(data.id)) return null;
  }
  if (kind !== "response") {
    if (typeof data.method !== "string" || !NAME_RE.test(data.method)) {
      return null;
    }
  } else {
    if (typeof data.ok !== "boolean") return null;
    if (!data.ok && !isAppError(data.error)) return null;
  }
  const env: AppEnvelope = {
    type: APP_MESSAGE_TYPE,
    v: APP_PROTOCOL_VERSION,
    channel: binding.channel,
    generation: binding.generation,
    kind,
  };
  if (typeof data.id === "string") env.id = data.id;
  if (typeof data.method === "string") env.method = data.method;
  if ("params" in data) env.params = data.params;
  if (kind === "response") {
    env.ok = data.ok as boolean;
    if (env.ok) env.result = data.result;
    else env.error = data.error as AppErrorPayload;
  }
  return env;
}

export function sanitizeChildRequest(env: AppEnvelope): AppChildRequest | null {
  if (env.kind !== "request" || !env.id || !env.id.startsWith("c")) {
    return null;
  }
  const p = isRecord(env.params) ? env.params : {};
  const id = env.id;
  switch (env.method) {
    case "ready":
      return {
        kind: "request",
        id,
        method: "ready",
        params: {
          sdkVersion:
            typeof p.sdkVersion === "string"
              ? p.sdkVersion.slice(0, 32)
              : "unknown",
        },
      };
    case "state.get":
      return { kind: "request", id, method: "state.get", params: {} };
    case "state.set":
      if (!("state" in p)) return null;
      return {
        kind: "request",
        id,
        method: "state.set",
        params: { state: p.state },
      };
    case "event.emit":
      if (typeof p.name !== "string" || !NAME_RE.test(p.name)) return null;
      return {
        kind: "request",
        id,
        method: "event.emit",
        params: { name: p.name, payload: p.payload },
      };
    case "context.stage": {
      if (typeof p.text !== "string" || p.text.length === 0) return null;
      if (p.text.length > MAX_TEXT) return null;
      const params: { text: string; label?: string } = { text: p.text };
      if (typeof p.label === "string") {
        params.label = p.label.slice(0, MAX_LABEL);
      }
      return { kind: "request", id, method: "context.stage", params };
    }
    case "action.request":
      if (typeof p.name !== "string" || !NAME_RE.test(p.name)) return null;
      return {
        kind: "request",
        id,
        method: "action.request",
        params: { name: p.name, payload: p.payload },
      };
    default:
      return null;
  }
}

export function makeAppEnvelope(
  binding: AppBinding,
  fields: Omit<AppEnvelope, "type" | "v" | "channel" | "generation">,
): AppEnvelope {
  return {
    type: APP_MESSAGE_TYPE,
    v: APP_PROTOCOL_VERSION,
    channel: binding.channel,
    generation: binding.generation,
    ...fields,
  };
}

export function makeAppNotification<M extends AppHostNotificationMethod>(
  binding: AppBinding,
  method: M,
  params: AppHostNotificationParams[M],
): AppEnvelope {
  return makeAppEnvelope(binding, { kind: "notification", method, params });
}

export function makeAppResponse(
  binding: AppBinding,
  id: string,
  outcome:
    | { ok: true; result: unknown }
    | { ok: false; error: AppErrorPayload },
): AppEnvelope {
  return outcome.ok
    ? makeAppEnvelope(binding, {
        kind: "response",
        id,
        ok: true,
        result: outcome.result,
      })
    : makeAppEnvelope(binding, {
        kind: "response",
        id,
        ok: false,
        error: outcome.error,
      });
}

export function makeAppRequest<M extends AppHostRequestMethod>(
  binding: AppBinding,
  id: string,
  method: M,
  params: AppHostRequestParams[M],
): AppEnvelope {
  return makeAppEnvelope(binding, { kind: "request", id, method, params });
}

export function approxMessageChars(value: unknown): number {
  try {
    return JSON.stringify(value).length;
  } catch {
    return Number.POSITIVE_INFINITY;
  }
}

export type AppChildHandlers = {
  [M in AppChildMethod]?: (
    params: AppChildRequestParams[M],
    request: Extract<AppChildRequest, { method: M }>,
  ) => unknown;
};

export type AppBridgeHostOptions = AppBinding & {
  getFrame: () => HTMLIFrameElement | null;
  handlers: AppChildHandlers;
  onRejected?: (reason: string, data: unknown) => void;
};

type Pending = {
  resolve: (value: unknown) => void;
  reject: (error: Error) => void;
  timer: ReturnType<typeof setTimeout>;
};

export class AppBridgeError extends Error {
  constructor(
    public code: string,
    message: string,
  ) {
    super(message);
    this.name = "AppBridgeError";
  }
}

export class AppBridgeHost {
  private target: Window | null = null;
  private seq = 0;
  private epoch = 0;
  private ready = false;
  private pending = new Map<string, Pending>();
  private inflight = new Set<string>();
  private seenChildIds = new Set<string>();
  private readonly binding: AppBinding;

  constructor(private readonly options: AppBridgeHostOptions) {
    this.binding = { channel: options.channel, generation: options.generation };
  }

  get channel(): string {
    return this.binding.channel;
  }

  get generation(): number {
    return this.binding.generation;
  }

  get isReady(): boolean {
    return this.ready;
  }

  attach(target: Window = window): () => void {
    this.detach();
    this.target = target;
    target.addEventListener("message", this.onMessage);
    return () => this.detach();
  }

  detach(): void {
    if (this.target) this.target.removeEventListener("message", this.onMessage);
    this.target = null;
    this.epoch += 1;
    this.ready = false;
    this.inflight.clear();
    this.seenChildIds.clear();
    for (const [id, p] of this.pending) {
      clearTimeout(p.timer);
      p.reject(new AppBridgeError("detached", `bridge detached before ${id}`));
    }
    this.pending.clear();
  }

  notify<M extends AppHostNotificationMethod>(
    method: M,
    params: AppHostNotificationParams[M],
  ): boolean {
    return this.post(makeAppNotification(this.binding, method, params));
  }

  request<M extends AppHostRequestMethod>(
    method: M,
    params: AppHostRequestParams[M],
    timeoutMs = 15_000,
  ): Promise<unknown> {
    this.seq += 1;
    const id = `h${this.seq}`;
    if (!this.ready) {
      return Promise.reject(
        new AppBridgeError(
          "not_ready",
          "app has not completed ready handshake",
        ),
      );
    }
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id);
        reject(new AppBridgeError("timeout", `${method} timed out`));
      }, timeoutMs);
      this.pending.set(id, { resolve, reject, timer });
      if (!this.post(makeAppRequest(this.binding, id, method, params))) {
        clearTimeout(timer);
        this.pending.delete(id);
        reject(new AppBridgeError("no_frame", "app frame is not available"));
      }
    });
  }

  private post(env: AppEnvelope): boolean {
    if (!this.target) return false;
    const win = this.options.getFrame()?.contentWindow;
    if (!win) return false;
    win.postMessage(env, "*");
    return true;
  }

  private reject(reason: string, data: unknown) {
    this.options.onRejected?.(reason, data);
  }

  private onMessage = (event: MessageEvent) => {
    const frameWin = this.options.getFrame()?.contentWindow ?? null;
    if (!frameWin || event.source !== frameWin) return;
    if (event.origin !== "null") {
      this.reject("origin", event.origin);
      return;
    }
    const chars = approxMessageChars(event.data);
    if (!Number.isFinite(chars)) {
      this.reject("unserializable", null);
      return;
    }
    if (
      isRecord(event.data) &&
      event.data.kind === "response" &&
      chars > APP_MAX_CAPTURE_MESSAGE_CHARS
    ) {
      this.reject("size", null);
      return;
    }
    const env = parseAppMessage(event.data, this.binding);
    if (!env) {
      this.reject("envelope", event.data);
      return;
    }
    if (env.kind === "response") {
      const p = env.id ? this.pending.get(env.id) : undefined;
      if (!p || !env.id) {
        this.reject("unknown_response", env.id);
        return;
      }
      this.pending.delete(env.id);
      clearTimeout(p.timer);
      if (env.ok) p.resolve(env.result);
      else
        p.reject(
          new AppBridgeError(
            env.error?.code ?? "error",
            env.error?.message ?? "app error",
          ),
        );
      return;
    }
    if (env.kind !== "request") {
      this.reject("unexpected_kind", env.kind);
      return;
    }
    const req = sanitizeChildRequest(env);
    if (req && this.seenChildIds.has(req.id)) {
      this.reject("duplicate_id", req.id);
      return;
    }
    if (req) {
      if (this.seenChildIds.size >= APP_MAX_SEEN_IDS) this.seenChildIds.clear();
      this.seenChildIds.add(req.id);
    }
    if (req && req.method !== "ready" && !this.ready) {
      this.post(
        makeAppResponse(this.binding, req.id, {
          ok: false,
          error: { code: "not_ready", message: "ready handshake required" },
        }),
      );
      return;
    }
    if (req && this.inflight.size >= APP_MAX_INFLIGHT) {
      this.post(
        makeAppResponse(this.binding, req.id, {
          ok: false,
          error: { code: "busy", message: "too many outstanding requests" },
        }),
      );
      return;
    }
    if (!req) {
      if (env.id) {
        this.post(
          makeAppResponse(this.binding, env.id, {
            ok: false,
            error: {
              code: "invalid_request",
              message: `invalid ${env.method ?? ""}`,
            },
          }),
        );
      }
      return;
    }
    void this.dispatch(req);
  };

  private async dispatch(req: AppChildRequest) {
    const handler = this.options.handlers[req.method] as
      | ((params: unknown, request: AppChildRequest) => unknown)
      | undefined;
    if (!handler) {
      if (req.method === "ready") this.ready = true;
      this.post(
        makeAppResponse(this.binding, req.id, {
          ok: false,
          error: { code: "unsupported", message: `${req.method} not granted` },
        }),
      );
      return;
    }
    const epoch = this.epoch;
    this.inflight.add(req.id);
    try {
      const result = await handler(req.params, req);
      if (epoch !== this.epoch) return;
      if (req.method === "ready") this.ready = true;
      this.post(
        makeAppResponse(this.binding, req.id, {
          ok: true,
          result: result ?? null,
        }),
      );
    } catch (err) {
      if (epoch !== this.epoch) return;
      const code = err instanceof AppBridgeError ? err.code : "handler_error";
      const message = err instanceof Error ? err.message : String(err);
      this.post(
        makeAppResponse(this.binding, req.id, {
          ok: false,
          error: { code, message },
        }),
      );
    } finally {
      if (epoch === this.epoch) this.inflight.delete(req.id);
    }
  }
}

export type AppSdkRuntimeConfig = AppBinding & {
  appearance: AppAppearance;
  initialState: unknown;
  sdkVersion: string;
};

export type AppCaptureRuntime = (
  params: unknown,
  env: { contextTypes: WeakMap<HTMLCanvasElement, string> },
) => Promise<unknown>;

/* Serialized into the sandboxed document; must stay self-contained. */
export function appSdkRuntime(
  config: AppSdkRuntimeConfig,
  capture: AppCaptureRuntime,
): void {
  const T = "refact-app";
  const parentWin = window.parent;
  const pending = new Map<
    string,
    { resolve: (v: unknown) => void; reject: (e: Error) => void }
  >();
  const listeners = new Map<string, Set<(payload: unknown) => void>>();
  const contextTypes = new WeakMap<HTMLCanvasElement, string>();
  let seq = 0;
  let appearance = config.appearance;
  let state: unknown = config.initialState;
  let stateRevision = -1;
  let stateEpoch = 0;
  let readyPromise: Promise<unknown> | null = null;

  const proto = HTMLCanvasElement.prototype;
  // eslint-disable-next-line @typescript-eslint/unbound-method
  const originalGetContext = proto.getContext;
  Object.defineProperty(proto, "getContext", {
    configurable: false,
    writable: false,
    value: function (
      this: HTMLCanvasElement,
      type: string,
      ...rest: unknown[]
    ) {
      const ctx = (originalGetContext as (...a: unknown[]) => unknown).call(
        this,
        type,
        ...rest,
      );
      if (ctx && !contextTypes.has(this)) contextTypes.set(this, String(type));
      return ctx;
    },
  });

  const isObj = (v: unknown): v is Record<string, unknown> =>
    typeof v === "object" && v !== null && !Array.isArray(v);

  const acceptState = (
    value: unknown,
    revision: unknown,
    epoch = stateEpoch,
  ) => {
    if (
      typeof revision === "number" &&
      Number.isSafeInteger(revision) &&
      revision >= 0
    ) {
      if (revision <= stateRevision) return false;
      stateRevision = revision;
    } else if (epoch !== stateEpoch) return false;
    state = value;
    stateEpoch += 1;
    return true;
  };

  const applyAppearance = () => {
    const root = document.documentElement;
    root.setAttribute("data-refact-appearance", appearance);
    root.style.colorScheme = appearance;
    const meta = document.querySelector('meta[name="color-scheme"]');
    if (meta) meta.setAttribute("content", appearance);
  };
  applyAppearance();

  // CSSOM setProperty on the host-owned :root rule: values cannot escape it.
  const TOKEN_NAME = /^--rf-[a-z0-9-]{1,64}$/;
  const TOKEN_VALUE = /^[A-Za-z0-9 #%(),.\-_'"/+*]{1,240}$/;
  const TOKEN_UNSAFE =
    /\/\*|\*\/|(?:url|image|image-set|cross-fade|element|expression|attr|src|env)\s*\(/i;
  const applyTokens = (next: unknown) => {
    if (!isObj(next)) return;
    const el = document.getElementById("refact-app-theme-tokens");
    const sheet = el instanceof HTMLStyleElement ? el.sheet : null;
    const rule = sheet?.cssRules[0];
    if (!(rule instanceof CSSStyleRule)) return;
    const accepted = new Map<string, string>();
    for (const name of Object.keys(next)) {
      const value = next[name];
      if (!TOKEN_NAME.test(name) || typeof value !== "string") continue;
      if (!TOKEN_VALUE.test(value) || TOKEN_UNSAFE.test(value)) continue;
      accepted.set(name, value);
    }
    const owned: string[] = [];
    for (let i = 0; i < rule.style.length; i += 1)
      owned.push(rule.style.item(i));
    for (const name of owned)
      if (!accepted.has(name)) rule.style.removeProperty(name);
    accepted.forEach((value, name) => {
      if (rule.style.getPropertyValue(name).trim() !== value)
        rule.style.setProperty(name, value);
    });
  };

  const post = (msg: Record<string, unknown>) => {
    parentWin.postMessage(
      {
        type: T,
        v: 1,
        channel: config.channel,
        generation: config.generation,
        ...msg,
      },
      "*",
    );
  };

  const emitLocal = (name: string, payload: unknown) => {
    const set = listeners.get(name);
    if (!set) return;
    for (const fn of Array.from(set)) {
      try {
        fn(payload);
      } catch (err) {
        setTimeout(() => {
          throw err;
        });
      }
    }
  };

  const request = (method: string, params: unknown): Promise<unknown> => {
    seq += 1;
    const id = "c" + String(seq);
    return new Promise((resolve, reject) => {
      pending.set(id, { resolve, reject });
      post({ kind: "request", id, method, params });
    });
  };

  window.addEventListener("message", (event: MessageEvent) => {
    if (event.source !== parentWin) return;
    const d: unknown = event.data;
    if (!isObj(d) || d.type !== T || d.v !== 1) return;
    if (d.channel !== config.channel || d.generation !== config.generation)
      return;
    if (d.kind === "response" && typeof d.id === "string") {
      const p = pending.get(d.id);
      if (!p) return;
      pending.delete(d.id);
      if (d.ok === true) p.resolve(d.result);
      else {
        const e = isObj(d.error) ? d.error : {};
        const err = new Error(
          typeof e.message === "string" ? e.message : "app request failed",
        );
        (err as Error & { code?: unknown }).code = e.code;
        p.reject(err);
      }
      return;
    }
    if (d.kind === "notification" && typeof d.method === "string") {
      const params = isObj(d.params) ? d.params : {};
      if (d.method === "theme") {
        if (params.appearance === "light" || params.appearance === "dark") {
          appearance = params.appearance;
          applyAppearance();
          if ("tokens" in params) applyTokens(params.tokens);
          emitLocal("theme", { appearance });
        }
      } else if (d.method === "state") {
        if (acceptState(params.state, params.revision))
          emitLocal("state", { state, revision: params.revision });
      } else if (d.method === "data") {
        if (typeof params.name === "string") {
          emitLocal("data", { name: params.name, payload: params.payload });
          emitLocal("data:" + params.name, params.payload);
        }
      }
      return;
    }
    if (
      d.kind === "request" &&
      typeof d.id === "string" &&
      d.id.startsWith("h")
    ) {
      const id = d.id;
      if (d.method === "capture") {
        capture(d.params, { contextTypes }).then(
          (result) => post({ kind: "response", id, ok: true, result }),
          (err: unknown) =>
            post({
              kind: "response",
              id,
              ok: false,
              error: {
                code: "capture_failed",
                message: String(err instanceof Error ? err.message : err),
              },
            }),
        );
      } else {
        post({
          kind: "response",
          id,
          ok: false,
          error: { code: "unsupported", message: "unknown host request" },
        });
      }
    }
  });

  const ready = () => {
    if (!readyPromise) {
      const epoch = stateEpoch;
      readyPromise = request("ready", { sdkVersion: config.sdkVersion }).then(
        (r) => {
          if (isObj(r)) {
            if (r.appearance === "light" || r.appearance === "dark") {
              appearance = r.appearance;
              applyAppearance();
            }
            if ("tokens" in r) applyTokens(r.tokens);
            if ("state" in r) acceptState(r.state, r.revision, epoch);
          }
          return r;
        },
      );
    }
    return readyPromise;
  };

  const api = Object.freeze({
    version: config.sdkVersion,
    ready,
    get appearance() {
      return appearance;
    },
    get state() {
      return state;
    },
    getState: () =>
      ready().then(() => {
        const epoch = stateEpoch;
        return request("state.get", {}).then((r) => {
          const value = isObj(r) && "state" in r ? r.state : r;
          acceptState(value, isObj(r) ? r.revision : undefined, epoch);
          return state;
        });
      }),
    setState: (next: unknown) =>
      ready().then(() => {
        const epoch = stateEpoch;
        return request("state.set", { state: next }).then((r) => {
          acceptState(
            isObj(r) && "state" in r ? r.state : next,
            isObj(r) ? r.revision : undefined,
            epoch,
          );
          return r;
        });
      }),
    emit: (name: string, payload?: unknown) =>
      ready().then(() => request("event.emit", { name, payload })),
    stageContext: (text: string, label?: string) =>
      ready().then(() =>
        request(
          "context.stage",
          label === undefined ? { text } : { text, label },
        ),
      ),
    requestAction: (name: string, payload?: unknown) =>
      ready().then(() => request("action.request", { name, payload })),
    on: (name: string, handler: (payload: unknown) => void) => {
      let set = listeners.get(name);
      if (!set) {
        set = new Set();
        listeners.set(name, set);
      }
      set.add(handler);
      return () => {
        set.delete(handler);
      };
    },
    off: (name: string, handler: (payload: unknown) => void) => {
      listeners.get(name)?.delete(handler);
    },
  });
  Object.defineProperty(window, "refactApp", {
    value: api,
    configurable: false,
    writable: false,
    enumerable: true,
  });
  void ready().catch(() => undefined);
}
