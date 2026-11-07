import { describe, expect, it, vi } from "vitest";
import {
  DEFAULT_SETTINGS,
  drawAnnotations,
  exportImage,
  MAX_PIXELS,
  safeDimensions,
  unchanged,
} from "./model";
import { inspectImage } from "./decode";

describe("local image export policy", () => {
  it("requires explicit safe resize while preserving aspect", () => {
    const safe = safeDimensions(16000, 12000);
    expect(safe.width * safe.height).toBeLessThanOrEqual(MAX_PIXELS);
    expect(safe.width / safe.height).toBeCloseTo(4 / 3, 2);
    expect(safeDimensions(600, 400)).toEqual({ width: 600, height: 400 });
  });
  it("only defaults with no annotations preserve original", () => {
    expect(unchanged([], DEFAULT_SETTINGS)).toBe(true);
    expect(unchanged([], { ...DEFAULT_SETTINGS, format: "image/webp" })).toBe(
      false,
    );
    expect(unchanged([], { ...DEFAULT_SETTINGS, target: 1024 })).toBe(false);
    expect(unchanged([], { ...DEFAULT_SETTINGS, quality: 80 })).toBe(false);
  });
  it("bounds lossy target search to eight encodes and never resizes", async () => {
    const toBlob = vi.fn(
      (callback: BlobCallback, format: string, quality: number) =>
        callback(
          new Blob([new Uint8Array(Math.round(10000 * quality))], {
            type: format,
          }),
        ),
    );
    const canvas = {
      toBlob,
      width: 600,
      height: 400,
    } as unknown as HTMLCanvasElement;
    const output = await exportImage(canvas, {
      ...DEFAULT_SETTINGS,
      format: "image/jpeg",
      target: 3000,
    });
    expect(output.blob.size).toBeLessThanOrEqual(3000);
    expect(toBlob.mock.calls.length).toBeLessThanOrEqual(8);
    expect([canvas.width, canvas.height]).toEqual([600, 400]);
  });
  it("PNG target is truthful and encoder failure is visible", async () => {
    const toBlob = vi.fn((callback: BlobCallback) =>
      callback(new Blob([new Uint8Array(3000)], { type: "image/png" })),
    );
    const canvas = { toBlob } as unknown as HTMLCanvasElement;
    expect(
      (await exportImage(canvas, { ...DEFAULT_SETTINGS, target: 1000 }))
        .warning,
    ).toContain("cannot be met");
    expect(toBlob).toHaveBeenCalledTimes(1);
    await expect(
      exportImage(
        { toBlob: (cb: BlobCallback) => cb(null) } as HTMLCanvasElement,
        DEFAULT_SETTINGS,
      ),
    ).rejects.toThrow("encoding failed");
  });
  it("preflights huge PNG dimensions without decoding pixels", async () => {
    const bytes = new Uint8Array(24);
    bytes.set([137, 80, 78, 71]);
    const view = new DataView(bytes.buffer);
    view.setUint32(16, 8000);
    view.setUint32(20, 4000);
    const info = await inspectImage(
      new File([bytes], "huge.png", { type: "image/png" }),
    );
    expect(info.width).toBe(8000);
    expect(info.safe.width).toBeLessThan(info.width);
  });
});

