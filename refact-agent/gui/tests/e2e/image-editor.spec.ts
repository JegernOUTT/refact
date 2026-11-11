import { expect, test, Page } from "@playwright/test";
async function open(page: Page) {
  await page.goto("/tests/e2e/image-editor.html");
  await page.getByRole("button", { name: "Open editor" }).click();
  await expect(page.getByLabel("Image annotation preview")).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Apply", exact: true }),
  ).toBeEnabled();
}
async function exportPanel(page: Page) {
  const panel = page.getByLabel("Export settings panel");
  if (!(await panel.isVisible()))
    await page
      .getByRole("button", { name: "Export settings", exact: true })
      .click();
}
async function draw(page: Page, tool = "Brush") {
  await page.getByRole("button", { name: tool, exact: true }).click();
  const box = await page.getByLabel("Image annotation preview").boundingBox();
  if (!box) throw new Error("Missing canvas");
  await page.mouse.move(box.x + box.width * 0.25, box.y + box.height * 0.5);
  await page.mouse.down();
  await page.mouse.move(
    box.x + box.width * 0.75,
    box.y +
      box.height * (tool === "Ellipse" || tool === "Rectangle" ? 0.75 : 0.5),
    {
      steps: 12,
    },
  );
  await page.mouse.up();
}
async function result(page: Page) {
  await page.getByRole("button", { name: "Apply", exact: true }).click();
  await expect(page.getByTestId("result")).toContainText('"width"');
  return JSON.parse(await page.getByTestId("result").innerText()) as {
    same: boolean;
    type: string;
    width: number;
    height: number;
    pixel: number[];
    redPixels: number;
    size: number;
  };
}
test("default apply preserves original bytes and focus; cancel discards", async ({
  page,
}) => {
  await open(page);
  expect((await result(page)).same).toBe(true);
  await expect(page.getByRole("button", { name: "Open editor" })).toBeFocused();
  await page.getByRole("button", { name: "Open editor" }).click();
  await page.getByRole("button", { name: "Cancel / Discard" }).click();
  await expect(page.getByTestId("result")).toHaveText("cancelled");
});

test("Enter expands keyboard help without attaching the image", async ({
  page,
}) => {
  await open(page);
  const help = page.locator("summary", { hasText: "Keyboard shortcuts" });
  const disclosure = page.locator("details").filter({ has: help });
  await expect(disclosure).not.toHaveAttribute("open");
  await help.focus();
  await page.keyboard.press("Enter");
  await expect(disclosure).toHaveAttribute("open", "");
  await expect(page.getByRole("dialog")).toBeVisible();
  await expect(page.getByTestId("result")).toHaveText("");
});

