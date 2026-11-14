import { expect, test } from "@playwright/test";
import { openReviewFixture } from "./reviewFixtureReady";

for (const { width, height } of [
  { width: 1600, height: 900 },
  { width: 1440, height: 900 },
  { width: 1280, height: 900 },
  { width: 820, height: 900 },
  { width: 360, height: 900 },
  { width: 1024, height: 900 },
  { width: 768, height: 900 },
  { width: 390, height: 900 },
  { width: 390, height: 420 },
  { width: 820, height: 420 },
]) {
  test(`review identity and controls remain usable at ${width}x${height}`, async ({
    page,
  }, testInfo) => {
    await page.setViewportSize({ width, height });
    await openReviewFixture(page, "/tests/e2e/review-canvas.html?tour=rich");
    const draft = page.getByRole("textbox", { name: "Chat composer" });
    await draft.fill("Keep this unsent draft");
    if (width <= 680) {
      await page
        .getByRole("radio", { name: "Canvas", exact: true })
        .locator("..")
        .click();
    }
    const canvas = page.getByRole("region", { name: "Review canvas" });
    const header = canvas.locator(":scope > header");
    const identity = header.locator('[title="feature/parser"]').first();
    await expect(identity).toHaveText(
      "Review: parser trimming and total() bounds",
    );
    const geometry = await header.evaluate((element) => {
      const identity = element.querySelector<HTMLElement>(
        '[title="feature/parser"]',
      )!.parentElement!;
      const box = element.getBoundingClientRect();
      return {
        height: box.height,
        rowHeight: Math.max(
          ...Array.from(
            element.querySelectorAll<HTMLElement>("button, select"),
          ).map((node) => node.getBoundingClientRect().height),
        ),
        rowGap: parseFloat(getComputedStyle(element).rowGap),
        verticalChrome:
          parseFloat(getComputedStyle(element).paddingTop) +
          parseFloat(getComputedStyle(element).paddingBottom) +
          parseFloat(getComputedStyle(element).borderBottomWidth),
        scrollWidth: element.scrollWidth,
        clientWidth: element.clientWidth,
        controls: Array.from(
          element.querySelectorAll<HTMLElement>(
            "button, input, select, a[href], [role=combobox]",
          ),
        ).map((node) => {
          const control = (
            node.closest("label") ?? node
          ).getBoundingClientRect();
          return {
            name: node.getAttribute("aria-label") ?? node.textContent,
            inside:
              control.left >= box.left &&
              control.right <= box.right &&
              control.top >= box.top &&
              control.bottom <= box.bottom,
          };
        }),
        canvasBottom: element.parentElement!.getBoundingClientRect().bottom,
        bodyBottom: element
          .parentElement!.querySelector<HTMLElement>(
            ':scope > [class*="_body_"]',
          )!
          .getBoundingClientRect().bottom,
        canvasScrollHeight: element.parentElement!.scrollHeight,
        canvasClientHeight: element.parentElement!.clientHeight,
        identityWidth: identity.getBoundingClientRect().width,
        identityHeight: identity.getBoundingClientRect().height,
        whiteSpace: getComputedStyle(identity).whiteSpace,
        documentWidth: document.documentElement.scrollWidth,
        viewportWidth: document.documentElement.clientWidth,
        overflowing: Array.from(
          document.querySelectorAll<HTMLElement>("body *"),
        )
          .filter(
            (node) =>
              !element.contains(node) &&
              node.getBoundingClientRect().right > innerWidth + 1,
          )
          .slice(0, 20)
          .map((node) => ({
            tag: node.tagName,
            className: String(node.className),
            right: node.getBoundingClientRect().right,
            inHeader: element.contains(node),
          })),
      };
    });
    console.log(JSON.stringify(geometry));
    expect(geometry.controls.length).toBeGreaterThanOrEqual(9);
    const receipt = await header.evaluate((element) =>
      Array.from(
        element.querySelectorAll<HTMLElement>(
          'a[href], [class*="_reference_"]',
        ),
      ).map((node) => ({
        width: Math.ceil(node.getBoundingClientRect().width),
        content: node.scrollWidth,
      })),
    );
    expect(receipt.length).toBeGreaterThan(0);
    for (const item of receipt) {
      expect(item.width).toBeGreaterThanOrEqual(item.content);
    }
    const candidateLabel = await header.evaluate((element) => {
      const trigger = element.querySelector<HTMLElement>(
        '[aria-label="Candidate revision"]',
      )!;
      const value = trigger.querySelector("span")!.getBoundingClientRect();
      const box = trigger.getBoundingClientRect();
      return {
        left: Math.round(value.left - box.left),
        right: Math.round(box.right - value.right),
      };
    });
    expect(candidateLabel.left).toBeGreaterThanOrEqual(0);
    expect(candidateLabel.right).toBeGreaterThanOrEqual(0);
    if (height > 420) {
      expect(geometry.scrollWidth).toBeLessThanOrEqual(geometry.clientWidth);
      expect(geometry.controls.filter((control) => !control.inside)).toEqual(
        [],
      );
    } else {
      expect(geometry.scrollWidth).toBeGreaterThan(geometry.clientWidth);
    }
    expect(
      Math.abs(geometry.bodyBottom - geometry.canvasBottom),
    ).toBeLessThanOrEqual(1);
    expect(geometry.canvasScrollHeight).toBeLessThanOrEqual(
      geometry.canvasClientHeight,
    );
    const maximumRows =
      height === 420
        ? 1
        : width >= 1600
          ? 1
          : width >= 1024
            ? 2
            : width === 820
              ? 3
              : 4;
    expect(geometry.height).toBeLessThanOrEqual(
      geometry.verticalChrome +
        maximumRows * geometry.rowHeight +
        (maximumRows - 1) * geometry.rowGap,
    );
    expect(geometry.identityWidth).toBeGreaterThan(0);
    expect(geometry.identityHeight).toBeLessThan(40);
    expect(geometry.whiteSpace).toBe("nowrap");
    await testInfo.attach("header-layout", {
      body: JSON.stringify(geometry, null, 2),
      contentType: "application/json",
    });
    await testInfo.attach("review-layout", {
      body: await page.screenshot(),
      contentType: "image/png",
    });

    const controls = header.locator(
      "button:not([disabled]), select:not([disabled]), input:not([disabled])",
    );
    const last = controls.last();
    await expect(last).toBeVisible();
    const outerScroll = () =>
      canvas.evaluate((element) => ({
        canvasTop: element.scrollTop,
        canvasLeft: element.scrollLeft,
        pageTop: window.scrollY,
        pageLeft: window.scrollX,
      }));
    const initialOuterScroll = await outerScroll();
    await page.evaluate(
      () => (document.activeElement as HTMLElement | null)?.blur(),
    );
    let reached = false;
    for (let i = 0; i < 80; i++) {
      await page.keyboard.press("Tab");
      if (await last.evaluate((node) => node === document.activeElement)) {
        reached = true;
        break;
      }
    }
    expect(reached, "rightmost header control is reachable by Tab").toBe(true);
    await expect(last).toBeInViewport();
    const focusedGeometry = await last.evaluate((node) => {
      const header = node.closest("header")!;
      return {
        control: node.getBoundingClientRect().toJSON(),
        header: header.getBoundingClientRect().toJSON(),
        scrollLeft: header.scrollLeft,
        scrollWidth: header.scrollWidth,
        clientWidth: header.clientWidth,
        paddingLeft: getComputedStyle(header).paddingLeft,
        paddingRight: getComputedStyle(header).paddingRight,
      };
    });
    console.log("focused-header", JSON.stringify(focusedGeometry));
    await testInfo.attach("focused-header-layout", {
      body: JSON.stringify(focusedGeometry, null, 2),
      contentType: "application/json",
    });
    const visibility = await last.evaluate((node) => {
      const control = node.getBoundingClientRect();
      const strip = node.closest("header")!.getBoundingClientRect();
      return control.left >= strip.left - 1 && control.right <= strip.right + 1;
    });
    expect(visibility).toBe(true);
    if (height > 420) expect(focusedGeometry.scrollLeft).toBe(0);
    else {
      expect(focusedGeometry.scrollLeft).toBeGreaterThan(0);
      for (const control of await controls.all()) {
        await control.focus();
        expect(
          await control.evaluate((node) => {
            const box = (node.closest("label") ?? node).getBoundingClientRect();
            const header = node.closest("header")!.getBoundingClientRect();
            return (
              box.left >= header.left - 1 &&
              box.right <= header.right + 1 &&
              box.top >= header.top &&
              box.bottom <= header.bottom
            );
          }),
        ).toBe(true);
      }
    }
    expect(await outerScroll()).toEqual(initialOuterScroll);
    if (width <= 680) {
      await page
        .getByRole("radio", { name: "Chat", exact: true })
        .locator("..")
        .click();
    }
    await expect(draft).toHaveValue("Keep this unsent draft");
    expect(geometry.documentWidth).toBeLessThanOrEqual(geometry.viewportWidth);
  });
}

