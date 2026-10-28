import { describe, expect, it } from "vitest";
import { buildImageDataUrl } from "./imageDataUrl";

describe("buildImageDataUrl", () => {
  it("builds a data url for image payloads", () => {
    expect(buildImageDataUrl("image/png", "abc")).toBe(
      "data:image/png;base64,abc",
    );
  });

  it("rejects the omitted placeholder", () => {
    expect(buildImageDataUrl("image/png", "<omitted>")).toBeNull();
  });

  it("rejects empty or non-string data", () => {
    expect(buildImageDataUrl("image/png", "")).toBeNull();
    expect(buildImageDataUrl("image/png", undefined)).toBeNull();
    expect(buildImageDataUrl("image/png", 42)).toBeNull();
  });

  it("rejects non-image mime types", () => {
    expect(buildImageDataUrl("application/pdf", "abc")).toBeNull();
    expect(buildImageDataUrl("text", "abc")).toBeNull();
    expect(buildImageDataUrl(undefined, "abc")).toBeNull();
  });
});