test("shipping brush pixels survive PNG export with undo redo", async ({
  page,
}) => {
  await open(page);
  await draw(page);
  await expect(
    page.getByRole("button", { name: "Undo", exact: true }),
  ).toBeEnabled();
  await page.getByRole("button", { name: "Undo", exact: true }).click();
  await page.getByRole("button", { name: "Redo", exact: true }).click();
  const output = await result(page);
  expect(output.same).toBe(false);
  expect(output.pixel[0]).toBeGreaterThan(240);
  expect(output.pixel[1]).toBeLessThan(30);
  expect(output.type).toBe("image/png");
});
for (const format of ["image/webp", "image/jpeg"]) {
  test(`exports ${format} at chosen resolution and quality`, async ({
    page,
  }) => {
    await open(page);
    await draw(page, "Line");
    await exportPanel(page);
    await page
      .getByLabel("Export format", { exact: true })
      .selectOption(format);
    await page.getByLabel("Export quality", { exact: true }).fill("35");
    await page.getByLabel("Resolution scale", { exact: true }).fill("50");
    const output = await result(page);
    expect(output.type).toBe(format);
    expect(output.width).toBe(300);
    expect(output.height).toBe(200);
    expect(output.pixel[1]).toBeLessThan(100);
  });
}
test("PNG target warning never silently changes selected resolution", async ({
  page,
}) => {
  await open(page);
  await exportPanel(page);
  await page.getByLabel("Target file size (best effort)").check();
  await page.getByLabel("Target file size", { exact: true }).fill("1");
  await expect(
    page.getByText("Target size cannot be met", { exact: false }),
  ).toBeVisible();
  expect((await result(page)).width).toBe(600);
});
test("all shape and text tools render through same canvas pipeline", async ({
  page,
}) => {
  await open(page);
  const redPixels = () =>
    page
      .getByLabel("Image annotation preview")
      .evaluate((node: HTMLCanvasElement) => {
        const data = node
          .getContext("2d")!
          .getImageData(0, 0, node.width, node.height).data;
        let red = 0;
        for (let i = 0; i < data.length; i += 4)
          if (data[i] > 200 && data[i + 1] < 80) red++;
        return red;
      });
  for (const tool of ["Arrow", "Line", "Ellipse", "Rectangle"]) {
    await draw(page, tool);
    await expect.poll(redPixels).toBeGreaterThan(100);
    await page.getByRole("button", { name: "Undo", exact: true }).click();
    await expect.poll(redPixels).toBe(0);
    await page.getByRole("button", { name: "Redo", exact: true }).click();
    await expect.poll(redPixels).toBeGreaterThan(100);
    await page.getByRole("button", { name: "Reset", exact: true }).click();
    await expect.poll(redPixels).toBe(0);
  }
  await page.getByRole("button", { name: "Text", exact: true }).click();
  await page.getByLabel("Image pan and zoom", { exact: true }).click();
  await page
    .getByLabel("Annotation text", { exact: true })
    .fill("Plain <text>");
  await page.keyboard.press("Control+Enter");
  await expect.poll(redPixels).toBeGreaterThan(100);
  expect((await result(page)).redPixels).toBeGreaterThan(100);
});
test("failed decode retains original or discard choices", async ({ page }) => {
  await page.goto("/tests/e2e/image-editor.html");
  await page.getByLabel("Load fixture file").setInputFiles({
    name: "broken.png",
    mimeType: "image/png",
    buffer: Buffer.from("not an image"),
  });
  await expect(page.getByRole("alert")).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Use original without editing" }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Cancel / Discard" }).click();
});

test("cursor anchored zoom maps drawing to original pixels and prevents page scroll", async ({
  page,
}) => {
  await open(page);
  const canvas = page.getByLabel("Image annotation preview");
  const before = await canvas.boundingBox();
  if (!before) throw new Error("Missing canvas");
  const x = before.x + before.width / 2,
    y = before.y + before.height / 2;
  await page.mouse.move(x, y);
  await page.mouse.wheel(0, -260);
  await expect
    .poll(async () => (await canvas.boundingBox())!.width)
    .toBeGreaterThan(before.width);
  const after = await canvas.boundingBox();
  expect(Math.abs(after!.x + after!.width / 2 - x)).toBeLessThan(2);
  expect(Math.abs(after!.y + after!.height / 2 - y)).toBeLessThan(2);
  await page.getByRole("button", { name: "Brush", exact: true }).click();
  await page.mouse.move(x - 20, y);
  await page.mouse.down();
  await page.mouse.move(x + 20, y, { steps: 8 });
  await page.mouse.up();
  const output = await result(page);
  expect(output.pixel[1]).toBeLessThan(30);
});

test("hard-cap image offers original only without raster decode", async ({
  page,
}) => {
  await page.goto("/tests/e2e/image-editor.html");
  const bytes = Buffer.alloc(24);
  bytes.set([137, 80, 78, 71]);
  bytes.writeUInt32BE(20000, 16);
  bytes.writeUInt32BE(10000, 20);
  await page
    .getByLabel("Load fixture file")
    .setInputFiles({ name: "huge.png", mimeType: "image/png", buffer: bytes });
  await expect(
    page.getByRole("button", { name: "Resize to safe resolution" }),
  ).toHaveCount(0);
  await expect(page.getByRole("alert")).toContainText("decoding limit");
  await expect(
    page.getByRole("button", { name: "Use original without editing" }),
  ).toBeVisible();
  await expect(page.getByLabel("Image annotation preview")).toHaveCount(0);
  await page.getByRole("button", { name: "Cancel / Discard" }).click();
});

test("encoder failure never enables stale Apply", async ({ page }) => {
  await open(page);
  await page.evaluate(() => {
    HTMLCanvasElement.prototype.toBlob = function (callback) {
      callback(null);
    };
  });
  await draw(page);
  await expect(page.getByRole("alert")).toHaveText("Image encoding failed.");
  await expect(
    page.getByRole("button", { name: "Apply", exact: true }),
  ).toBeDisabled();
  await page.getByRole("button", { name: "Cancel / Discard" }).click();
});