for (const canvasWidth of [780, 660, 480, 360]) {
  test(`header contains controls in a ${canvasWidth}px canvas`, async ({
    page,
  }, testInfo) => {
    await page.setViewportSize({ width: 1440, height: 900 });
    await openReviewFixture(page, "/tests/e2e/review-canvas.html?tour=rich");
    const canvas = page.getByRole("region", { name: "Review canvas" });
    await expect(canvas).toBeVisible();
    await canvas.evaluate((element, width) => {
      element.style.width = `${width}px`;
    }, canvasWidth);
    const geometry = await canvas.evaluate((element) => {
      const header = element.querySelector("header")!;
      const box = header.getBoundingClientRect();
      const identity = header.querySelector<HTMLElement>(
        '[title="feature/parser"]',
      )!.parentElement!;
      const candidate = header.querySelector<HTMLElement>(
        '[aria-label="Candidate revision"]',
      )!;
      const style = getComputedStyle(header);
      return {
        identityWidth: identity.clientWidth,
        identityContentWidth: identity.scrollWidth,
        candidateHeight: candidate.getBoundingClientRect().height,
        candidateValueHeight: candidate
          .querySelector("span")!
          .getBoundingClientRect().height,
        rowHeight: Math.max(
          ...Array.from(
            header.querySelectorAll<HTMLElement>("button, select"),
          ).map((node) => node.getBoundingClientRect().height),
        ),
        rowGap: parseFloat(style.rowGap),
        verticalChrome:
          parseFloat(style.paddingTop) +
          parseFloat(style.paddingBottom) +
          parseFloat(style.borderBottomWidth),
        canvasWidth: element.clientWidth,
        clientWidth: header.clientWidth,
        scrollWidth: header.scrollWidth,
        height: box.height,
        outside: Array.from(
          header.querySelectorAll<HTMLElement>(
            "button, input, select, a[href], [role=combobox]",
          ),
        )
          .filter((node) => {
            const control = (
              node.closest("label") ?? node
            ).getBoundingClientRect();
            return (
              control.left < box.left ||
              control.right > box.right ||
              control.top < box.top ||
              control.bottom > box.bottom
            );
          })
          .map((node) => node.getAttribute("aria-label") ?? node.textContent),
        canvasBottom: element.getBoundingClientRect().bottom,
        bodyBottom: element
          .querySelector(':scope > [class*="_body_"]')!
          .getBoundingClientRect().bottom,
      };
    });
    console.log("constrained-header", JSON.stringify(geometry));
    await testInfo.attach("constrained-header", {
      body: JSON.stringify(geometry, null, 2),
      contentType: "application/json",
    });
    const maximumRows = canvasWidth >= 660 ? 2 : canvasWidth === 480 ? 3 : 4;
    expect(geometry.height).toBeLessThanOrEqual(
      geometry.verticalChrome +
        maximumRows * geometry.rowHeight +
        (maximumRows - 1) * geometry.rowGap,
    );
    expect(geometry.identityWidth).toBe(geometry.identityContentWidth);
    expect(geometry.identityWidth).toBeGreaterThan(250);
    expect(geometry.candidateValueHeight).toBeLessThanOrEqual(
      geometry.candidateHeight,
    );
    expect(geometry.canvasWidth).toBe(canvasWidth);
    expect(geometry.scrollWidth).toBeLessThanOrEqual(geometry.clientWidth);
    expect(geometry.outside).toEqual([]);
    expect(
      Math.abs(geometry.canvasBottom - geometry.bodyBottom),
    ).toBeLessThanOrEqual(1);
  });
}

