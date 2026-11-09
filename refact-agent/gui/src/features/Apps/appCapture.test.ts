import { describe, expect, it } from "vitest";
import {
  normalizeCaptureParams,
  parseAppCaptureResult,
  requestAppCapture,
} from "./appCapture";

const PNG =
  "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8DwHwAFBQIAX8jx0gAAAABJRU5ErkJggg==";
const ok = {
  status: "ok",
  method: "dom-raster",
  trust: "app-reported",
  format: "image/png",
  dataUrl: PNG,
  width: 1,
  height: 1,
  viewportWidth: 1,
  viewportHeight: 1,
  scale: 1,
  bytes: 1,
  partial: false,
  unsupported: [],
  masks: 0,
  elements: 3,
};

describe("parseAppCaptureResult", () => {
  it("accepts PNG and recomputes bytes and trust", () => {
    const r = parseAppCaptureResult({ ...ok, trust: "trusted", bytes: 999 });
    expect(r?.trust).toBe("app-reported");
    expect(r?.bytes).toBe(70);
  });
  it("rejects svg, fake png, oversize", () => {
    expect(
      parseAppCaptureResult({
        ...ok,
        format: "image/svg+xml",
        dataUrl: "data:image/svg+xml;base64,PHN2Zz4=",
      }),
    ).toBeNull();
    expect(
      parseAppCaptureResult({ ...ok, dataUrl: "data:image/png;base64,AAAA" }),
    ).toBeNull();
    expect(parseAppCaptureResult({ ...ok, width: 5000 })).toBeNull();
    expect(parseAppCaptureResult(ok, { maxBytes: 1024 })).not.toBeNull();
  });
  it("marks unsupported as partial without image", () => {
    const r = parseAppCaptureResult({
      ...ok,
      status: "unsupported",
      dataUrl: PNG,
      unsupported: ["canvas-webgl"],
    });
    expect(r?.dataUrl).toBeUndefined();
    expect(r?.partial).toBe(true);
  });
  it("clamps params", () => {
    expect(
      normalizeCaptureParams({
        maxWidth: 1e9,
        redactSelectors: [1 as never, "a"],
      }),
    ).toMatchObject({ maxWidth: 4096, redactSelectors: ["a"] });
  });
  it("requestAppCapture throws on invalid result", async () => {
    await expect(
      requestAppCapture({ request: () => Promise.resolve({ status: "ok" }) }),
    ).rejects.toMatchObject({ code: "invalid_capture" });
    await expect(
      requestAppCapture({ request: () => Promise.resolve(ok) }),
    ).resolves.toMatchObject({ format: "image/png" });
  });
});