test("EXIF orientation is retained in preview and edited JPEG export", async ({
  page,
}) => {
  await page.goto("/tests/e2e/image-editor.html");
  const jpeg = await page.evaluate(async () => {
    const canvas = document.createElement("canvas");
    canvas.width = 60;
    canvas.height = 40;
    const ctx = canvas.getContext("2d")!;
    ctx.fillStyle = "red";
    ctx.fillRect(0, 0, 30, 40);
    ctx.fillStyle = "blue";
    ctx.fillRect(30, 0, 30, 40);
    const blob = await new Promise<Blob>((resolve) =>
      canvas.toBlob((b) => resolve(b!), "image/jpeg"),
    );
    return Array.from(new Uint8Array(await blob.arrayBuffer()));
  });
  const app1 = Buffer.alloc(36);
  app1.set([
    255, 225, 0, 34, 69, 120, 105, 102, 0, 0, 73, 73, 42, 0, 8, 0, 0, 0, 1, 0,
    18, 1, 3, 0, 1, 0, 0, 0, 6, 0,
  ]);
  const buffer = Buffer.concat([
    Buffer.from(jpeg.slice(0, 2)),
    app1,
    Buffer.from(jpeg.slice(2)),
  ]);
  await page
    .getByLabel("Load fixture file")
    .setInputFiles({ name: "rotated.jpg", mimeType: "image/jpeg", buffer });
  const canvas = page.getByLabel("Image annotation preview");
  await expect(canvas).toHaveAttribute("width", "40");
  await expect(canvas).toHaveAttribute("height", "60");
  const corners = await canvas.evaluate((node: HTMLCanvasElement) => {
    const ctx = node.getContext("2d")!;
    return [
      Array.from(ctx.getImageData(20, 10, 1, 1).data),
      Array.from(ctx.getImageData(20, 50, 1, 1).data),
    ];
  });
  expect(corners[0][0]).toBeGreaterThan(220);
  expect(corners[1][2]).toBeGreaterThan(220);
  await exportPanel(page);
  await page
    .getByLabel("Export format", { exact: true })
    .selectOption("image/jpeg");
  const output = await result(page);
  expect([output.width, output.height]).toEqual([40, 60]);
});

