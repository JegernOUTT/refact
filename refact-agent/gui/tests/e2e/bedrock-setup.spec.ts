import { expect, test } from "@playwright/test";

for (const width of [360, 1280]) {
  test(`Bedrock stages stay compact and contained at ${width}px`, async ({
    page,
  }) => {
    await page.setViewportSize({ width, height: 900 });
    const longId = `arn:aws:bedrock:eu-west-1:123456789012:inference-profile/${"anthropic-long-model-id-".repeat(
      8,
    )}`;
    let probes = 0;
    let verifies = 0;
    let releaseVerify: (() => void) | undefined;
    const verifyGate = new Promise<void>((resolve) => {
      releaseVerify = resolve;
    });
    await page.route("**/v1/**", async (route) => {
      const path = new URL(route.request().url()).pathname;
      let body: unknown = {};
      if (path === "/v1/providers") body = { providers: [] };
      if (path === "/v1/providers/bedrock_team")
        body = {
          name: "bedrock_team",
          base_provider: "claude_code",
          display_name: "Bedrock team",
          enabled: true,
          readonly: false,
          status: "configured",
          has_credentials: false,
          settings: { backend: "bedrock" },
          runtime: null,
        };
      if (path.endsWith("/schema"))
        body = {
          name: "bedrock_team",
          schema: "description: Claude Code on Amazon Bedrock\nfields: {}",
        };
      if (path.endsWith("/available-models"))
        body = { models: [], source: "manual" };
      if (path.endsWith("/bedrock/profiles"))
        body = {
          profiles: [
            { name: "work", region: "eu-west-1", sso_configured: true },
          ],
          suggested_profile: "work",
          suggested_region: "eu-west-1",
        };
      if (path.endsWith("/bedrock/verify")) {
        verifies += 1;
        await verifyGate;
        body = {
          setup_id: "setup",
          config_revision: "revision",
          profile: "work",
          region: "eu-west-1",
          identity: {
            account: "123456789012",
            arn: `arn:aws:sts::123456789012:assumed-role/${"long-role-".repeat(
              20,
            )}`,
            user_id: "user",
          },
          inference_profiles: [],
          candidates: [{ model_id: longId }],
          warnings: ["Model invocation permissions must be tested explicitly."],
        };
      }
      if (path.endsWith("/bedrock/probe")) {
        probes += 1;
        body = {
          setup_id: "setup",
          results: [
            {
              model_id: longId,
              status: "denied",
              message:
                "This AWS identity does not have InvokeModel permission for the selected inference profile.",
            },
          ],
          warnings: [],
        };
      }
      await route.fulfill({ json: body });
    });
    await page.goto("/tests/e2e/bedrock-setup.html");
    await expect(page.getByLabel("AWS profile", { exact: true })).toHaveValue(
      "work",
    );
    expect(verifies).toBe(0);
    expect(probes).toBe(0);
    const description = page.getByText("Use AWS profiles on the engine host.", {
      exact: false,
    });
    const step = page.getByText("1. Choose profile and region");
    const before = await description.boundingBox();
    const after = await step.boundingBox();
    expect(before).not.toBeNull();
    expect(after).not.toBeNull();
    expect(after!.y - before!.y - before!.height).toBeLessThanOrEqual(16);
    await page
      .getByRole("button", { name: "Verify identity and discover models" })
      .click();
    await expect(
      page.getByRole("button", { name: "Verify identity and discover models" }),
    ).toBeDisabled();
    releaseVerify?.();
    await expect(page.getByText(/Verified account/)).toBeVisible();
    expect(probes).toBe(0);
    await page
      .getByRole("button", { name: "Test models", exact: true })
      .click();
    await expect(page.getByText(/Permission denied/)).toBeVisible();
    await expect(
      page.getByRole("button", { name: "Pin working model IDs" }),
    ).toBeDisabled();
    const overflow = await page.evaluate(() => ({
      document:
        document.documentElement.scrollWidth -
        document.documentElement.clientWidth,
      wizard: [
        ...document.querySelectorAll(
          "section[aria-label='Set up Amazon Bedrock'] *",
        ),
      ]
        .filter(
          (el) =>
            el.scrollWidth > el.clientWidth + 2 &&
            !["INPUT", "TEXTAREA"].includes(el.tagName),
        )
        .map((el) => el.tagName + "." + el.className),
    }));
    expect(overflow).toEqual({ document: 0, wizard: [] });
    await page.screenshot({
      path: `test-results/bedrock-${width}.png`,
      fullPage: true,
    });
  });
}