describe("header safety", () => {
  it("uses bounded EXIF orientation for JPEG dimensions including APP1 after SOF", async () => {
    for (const little of [false, true])
      for (const orientation of [1, 5, 6, 7, 8]) {
        const bytes = new Uint8Array(50);
        bytes.set([
          255, 216, 255, 192, 0, 8, 8, 0, 40, 0, 60, 0, 255, 225, 0, 34,
        ]);
        bytes.set([69, 120, 105, 102, 0, 0], 16);
        const v = new DataView(bytes.buffer);
        v.setUint16(22, little ? 0x4949 : 0x4d4d);
        v.setUint16(24, 42, little);
        v.setUint32(26, 8, little);
        v.setUint16(30, 1, little);
        v.setUint16(32, 0x112, little);
        v.setUint16(34, 3, little);
        v.setUint32(36, 1, little);
        v.setUint16(40, orientation, little);
        const file = () =>
          new File([bytes], "rotated.jpg", { type: "image/jpeg" });
        const info = await inspectImage(file());
        expect([info.width, info.height]).toEqual(
          orientation >= 5 ? [40, 60] : [60, 40],
        );
        v.setUint32(26, 0xfffffff0, little);
        expect((await inspectImage(file())).width).toBe(60);
      }
  });
  it("bounds PNG eXIf TIFF parsing and keeps the decode cap before rasterization", async () => {
    const bytes = new Uint8Array(71);
    bytes.set([137, 80, 78, 71]);
    const v = new DataView(bytes.buffer);
    v.setUint32(8, 13);
    v.setUint32(16, 60);
    v.setUint32(20, 40);
    v.setUint32(33, 26);
    bytes.set([101, 88, 73, 102], 37);
    bytes.set(
      [
        73, 73, 42, 0, 8, 0, 0, 0, 1, 0, 18, 1, 3, 0, 1, 0, 0, 0, 6, 0, 0, 0, 0,
        0, 0, 0,
      ],
      41,
    );
    const inspect = () => inspectImage(new File([bytes], "rotated.png"));
    expect(await inspect()).toMatchObject({ width: 40, height: 60 });
    v.setUint32(45, 0xfffffff0, true);
    expect(await inspect()).toMatchObject({ width: 60, height: 40 });
    v.setUint32(45, 8, true);
    v.setUint32(33, 4);
    expect(await inspect()).toMatchObject({ width: 60, height: 40 });
    v.setUint32(33, 26);
    v.setUint32(16, 32769);
    await expect(inspect()).rejects.toThrow("decoding limit");
  });
  it("sniffs animation by magic even with a JPEG MIME label", async () => {
    const gif = new Uint8Array([71, 73, 70, 56, 57, 97, 10, 0, 10, 0]);
    const png = new Uint8Array(45);
    png.set([137, 80, 78, 71]);
    const pv = new DataView(png.buffer);
    pv.setUint32(8, 13);
    pv.setUint32(16, 10);
    pv.setUint32(20, 10);
    png.set([97, 99, 84, 76], 37);
    const webp = new Uint8Array(30);
    webp.set([82, 73, 70, 70], 0);
    webp.set([87, 69, 66, 80, 86, 80, 56, 88], 8);
    webp[20] = 2;
    for (const bytes of [gif, png, webp])
      expect(
        (
          await inspectImage(
            new File([bytes], "renamed.jpg", { type: "image/jpeg" }),
          )
        ).animationWarning,
      ).toBe(true);
    webp[20] = 0;
    expect(
      (await inspectImage(new File([webp], "still.webp"))).animationWarning,
    ).toBe(false);
  });
  it("refuses hard-cap images and arbitrary AVIF ispe without attempting decode", async () => {
    for (const [w, h] of [
      [8001, 8000],
      [32769, 1],
    ]) {
      const bytes = new Uint8Array(24);
      bytes.set([137, 80, 78, 71]);
      const v = new DataView(bytes.buffer);
      v.setUint32(16, w);
      v.setUint32(20, h);
      await expect(inspectImage(new File([bytes], "huge.png"))).rejects.toThrow(
        "decoding limit",
      );
    }
    const bytes = new Uint8Array(32);
    bytes.set([105, 115, 112, 101], 4);
    const v = new DataView(bytes.buffer);
    v.setUint32(12, 100);
    v.setUint32(16, 100);
    await expect(
      inspectImage(new File([bytes], "tile.avif", { type: "image/avif" })),
    ).rejects.toThrow("safely determine");
  });
});

it("renders compound stroke and label through the existing primitive paths", () => {
  const calls: unknown[][] = [];
  const ctx = {
    save: vi.fn(),
    restore: vi.fn(),
    beginPath: vi.fn(),
    moveTo: (...args: number[]) => calls.push(["move", ...args]),
    lineTo: (...args: number[]) => calls.push(["line", ...args]),
    stroke: () => calls.push(["stroke"]),
    fillText: (...args: unknown[]) => calls.push(["text", ...args]),
  } as unknown as CanvasRenderingContext2D;
  drawAnnotations(ctx, [
    {
      tool: "brush-text",
      points: [
        { x: 1, y: 2 },
        { x: 9, y: 10 },
        { x: 12, y: 4 },
      ],
      size: 4,
      color: "red",
      label: {
        point: { x: 20, y: 10 },
        text: "Label",
        size: 24,
        color: "blue",
      },
    },
  ]);
  expect(calls).toEqual([
    ["move", 1, 2],
    ["line", 9, 10],
    ["line", 12, 4],
    ["stroke"],
    ["text", "Label", 20, 10],
  ]);
  expect(ctx.font).toBe("24px sans-serif");
  expect(ctx.fillStyle).toBe("blue");
});