for (const format of ["image/png", "image/webp"]) {
  test(`EXIF ${format} native orientation survives edited export`, async ({
    page,
  }) => {
    await page.goto("/tests/e2e/image-editor.html");
    const encoded = await page.evaluate(async (type) => {
      const canvas = document.createElement("canvas");
      canvas.width = 60;
      canvas.height = 40;
      const ctx = canvas.getContext("2d")!;
      ctx.fillStyle = "red";
      ctx.fillRect(0, 0, 30, 40);
      ctx.fillStyle = "blue";
      ctx.fillRect(30, 0, 30, 40);
      const blob = await new Promise<Blob>((resolve) =>
        canvas.toBlob((b) => resolve(b!), type),
      );
      return Array.from(new Uint8Array(await blob.arrayBuffer()));
    }, format);
    const tiff = Buffer.from([
      73, 73, 42, 0, 8, 0, 0, 0, 1, 0, 18, 1, 3, 0, 1, 0, 0, 0, 6, 0, 0, 0, 0,
      0, 0, 0,
    ]);
    const source = Buffer.from(encoded);
    let buffer: Buffer;
    if (format === "image/png") {
      const chunk = Buffer.alloc(tiff.length + 12);
      chunk.writeUInt32BE(tiff.length);
      chunk.write("eXIf", 4);
      tiff.copy(chunk, 8);
      let crc = 0xffffffff;
      for (const byte of chunk.subarray(4, -4)) {
        crc ^= byte;
        for (let bit = 0; bit < 8; bit++)
          crc = (crc >>> 1) ^ (crc & 1 ? 0xedb88320 : 0);
      }
      chunk.writeUInt32BE((crc ^ 0xffffffff) >>> 0, chunk.length - 4);
      buffer = Buffer.concat([
        source.subarray(0, 33),
        chunk,
        source.subarray(33),
      ]);
    } else {
      const extended = Buffer.alloc(18);
      extended.write("VP8X");
      extended.writeUInt32LE(10, 4);
      extended[8] = 8;
      extended.writeUIntLE(59, 12, 3);
      extended.writeUIntLE(39, 15, 3);
      const payload = Buffer.concat([Buffer.from("Exif\0\0"), tiff]);
      const exif = Buffer.alloc(payload.length + 8);
      exif.write("EXIF");
      exif.writeUInt32LE(payload.length, 4);
      payload.copy(exif, 8);
      buffer = Buffer.concat([
        source.subarray(0, 12),
        extended,
        source.subarray(source.toString("ascii", 12, 16) === "VP8X" ? 30 : 12),
        exif,
      ]);
      buffer.writeUInt32LE(buffer.length - 8, 4);
    }
    const native = await page.evaluate(
      async ({ bytes, type }) => {
        const bitmap = await createImageBitmap(
          new Blob([new Uint8Array(bytes)], { type }),
          { imageOrientation: "from-image" },
        );
        const size = [bitmap.width, bitmap.height];
        bitmap.close();
        return size;
      },
      { bytes: Array.from(buffer), type: format },
    );
    const expected = format === "image/png" ? [40, 60] : [60, 40];
    expect(native).toEqual(expected);
    await page.getByLabel("Load fixture file").setInputFiles({
      name: `rotated.${format.split("/")[1]}`,
      mimeType: format,
      buffer,
    });
    const canvas = page.getByLabel("Image annotation preview");
    await expect(canvas).toHaveAttribute("width", String(expected[0]));
    await expect(canvas).toHaveAttribute("height", String(expected[1]));
    const colors = await canvas.evaluate((node: HTMLCanvasElement) =>
      (node.width === 40
        ? [
            [20, 10],
            [20, 50],
          ]
        : [
            [10, 20],
            [50, 20],
          ]
      ).map(([x, y]) =>
        Array.from(node.getContext("2d")!.getImageData(x, y, 1, 1).data),
      ),
    );
    expect(colors[0][0]).toBeGreaterThan(220);
    expect(colors[1][2]).toBeGreaterThan(220);
    await exportPanel(page);
    await page
      .getByLabel("Export format", { exact: true })
      .selectOption(format);
    await page.getByLabel("Resolution scale", { exact: true }).fill("50");
    const output = await result(page);
    expect(output.same).toBe(false);
    expect([output.width, output.height]).toEqual(
      expected.map((size) => size / 2),
    );
  });
}

test("actual lossy target encoder honors target and stale settings cannot Apply", async ({
  page,
}) => {
  await open(page);
  await page.evaluate(() => {
    const encode = HTMLCanvasElement.prototype.toBlob;
    HTMLCanvasElement.prototype.toBlob = function (callback, ...args) {
      encode.call(
        this,
        (blob) => window.setTimeout(() => callback(blob), 150),
        ...args,
      );
    };
  });
  await exportPanel(page);
  await page
    .getByLabel("Export format", { exact: true })
    .selectOption("image/jpeg");
  await exportPanel(page);
  await page.getByLabel("Target file size (best effort)").check();
  await page.getByLabel("Target file size", { exact: true }).fill("2");
  await page.getByLabel("Resolution scale", { exact: true }).fill("60");
  await expect(
    page.getByRole("button", { name: "Apply", exact: true }),
  ).toBeDisabled();
  await page.getByLabel("Resolution scale", { exact: true }).fill("50");
  const output = await result(page);
  expect([output.width, output.height]).toEqual([300, 200]);
  expect(output.size).toBeLessThanOrEqual(2048);
  expect(output.type).toBe("image/jpeg");
});

