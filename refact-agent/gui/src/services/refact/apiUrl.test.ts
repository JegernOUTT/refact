import { describe, expect, test } from "vitest";
import {
  buildApiUrl,
  buildApiUrlFromParts,
  buildApiUrlFromState,
  getEngineEndpointIdentity,
  hasUsableEngineEndpoint,
  normalizeEndpointPath,
  resolveEngineBaseUrl,
  sanitizeEngineBaseUrl,
  type EngineApiConfig,
} from "./apiUrl";
import type { RootState } from "../../app/store";

describe("sanitizeEngineBaseUrl", () => {
  test("trims trailing slash and strips query and hash", () => {
    expect(
      sanitizeEngineBaseUrl("  https://api.example.com/base/?x=1#hash  "),
    ).toBe("https://api.example.com/base");
  });

  test("preserves reverse proxy base before v1", () => {
    expect(sanitizeEngineBaseUrl("https://example.com/refact/v1/ping")).toBe(
      "https://example.com/refact",
    );
    expect(
      sanitizeEngineBaseUrl(
        "https://example.com/refact/proxy/v1/ping?x=1#hash",
      ),
    ).toBe("https://example.com/refact/proxy");
  });

  test("preserves earlier v1 proxy path segments", () => {
    expect(
      sanitizeEngineBaseUrl("https://example.com/api/v1/refact/v1/ping"),
    ).toBe("https://example.com/api/v1/refact");
  });

  test("strips stale v1 ping path back to the engine base", () => {
    expect(sanitizeEngineBaseUrl("http://localhost:5173/v1/ping/Refact")).toBe(
      "http://localhost:5173",
    );
  });

  test("rejects unsupported browser schemes and invalid non-empty values", () => {
    expect(sanitizeEngineBaseUrl("http2://127.0.0.1:8001")).toBeNull();
    expect(sanitizeEngineBaseUrl("ws://127.0.0.1:8001")).toBeNull();
    expect(sanitizeEngineBaseUrl("file:///tmp/refact")).toBeNull();
    expect(sanitizeEngineBaseUrl("not a url")).toBeNull();
  });

  test("returns null for empty values", () => {
    expect(sanitizeEngineBaseUrl(undefined)).toBeNull();
    expect(sanitizeEngineBaseUrl("   ")).toBeNull();
  });
});

describe("resolveEngineBaseUrl", () => {
  test("uses relative base for Vite dev web mode", () => {
    expect(resolveEngineBaseUrl({ host: "web", dev: true })).toBe("");
  });

  test("uses relative base for engine-served web mode", () => {
    expect(
      resolveEngineBaseUrl({
        host: "web",
        engineServed: true,
        lspUrl: "http://host:8001",
      }),
    ).toBe("");
  });

  test("uses sanitized standalone remote web base", () => {
    expect(
      resolveEngineBaseUrl({ host: "web", lspUrl: "http://remote:8001" }),
    ).toBe("http://remote:8001");
  });

  test("falls back to relative base for invalid or missing web lspUrl", () => {
    expect(resolveEngineBaseUrl({ host: "web" })).toBe("");
    expect(
      resolveEngineBaseUrl({ host: "web", lspUrl: "http2://remote:8001" }),
    ).toBe("");
  });

  test("falls back to loopback only for VS Code, JetBrains, and legacy IDE hosts", () => {
    expect(resolveEngineBaseUrl({ host: "vscode", lspPort: 8010 })).toBe(
      "http://127.0.0.1:8010",
    );
    expect(resolveEngineBaseUrl({ host: "jetbrains", lspPort: 8020 })).toBe(
      "http://127.0.0.1:8020",
    );
    expect(resolveEngineBaseUrl({ host: "ide", lspPort: 8030 })).toBe(
      "http://127.0.0.1:8030",
    );
  });

  test("prefers valid lspUrl over IDE loopback fallback", () => {
    expect(
      resolveEngineBaseUrl({
        host: "vscode",
        lspPort: 8010,
        lspUrl: "https://remote.example.com/refact/v1/ping",
      }),
    ).toBe("https://remote.example.com/refact");
  });

  test("prefers JetBrains lspUrl over browserUrl for API base", () => {
    expect(
      resolveEngineBaseUrl({
        host: "jetbrains",
        lspUrl: "http://127.0.0.1:8488/p/x",
        browserUrl: "http://myhost.local:8488/p/x",
      }),
    ).toBe("http://127.0.0.1:8488/p/x");
  });
});

