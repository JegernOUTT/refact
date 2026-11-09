import { expect, test } from "@playwright/test";

for (const width of [240, 360, 768, 1280]) {
  for (const theme of ["light", "dark"]) {
    test(`worktree cards and detailed review at ${width}px ${theme}`, async ({
      page,
    }, testInfo) => {
      test.setTimeout(60_000);
      await page.setViewportSize({ width: Math.max(width, 768), height: 900 });
      await page.goto(
        `/tests/e2e/route-showcase.html?route=worktrees&theme=${theme}`,
      );
      await page.getByTestId("worktree-control-trigger").click();
      if (width < 768) {
        await page.setViewportSize({ width, height: 900 });
      }
      const selected = page.getByRole("article", {
        name: "refact/chat/worktree-polish",
        exact: true,
      });
      await expect(selected).toHaveAttribute("data-selected", "true");
      await expect(selected).toContainText("304 MiB");
      await expect(selected).toContainText("Not in base");
      const cards = page.getByRole("article");
      await expect(cards.first()).toHaveAccessibleName(
        "refact/chat/worktree-polish",
      );
      const merged = page.getByRole("article", {
        name: "refact/chat/finished-polish",
        exact: true,
      });
      await expect(merged).toContainText("Merged into base");
      await expect(merged).toContainText("2.0 MiB");
      const unknown = page.getByRole("article", {
        name: "refact/chat/recovered-checkout",
        exact: true,
      });
      await expect(unknown).toContainText("Size unknown");
      await expect(unknown).toContainText("Merge status unknown");
      for (const card of [selected, merged, unknown]) {
        const actions = card.getByRole("group");
        await expect(actions.getByRole("button")).toHaveCount(5);
        expect(await actions.getByRole("button").first().textContent()).toBe(
          "",
        );
        const identity = await card.locator("button").first().boundingBox();
        const footer = await actions.boundingBox();
        expect(identity).not.toBeNull();
        expect(footer).not.toBeNull();
        expect(footer!.y).toBeGreaterThanOrEqual(
          identity!.y + identity!.height,
        );
      }
      await expect(
        page
          .getByRole("article", {
            name: "refact/task/workspace/cards/T-17",
            exact: true,
          })
          .getByRole("button", { name: /^Merge worktree/ }),
      ).toBeDisabled();
      expect(
        await page.evaluate(
          () => document.documentElement.scrollWidth <= window.innerWidth,
        ),
      ).toBe(true);
      await testInfo.attach("worktree-cards", {
        body: await page.screenshot(),
        contentType: "image/png",
      });
      await merged.getByRole("button", { name: /^View worktree diff/ }).click();
      const review = page.getByRole("region", {
        name: "Worktree diff",
        exact: true,
      });
      await expect(review).toBeVisible();
      await expect(
        review
          .getByRole("navigation", { name: "All changed files and layers" })
          .getByRole("button"),
      ).toHaveCount(4);
      await expect(review.locator(".cm-editor").first()).toBeVisible();
      await expect(review).toContainText("Session-only review");
      const files = review.getByRole("navigation", {
        name: "All changed files and layers",
      });
      await files
        .getByRole("button", { name: /src\/worktrees.ts.*staged ·/ })
        .first()
        .click();
      await expect(review.locator(".cm-editor").first()).toBeVisible();
      await files.getByRole("button", { name: /notes\/review.md/ }).click();
      await expect(review.locator(".cm-editor").last()).toContainText(
        "Review checklist",
      );
      expect(
        await page.evaluate(
          () => document.documentElement.scrollWidth <= window.innerWidth,
        ),
      ).toBe(true);
      const scopeChanges = await page.evaluate(() => {
        const requests = (
          window as unknown as { __routeShowcaseRequests: { body: unknown }[] }
        ).__routeShowcaseRequests;
        return requests.filter(
          (request) =>
            request.body &&
            typeof request.body === "object" &&
            "type" in request.body &&
            request.body.type === "set_params",
        );
      });
      expect(scopeChanges).toEqual([]);
      await testInfo.attach("worktree-detailed-diff", {
        body: await page.screenshot(),
        contentType: "image/png",
      });
      await page
        .getByRole("button", {
          name: "Close Diff · refact/chat/finished-polish",
          exact: true,
        })
        .click();
      await expect(review).toHaveCount(0);
      await expect(
        page.getByTestId("worktree-control-trigger"),
      ).toHaveAttribute(
        "aria-label",
        "Worktree scope: refact/chat/worktree-polish",
      );
    });
  }
}