test("compound brush and beside label are one undoable exported edit", async ({
  page,
}) => {
  await open(page);
  const tool = page.getByRole("button", { name: "Brush + Text", exact: true });
  await expect(tool).not.toHaveAttribute("title");
  await expect(tool.locator("svg")).toHaveCount(2);
  await draw(page, "Brush + Text");
  await expect(tool).toHaveAttribute("aria-pressed", "true");
  await expect(
    page.getByText("Click beside the stroke to place", { exact: false }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Apply", exact: true }),
  ).toBeDisabled();
  const canvas = page.getByLabel("Image annotation preview");
  const regions = () =>
    canvas.evaluate((node: HTMLCanvasElement) => {
      const ctx = node.getContext("2d")!;
      const count = (y: number, height: number) => {
        const pixels = ctx.getImageData(0, y, node.width, height).data;
        let red = 0;
        for (let i = 0; i < pixels.length; i += 4)
          if (pixels[i] > 200 && pixels[i + 1] < 80) red++;
        return red;
      };
      return [count(190, 20), count(240, 100)];
    });
  await expect.poll(async () => (await regions())[0]).toBeGreaterThan(500);
  const box = await canvas.boundingBox();
  if (!box) throw new Error("Missing canvas");
  await page.mouse.move(box.x + box.width * 0.35, box.y + box.height * 0.65);
  await page.mouse.down();
  await page.mouse.move(box.x + box.width * 0.6, box.y + box.height * 0.65, {
    steps: 5,
  });
  await page.mouse.up();
  await page
    .getByLabel("Annotation text", { exact: true })
    .fill("Beside stroke");
  await page.keyboard.press("Control+Enter");
  await expect.poll(async () => (await regions())[1]).toBeGreaterThan(100);
  const completed = await regions();
  await page.getByRole("button", { name: "Undo", exact: true }).click();
  await expect.poll(regions).toEqual([0, 0]);
  await draw(page, "Brush + Text");
  await page.getByRole("button", { name: "Redo", exact: true }).click();
  await expect.poll(regions).toEqual([0, 0]);
  await expect(
    page.getByRole("button", { name: "Redo", exact: true }),
  ).toBeEnabled();
  await expect(
    page.getByRole("button", { name: "Undo", exact: true }),
  ).toBeDisabled();
  await page.getByRole("button", { name: "Redo", exact: true }).click();
  await expect.poll(regions).toEqual(completed);
  const output = await result(page);
  expect(output.redPixels).toBe(completed[0] + completed[1]);
  expect(output.pixel[1]).toBeLessThan(30);
});

test("pending compound edit cancels without export or duplicate stroke", async ({
  page,
}) => {
  await open(page);
  const apply = page.getByRole("button", { name: "Apply", exact: true });
  for (const action of ["Escape", "Undo", "Redo", "Reset", "Pan"]) {
    await draw(page, "Brush + Text");
    await expect(apply).toBeDisabled();
    if (action === "Escape") await page.keyboard.press("Escape");
    else await page.getByRole("button", { name: action, exact: true }).click();
    await expect(page.getByRole("dialog")).toBeVisible();
    await expect(apply).toBeEnabled();
    await expect(
      page.getByText("Click beside the stroke to place", { exact: false }),
    ).toHaveCount(0);
  }
  expect((await result(page)).same).toBe(true);
});

async function redPixels(page: Page) {
  return page
    .getByLabel("Image annotation preview")
    .evaluate((node: HTMLCanvasElement) => {
      const data = node
        .getContext("2d")!
        .getImageData(0, 0, node.width, node.height).data;
      let red = 0;
      for (let i = 0; i < data.length; i += 4)
        if (data[i] > 200 && data[i + 1] < 80) red++;
      return red;
    });
}

test("editor keyboard undo redo is local, pixel-correct and cancels pending before history", async ({
  page,
}) => {
  await open(page);
  await page.evaluate(() => {
    document.body.dataset.bubbled = "";
    window.addEventListener("keydown", (event) => {
      document.body.dataset.bubbled += event.key;
    });
  });
  await draw(page);
  await expect(
    page.getByRole("button", { name: "Apply", exact: true }),
  ).toBeEnabled();
  const committed = await redPixels(page);
  expect(committed).toBeGreaterThan(500);
  await page.keyboard.press("Control+z");
  await expect.poll(() => redPixels(page)).toBe(0);
  await page.keyboard.press("Control+Shift+z");
  await expect.poll(() => redPixels(page)).toBe(committed);
  await page.keyboard.press("Meta+z");
  await expect.poll(() => redPixels(page)).toBe(0);
  await page.keyboard.press("Meta+Shift+z");
  await expect.poll(() => redPixels(page)).toBe(committed);
  await page.keyboard.press("Control+z");
  await page.keyboard.press("Control+y");
  await expect.poll(() => redPixels(page)).toBe(committed);
  await page.keyboard.press("Control+z");
  await draw(page, "Brush + Text");
  await page.keyboard.press("Control+z");
  await expect.poll(() => redPixels(page)).toBe(0);
  await expect(
    page.getByRole("button", { name: "Redo", exact: true }),
  ).toBeEnabled();
  await expect(
    page.getByRole("button", { name: "Undo", exact: true }),
  ).toBeDisabled();
  await page.keyboard.press("Control+y");
  await expect.poll(() => redPixels(page)).toBe(committed);
  const bubbled = await page.locator("body").getAttribute("data-bubbled");
  expect(bubbled).not.toMatch(/[zy]/i);
  expect((await result(page)).redPixels).toBe(committed);
});

test("typing keeps native undo and ignores tool shortcuts, composition and repeats", async ({
  page,
}) => {
  await open(page);
  await draw(page);
  await expect(
    page.getByRole("button", { name: "Apply", exact: true }),
  ).toBeEnabled();
  const committed = await redPixels(page);
  await page.keyboard.press("t");
  await page.getByLabel("Image pan and zoom", { exact: true }).click();
  const input = page.getByLabel("Annotation text", { exact: true });
  await input.click();
  await page.keyboard.type("brush + 01");
  await page.keyboard.press("Control+z");
  await expect(input).not.toHaveValue("brush + 01");
  await page.keyboard.press("Control+Shift+z");
  await expect(input).toHaveValue("brush + 01");
  await expect.poll(() => redPixels(page)).toBe(committed);
  await expect(
    page.getByRole("button", { name: "Text", exact: true }),
  ).toHaveAttribute("aria-pressed", "true");
  await page.getByRole("button", { name: "Pan", exact: true }).click();
  for (const modifiers of [
    { repeat: true },
    { isComposing: true },
    { altKey: true },
    { ctrlKey: true },
    { metaKey: true },
  ]) {
    await page
      .getByRole("button", { name: "Pan", exact: true })
      .dispatchEvent("keydown", { key: "b", bubbles: true, ...modifiers });
    await expect(
      page.getByRole("button", { name: "Pan", exact: true }),
    ).toHaveAttribute("aria-pressed", "true");
  }
  await page.keyboard.press("Control+Enter");
  await expect(page.getByRole("dialog")).toBeVisible();
});

test("tool shortcuts, shared zoom actions, accessible buttons and two-stage Escape", async ({
  page,
}) => {
  await open(page);
  for (const [key, name] of [
    ["b", "Brush"],
    ["t", "Text"],
    ["Shift+b", "Brush + Text"],
    ["l", "Line"],
    ["a", "Arrow"],
    ["e", "Ellipse"],
    ["r", "Rectangle"],
    ["v", "Pan"],
  ]) {
    await page.keyboard.press(key);
    await expect(
      page.getByRole("button", { name, exact: true }),
    ).toHaveAttribute("aria-pressed", "true");
  }
  const zoom = page.getByLabel("Image zoom", { exact: true });
  const fit = await zoom.innerText();
  await page.keyboard.press("+");
  expect(await zoom.innerText()).not.toBe(fit);
  await page.keyboard.press("-");
  await expect(zoom).toHaveText(fit);
  await page.keyboard.press("1");
  await expect(zoom).toHaveText("100%");
  await page.keyboard.press("+");
  await page.keyboard.press("0");
  await expect(zoom).toHaveText(fit);
  const brush = page.getByRole("button", { name: "Brush", exact: true });
  await brush.focus();
  await page.keyboard.press("Enter");
  await expect(brush).toHaveAttribute("aria-pressed", "true");
  await expect(brush).toBeFocused();
  await page.getByText("Keyboard shortcuts", { exact: true }).click();
  await expect(
    page.getByText("V Pan · B Brush", { exact: false }),
  ).toBeVisible();
  await draw(page, "Brush + Text");
  await page.keyboard.press("Escape");
  await expect(page.getByRole("dialog")).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Apply", exact: true }),
  ).toBeEnabled();
  await page.keyboard.press("Escape");
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Open editor" })).toBeFocused();
});

