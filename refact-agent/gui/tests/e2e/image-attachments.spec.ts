import { test, expect, type Page } from "@playwright/test";
const check = expect.configure({ timeout: 30000 });
test.setTimeout(60000);
async function ingress(page: Page, method: string, mixed = false) {
  const png = Buffer.from(
    await page.evaluate(() => {
      const c = document.createElement("canvas");
      c.width = 40;
      c.height = 30;
      c.getContext("2d")!.fillRect(0, 0, 40, 30);
      return c.toDataURL("image/png").split(",")[1];
    }),
    "base64",
  );
  if (method === "upload") {
    await page
      .locator('input[type="file"]')
      .first()
      .setInputFiles({ name: "test.png", mimeType: "image/png", buffer: png });
    return;
  }
  await page
    .locator("textarea")
    .first()
    .evaluate(
      (node, args) => {
        const bytes = Uint8Array.from(atob(args.png), (c) => c.charCodeAt(0));
        const data = new DataTransfer();
        data.items.add(new File([bytes], "test.png", { type: "image/png" }));
        if (args.mixed) data.setData("text/plain", "new\ntext");
        if (args.method === "paste")
          node.dispatchEvent(
            new ClipboardEvent("paste", {
              bubbles: true,
              cancelable: true,
              clipboardData: data,
            }),
          );
        else
          node.dispatchEvent(
            new DragEvent("drop", {
              bubbles: true,
              cancelable: true,
              dataTransfer: data,
            }),
          );
      },
      { png: png.toString("base64"), method, mixed },
    );
}
for (const method of ["upload", "drop", "paste"])
  test(`${method}: Cancel discards, Apply attaches without sending`, async ({
    page,
  }) => {
    await page.goto("/tests/e2e/image-attachments.html");
    await check(page.locator("textarea").first()).toBeVisible();
    await check(
      page.getByRole("button", { name: /gpt-5.5/ }).first(),
    ).toBeVisible();
    await ingress(page, method);
    await check(page.getByRole("dialog")).toBeVisible();
    await page
      .getByRole("dialog")
      .getByRole("button", { name: "Cancel / Discard", exact: true })
      .click();
    await check(page.getByRole("dialog")).toHaveCount(0);
    const count = () =>
      page.evaluate(
        () =>
          (
            window as unknown as {
              __toolbarAlignmentStore: {
                getState: () => {
                  chat: {
                    threads: Record<string, { attached_images: unknown[] }>;
                  };
                };
              };
            }
          ).__toolbarAlignmentStore.getState().chat.threads["showcase-chat"]
            .attached_images.length,
      );
    expect(await count()).toBe(0);
    await ingress(page, method);
    await check(page.getByRole("dialog")).toBeVisible();
    await page
      .getByRole("dialog")
      .getByRole("button", { name: "Apply", exact: true })
      .click();
    await expect.poll(count).toBe(1);
    expect(
      await page.evaluate(
        () =>
          (
            window as unknown as { __routeShowcaseCommands: unknown[] }
          ).__routeShowcaseCommands.filter(
            (command) => (command as { type?: string }).type === "user_message",
          ).length,
      ),
    ).toBe(0);
  });
test("Retry mixed clipboard text/caret survives editor Cancel", async ({
  page,
}) => {
  await page.goto("/tests/e2e/image-attachments.html?retry=1");
  const textarea = page.locator("textarea");
  await check(textarea).toHaveValue("before OLD after");
  await check(page.getByRole("button", { name: "Add image" })).toBeVisible();
  await textarea.evaluate((n) => n.setSelectionRange(7, 10));
  await ingress(page, "paste", true);
  await check(page.getByRole("dialog")).toBeVisible();
  await check(textarea).toHaveValue("before new\ntext after");
  expect(
    await textarea.evaluate((n) => [n.selectionStart, n.selectionEnd]),
  ).toEqual([15, 15]);
  await page
    .getByRole("dialog")
    .getByRole("button", { name: "Cancel / Discard", exact: true })
    .click();
  await check(textarea).toHaveValue("before new\ntext after");
  await page.getByRole("button", { name: "Submit", exact: true }).click();
  expect(
    await page.evaluate(
      () =>
        (window as unknown as { __retrySubmitted: unknown }).__retrySubmitted,
    ),
  ).toBe("before new\ntext after");
});

test("model scope change discards a pending edit without cross-thread attachment", async ({
  page,
}) => {
  await page.goto("/tests/e2e/image-attachments.html");
  await check(
    page.getByRole("button", { name: /gpt-5.5/ }).first(),
  ).toBeVisible();
  await ingress(page, "paste");
  await check(page.getByRole("dialog")).toBeVisible();
  await page.evaluate(() => {
    const store = (
      window as unknown as {
        __toolbarAlignmentStore: { dispatch: (action: unknown) => void };
      }
    ).__toolbarAlignmentStore;
    store.dispatch({
      type: "chatThread/setChatModel",
      payload: { chatId: "showcase-chat", model: "unsupported-model" },
    });
  });
  await check(page.getByRole("dialog")).toHaveCount(0);
  const counts = await page.evaluate(() =>
    Object.values(
      (
        window as unknown as {
          __toolbarAlignmentStore: {
            getState: () => {
              chat: { threads: Record<string, { attached_images: unknown[] }> };
            };
          };
        }
      ).__toolbarAlignmentStore.getState().chat.threads,
    ).map((thread) => thread.attached_images.length),
  );
  expect(counts.every((count) => count === 0)).toBe(true);
});