describe("hasUsableEngineEndpoint", () => {
  test("allows web relative modes", () => {
    expect(hasUsableEngineEndpoint({ host: "web", dev: true })).toBe(true);
    expect(hasUsableEngineEndpoint({ host: "web", engineServed: true })).toBe(
      true,
    );
  });

  test("does not treat standalone web without a valid lspUrl as same-origin usable", () => {
    expect(hasUsableEngineEndpoint({ host: "web" })).toBe(false);
    expect(
      hasUsableEngineEndpoint({ host: "web", lspUrl: "http2://remote:8001" }),
    ).toBe(false);
  });

  test("allows sanitized remote lspUrl without a usable port", () => {
    expect(
      hasUsableEngineEndpoint({
        host: "web",
        lspUrl: "https://remote.example.com/v1/ping",
        lspPort: 0,
      }),
    ).toBe(true);
    expect(
      hasUsableEngineEndpoint({
        host: "vscode",
        lspUrl: "https://remote.example.com/v1/ping",
      }),
    ).toBe(true);
  });

  test("blocks non-ready IDE plugin endpoints even when URL and port are present", () => {
    expect(
      hasUsableEngineEndpoint({
        host: "vscode",
        lspPort: 8001,
        browserUrl: "http://127.0.0.1:8001",
        lspUrl: "http://127.0.0.1:8001",
        backendReady: false,
        connectionStatus: "starting",
      }),
    ).toBe(false);
    expect(
      hasUsableEngineEndpoint({
        host: "jetbrains",
        lspPort: 8001,
        lspUrl: "http://127.0.0.1:8001",
        backendReady: true,
        connectionStatus: "installing",
      }),
    ).toBe(false);
    expect(
      hasUsableEngineEndpoint({
        host: "vscode",
        lspPort: 8001,
        browserUrl: "http://127.0.0.1:8001",
        lspUrl: "http://127.0.0.1:8001",
        backendReady: false,
        connectionStatus: "failed",
      }),
    ).toBe(false);
  });

  test("keeps ready IDE plugin config usable", () => {
    expect(
      hasUsableEngineEndpoint({
        host: "vscode",
        lspPort: 8001,
        backendReady: true,
        connectionStatus: "ready",
      }),
    ).toBe(true);
  });

  test("requires a positive finite port for local IDE fallback", () => {
    expect(hasUsableEngineEndpoint({ host: "vscode", lspPort: 8001 })).toBe(
      true,
    );
    expect(hasUsableEngineEndpoint({ host: "vscode", lspPort: 0 })).toBe(false);
    expect(hasUsableEngineEndpoint({ host: "jetbrains" })).toBe(false);
    expect(
      hasUsableEngineEndpoint({
        host: "ide",
        lspPort: Number.POSITIVE_INFINITY,
      }),
    ).toBe(false);
  });
});

describe("normalizeEndpointPath", () => {
  test("accepts leading slash and bare v1 endpoint paths", () => {
    expect(normalizeEndpointPath("/v1/ping")).toBe("/v1/ping");
    expect(normalizeEndpointPath("v1/ping")).toBe("/v1/ping");
  });

  test("rejects ambiguous endpoint paths", () => {
    expect(() => normalizeEndpointPath("/ping")).toThrow(
      "Engine API endpoint must start with /v1/",
    );
    expect(() => normalizeEndpointPath("ping")).toThrow(
      "Engine API endpoint must start with /v1/",
    );
  });
});

describe("buildApiUrl", () => {
  test("builds relative Vite dev URLs", () => {
    expect(buildApiUrl({ host: "web", dev: true }, "/v1/ping")).toBe(
      "/v1/ping",
    );
  });

  test("builds engine-served relative URLs", () => {
    expect(
      buildApiUrl(
        { host: "web", engineServed: true, lspUrl: "http://host:8001" },
        "/v1/ping",
      ),
    ).toBe("/v1/ping");
  });

  test("builds URLs with reverse proxy bases", () => {
    expect(
      buildApiUrl(
        { host: "web", lspUrl: "https://example.com/proxy/v1/ping" },
        "v1/caps",
      ),
    ).toBe("https://example.com/proxy/v1/caps");
  });

  test("does not double stale v1 ping paths", () => {
    expect(
      buildApiUrl(
        { host: "web", lspUrl: "http://localhost:5173/v1/ping/Refact" },
        "/v1/ping",
      ),
    ).toBe("http://localhost:5173/v1/ping");
  });

  test("encodes query values and skips nullish values", () => {
    expect(
      buildApiUrl({ host: "vscode", lspPort: 8123 }, "/v1/ping", {
        name: "Refact Agent",
        count: 2,
        enabled: true,
        empty: null,
        missing: undefined,
      }),
    ).toBe(
      "http://127.0.0.1:8123/v1/ping?name=Refact+Agent&count=2&enabled=true",
    );
  });

  test("accepts URLSearchParams query values", () => {
    const params = new URLSearchParams();
    params.append("chat_id", "chat/1");
    params.append("include_content", "false");

    expect(
      buildApiUrl({ host: "web", dev: true }, "/v1/chats/subscribe", params),
    ).toBe("/v1/chats/subscribe?chat_id=chat%2F1&include_content=false");
  });
});

describe("legacy and state adapters", () => {
  test("buildApiUrlFromParts uses local IDE fallback", () => {
    expect(buildApiUrlFromParts(8123, undefined, "/v1/ping")).toBe(
      "http://127.0.0.1:8123/v1/ping",
    );
  });

  test("buildApiUrlFromState reads config", () => {
    const state = {
      config: {
        host: "web",
        dev: true,
        lspPort: 8123,
        themeProps: { appearance: "dark" },
      },
    } as RootState;

    expect(buildApiUrlFromState(state, "/v1/ping", { ok: true })).toBe(
      "/v1/ping?ok=true",
    );
  });

  test("getEngineEndpointIdentity returns stable same-origin identity for relative mode", () => {
    expect(getEngineEndpointIdentity({ host: "web", dev: true })).toBe(
      "same-origin",
    );
  });

  test("getEngineEndpointIdentity returns the sanitized base for remote mode", () => {
    const config: EngineApiConfig = {
      host: "web",
      lspUrl: "https://example.com/base/v1/ping",
    };

    expect(getEngineEndpointIdentity(config)).toBe("https://example.com/base");
  });

  test("getEngineEndpointIdentity reflects IDE fallback port changes", () => {
    expect(getEngineEndpointIdentity({ host: "vscode", lspPort: 8123 })).toBe(
      "http://127.0.0.1:8123",
    );
  });
});