for (const width of [360, 768, 1280]) {
  test(`compact workspace geometry and collapsible export at ${width}px`, async ({
    page,
  }) => {
    await page.setViewportSize({ width, height: 900 });
    await open(page);
    const dialog = page.getByRole("dialog");
    const rail = page.getByRole("group", { name: "Annotation tools" });
    const options = page.getByLabel("Tool options", { exact: true });
    const canvas = page.getByLabel("Image pan and zoom", { exact: true });
    const [d, r, o, c] = await Promise.all([
      dialog.boundingBox(),
      rail.boundingBox(),
      options.boundingBox(),
      canvas.boundingBox(),
    ]);
    const title = await page
      .getByRole("heading", { name: "Edit image" })
      .boundingBox();
    const zoom = await page
      .getByRole("group", { name: "Image view controls", exact: true })
      .boundingBox();
    expect(c!.y - title!.y).toBeLessThanOrEqual(100);
    expect(zoom!.y).toBeGreaterThanOrEqual(c!.y + c!.height);
    const zoomGroup = page.getByRole("group", {
      name: "Image view controls",
      exact: true,
    });
    await expect(zoomGroup).toHaveCount(1);
    await expect(dialog).toContainText("Original bytes unchanged");
    for (const height of await zoomGroup
      .locator("button")
      .evaluateAll((nodes) =>
        nodes.map((node) => node.getBoundingClientRect().height),
      ))
      expect(height).toBe(28);
    await zoomGroup.getByRole("button", { name: "Zoom in" }).focus();
    const initialZoom = await zoomGroup.getByLabel("Image zoom").textContent();
    await page.keyboard.press("+");
    await expect(zoomGroup.getByLabel("Image zoom")).not.toHaveText(
      initialZoom!,
    );
    await zoomGroup.getByRole("button", { name: "Fit", exact: true }).click();
    expect(r!.width).toBeLessThanOrEqual(42);
    expect(r!.height).toBeGreaterThan(r!.width * 6);
    expect(o!.height).toBeLessThanOrEqual(40);
    expect(c!.height).toBeGreaterThan(d!.height * 0.5);
    expect(c!.width).toBeGreaterThan(d!.width * 0.7);
    expect(c!.x).toBeGreaterThan(r!.x + r!.width - 1);
    expect(
      await dialog.evaluate((node) => node.scrollWidth <= node.clientWidth),
    ).toBe(true);
    await expect(page.getByLabel("Export settings panel")).toHaveCount(0);
    await exportPanel(page);
    await expect(
      page.getByLabel("Export format", { exact: true }),
    ).toBeVisible();
    await page
      .getByRole("button", { name: "Export settings", exact: true })
      .click();
    await page
      .getByRole("button", { name: "Brush + Text", exact: true })
      .click();
    expect((await options.boundingBox())!.height).toBeLessThanOrEqual(40);
    const controls = options.locator("input, button");
    const boxes = await controls.evaluateAll((nodes) =>
      nodes.map((node) => {
        const { x, y, width, height } = node.getBoundingClientRect();
        return { x, y, width, height };
      }),
    );
    expect(new Set(boxes.map((box) => box.height)).size).toBe(1);
    expect(boxes[0].height).toBe(28);
    for (let i = 1; i < boxes.length; i++)
      expect(boxes[i].x).toBeGreaterThanOrEqual(
        boxes[i - 1].x + boxes[i - 1].width,
      );
    const optionBox = (await options.boundingBox())!;
    expect(boxes.at(-1)!.x + boxes.at(-1)!.width).toBeLessThanOrEqual(
      optionBox.x + optionBox.width,
    );
    await expect(
      page.getByRole("button", { name: "Apply", exact: true }),
    ).toBeInViewport();
    await expect(
      page.getByRole("button", { name: "Apply", exact: true }),
    ).toBeEnabled();

    expect(
      await dialog.evaluate((node) => node.scrollWidth <= node.clientWidth),
    ).toBe(true);
  });
}

