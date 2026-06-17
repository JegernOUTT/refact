import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

const readProjectFile = (path: string) =>
  readFileSync(join(process.cwd(), path), "utf8");

describe("mobile viewport contract", () => {
  it("keeps app shells on the dynamic viewport", () => {
    const responsiveCss = readProjectFile("src/styles/responsive.css");
    const appCss = readProjectFile("src/features/App.module.css");

    expect(responsiveCss).toContain("height: 100dvh;");
    expect(appCss).toContain("height: 100dvh;");
  });

  it("lets the startup splash fit inside the safe-area padded app shell", () => {
    const splashCss = readProjectFile(
      "src/features/Splash/SplashScreen.module.css",
    );

    expect(splashCss).toContain("min-height: 0;");
    expect(splashCss).not.toContain("min-height: 100dvh;");
  });

  it("keeps safe-area insets available to the app root", () => {
    const responsiveCss = readProjectFile("src/styles/responsive.css");

    expect(responsiveCss).toContain("env(safe-area-inset-top, 0)");
    expect(responsiveCss).toContain("env(safe-area-inset-right, 0)");
    expect(responsiveCss).toContain("env(safe-area-inset-bottom, 0)");
    expect(responsiveCss).toContain("env(safe-area-inset-left, 0)");
  });

  it("enables safe-area viewport semantics in standalone shells", () => {
    const devIndexHtml = readProjectFile("index.html");
    const engineIndexHtml = readProjectFile("../engine/assets/chat/index.html");
    const routeShowcaseHtml = readProjectFile("tests/e2e/route-showcase.html");

    for (const html of [devIndexHtml, engineIndexHtml, routeShowcaseHtml]) {
      expect(html).toContain("viewport-fit=cover");
      expect(html).toContain("interactive-widget=resizes-content");
    }
  });
});
