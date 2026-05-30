import { describe, expect, test } from "vitest";
import { resolveWebLspUrl } from "./useEventBusForWeb";

describe("resolveWebLspUrl", () => {
  test("ignores stale localStorage in engine-served mode", () => {
    expect(
      resolveWebLspUrl(
        {
          engineServed: true,
          lspUrl: "https://configured.example.com/v1/ping",
        },
        "https://stale.example.com/v1/ping",
      ),
    ).toBe("");
  });

  test("ignores stale localStorage in dev mode and uses sanitized config", () => {
    expect(
      resolveWebLspUrl(
        {
          dev: true,
          lspUrl: "https://configured.example.com/proxy/v1/ping?x=1",
        },
        "https://stale.example.com/v1/ping",
      ),
    ).toBe("https://configured.example.com/proxy");
  });

  test("uses relative URLs in dev mode without configured lspUrl", () => {
    expect(
      resolveWebLspUrl({ dev: true, lspUrl: "" }, "https://stale.example.com"),
    ).toBe("");
  });

  test("prefers sanitized localStorage in standalone web mode", () => {
    expect(
      resolveWebLspUrl(
        { lspUrl: "https://configured.example.com" },
        "https://stored.example.com/base/v1/ping#hash",
      ),
    ).toBe("https://stored.example.com/base");
  });

  test("falls back to sanitized config when standalone localStorage is invalid", () => {
    expect(
      resolveWebLspUrl(
        { lspUrl: "https://configured.example.com/refact/v1/ping" },
        "http2://stored.example.com",
      ),
    ).toBe("https://configured.example.com/refact");
  });

  test("falls back to relative URLs when standalone values are invalid", () => {
    expect(
      resolveWebLspUrl(
        { lspUrl: "file:///tmp/refact" },
        "http2://stored.example.com",
      ),
    ).toBe("");
  });
});