test("inline text tracks transformed image coordinates, survives blur, multiline commits and cancels", async ({
  page,
}) => {
  await open(page);
  const canvas = page.getByLabel("Image annotation preview");
  await page.getByRole("button", { name: "Zoom in", exact: true }).click();
  const view = page.getByLabel("Image pan and zoom", { exact: true });
  const box = (await view.boundingBox())!;
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await page.mouse.down();
  await page.mouse.move(
    box.x + box.width / 2 + 30,
    box.y + box.height / 2 + 20,
  );
  await page.mouse.up();
  await page.getByRole("button", { name: "Text", exact: true }).click();
  const before = (await canvas.boundingBox())!;
  const clicked = {
    x: before.x + before.width * 0.3,
    y: before.y + before.height * 0.4,
  };
  await page.mouse.click(clicked.x, clicked.y);
  const input = page.getByLabel("Annotation text", { exact: true });
  await expect(input).toBeFocused();
  const inline = (await input.boundingBox())!;
  expect(Math.abs(inline.x - clicked.x)).toBeLessThan(2);
  expect(Math.abs(inline.y - clicked.y)).toBeLessThan(2);
  await page.keyboard.type("First");
  await page.keyboard.press("Enter");
  await page.keyboard.type("Second");
  await expect(input).toHaveValue("First\nSecond");
  await page.getByRole("button", { name: "Zoom in", exact: true }).click();
  const after = (await canvas.boundingBox())!;
  const moved = (await input.boundingBox())!;
  expect(Math.abs((moved.x - after.x) / after.width - 0.3)).toBeLessThan(0.005);
  expect(Math.abs((moved.y - after.y) / after.height - 0.4)).toBeLessThan(
    0.005,
  );
  await exportPanel(page);
  await page
    .getByLabel("Export format", { exact: true })
    .selectOption("image/png");
  await expect(input).toHaveValue("First\nSecond");
  await expect(
    page.getByRole("button", { name: "Apply", exact: true }),
  ).toBeDisabled();
  await input.focus();
  await page.keyboard.press("Meta+Enter");
  await expect(input).toHaveCount(0);
  await expect.poll(() => redPixels(page)).toBeGreaterThan(100);
  const committed = await redPixels(page);
  await page.getByRole("button", { name: "Undo", exact: true }).click();
  await expect.poll(() => redPixels(page)).toBe(0);
  await page.getByRole("button", { name: "Redo", exact: true }).click();
  await expect.poll(() => redPixels(page)).toBe(committed);
  await page
    .getByRole("button", { name: "Export settings", exact: true })
    .click();
  await page.getByRole("button", { name: "Fit", exact: true }).click();
  await view.click();
  await input.fill("discarded");
  await page.keyboard.press("Escape");
  await expect(input).toHaveCount(0);
  await expect(page.getByRole("dialog")).toBeVisible();
  expect((await result(page)).redPixels).toBe(committed);
});