test("short identity reserves no empty width and long identity respects its cap", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await openReviewFixture(page, "/tests/e2e/review-canvas.html?tour=rich");
  const canvas = page.getByRole("region", { name: "Review canvas" });
  await expect(canvas).toBeVisible();
  for (const title of [
    "Review",
    "A very long review title describing parser boundaries and module loading. ".repeat(
      4,
    ),
  ]) {
    const geometry = await canvas.evaluate((element, title) => {
      const text = element.querySelector<HTMLElement>(
        'header [title="feature/parser"]',
      )!;
      text.textContent = title;
      const identity = text.parentElement!;
      const header = identity.closest("header")!;
      const range = document.createRange();
      range.selectNodeContents(text);
      return {
        width: identity.clientWidth,
        contentWidth: identity.scrollWidth,
        naturalWidth: range.getBoundingClientRect().width,
        cap: parseFloat(getComputedStyle(identity).fontSize) * 48,
        headerWidth: header.clientWidth,
      };
    }, title);
    if (title === "Review") {
      expect(
        Math.abs(geometry.width - geometry.naturalWidth),
      ).toBeLessThanOrEqual(1);
      expect(geometry.width).toBe(geometry.contentWidth);
    } else {
      expect(geometry.width).toBeLessThanOrEqual(Math.ceil(geometry.cap));
      expect(geometry.contentWidth).toBeGreaterThan(geometry.width);
    }
  }
});
