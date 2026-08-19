import { describe, expect, it } from "vitest";
import fs from "fs";
import path from "path";

/**
 * Characterization test for the global-CSS cascade contract.
 *
 * Component CSS modules win equal-specificity battles against Radix
 * defaults (`.rt-Box { display: block }` vs a module's `display: flex`)
 * only because the global stylesheets are emitted at the HEAD of the
 * bundle. That is guaranteed by importing them first in the library
 * entry; if they were only reachable through a component (Theme.tsx),
 * their cascade position would float with the module graph and flip on
 * unrelated refactors — which shipped a 0-height task workspace once.
 */
describe("lib entry global CSS order", () => {
  const source = fs.readFileSync(path.join(__dirname, "index.ts"), "utf8");
  const imports = [
    ...source.matchAll(/^import\s+(?:.*?from\s+)?"([^"]+)";?$/gm),
  ].map((m) => m[1]);

  it("imports @radix-ui/themes/styles.css before everything else", () => {
    expect(imports[0]).toBe("@radix-ui/themes/styles.css");
  });

  it("imports every global stylesheet before any non-CSS module", () => {
    const firstNonCss = imports.findIndex((s) => !s.endsWith(".css"));
    const globalSheets = [
      "@radix-ui/themes/styles.css",
      "../styles/tokens.css",
      "../styles/base.css",
      "../styles/glass.css",
      "../styles/motion.css",
      "../styles/responsive.css",
      "../styles/scrollbar.css",
      "../components/Theme/theme-config.css",
      "../components/shared/tokens.css",
    ];
    for (const sheet of globalSheets) {
      const idx = imports.indexOf(sheet);
      expect(
        idx,
        `${sheet} must be imported in src/lib/index.ts`,
      ).toBeGreaterThanOrEqual(0);
      expect(
        idx,
        `${sheet} must precede the first non-CSS import`,
      ).toBeLessThan(firstNonCss === -1 ? imports.length : firstNonCss);
    }
  });

  it("keeps tokens.css imported after the Radix stylesheet", () => {
    expect(imports.indexOf("../styles/tokens.css")).toBeGreaterThan(
      imports.indexOf("@radix-ui/themes/styles.css"),
    );
  });
});