test("inline wide glyphs do not soft wrap and near-edge text remains scrollable", async ({
  page,
}) => {
  await open(page);
  await page.getByRole("button", { name: "Text", exact: true }).click();
  const canvas = page.getByLabel("Image annotation preview");
  const box = (await canvas.boundingBox())!;
  await page.mouse.click(box.x + box.width * 0.92, box.y + box.height * 0.3);
  const input = page.getByLabel("Annotation text", { exact: true });
  await input.fill("WWWWWWWWWWWW\nWWWW");
  expect(await input.getAttribute("wrap")).toBe("off");
  expect(
    await input.evaluate(
      (node: HTMLTextAreaElement) => node.scrollWidth > node.clientWidth,
    ),
  ).toBe(true);
  await input.dispatchEvent("compositionstart");
  await page.keyboard.press("Escape");
  await expect(input).toHaveValue("WWWWWWWWWWWW\nWWWW");
  await input.dispatchEvent("compositionend");
  await page.keyboard.press("Control+Enter");
  await expect(canvas).toBeFocused();
  await expect.poll(() => redPixels(page)).toBeGreaterThan(100);
  const pixels = await redPixels(page);
  expect(pixels).toBeGreaterThan(100);
  expect((await result(page)).redPixels).toBe(pixels);
});

test("decode failure occupies workspace width instead of the tool rail", async ({
  page,
}) => {
  await page.setViewportSize({ width: 360, height: 600 });
  await page.goto("/tests/e2e/image-editor.html");
  await page.getByLabel("Load fixture file").setInputFiles({
    name: "bad.png",
    mimeType: "image/png",
    buffer: Buffer.from("broken"),
  });
  const alert = page.getByRole("alert");
  await expect(alert).toBeVisible();
  expect((await alert.boundingBox())!.width).toBeGreaterThan(200);
  await expect(
    page.getByRole("button", { name: "Use original without editing" }),
  ).toBeVisible();
  expect(
    await page
      .getByRole("dialog")
      .evaluate((n) => n.scrollWidth <= n.clientWidth),
  ).toBe(true);
});
