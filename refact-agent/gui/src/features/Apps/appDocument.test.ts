import { describe, expect, it } from "vitest";
import {
  APP_DOCUMENT_CSP,
  APP_IFRAME_SANDBOX,
  buildAppDocument,
  serializeForScript,
} from "./appDocument";
import {
  APP_THEME_BASE_CSS,
  sanitizeAppThemeTokens,
  serializeAppThemeTokens,
} from "./appTheme";

const base = { channel: "abcdef0123456789", generation: 1 };

describe("buildAppDocument", () => {
  it("injects CSP and SDK before app html", () => {
    const doc = buildAppDocument({
      ...base,
      html: "<!doctype html><html><body>hi</body></html>",
    });
    const csp = doc.indexOf("Content-Security-Policy");
    const sdk = doc.indexOf("<script>");
    expect(csp).toBeGreaterThan(0);
    expect(sdk).toBeGreaterThan(csp);
    expect(doc.indexOf("<body>hi")).toBeGreaterThan(sdk);
    expect(doc.toLowerCase().match(/<!doctype/g)).toHaveLength(1);
  });
  it("CSP forbids network, eval, frames and forms", () => {
    expect(APP_DOCUMENT_CSP).toContain("default-src 'none'");
    expect(APP_DOCUMENT_CSP).toContain("connect-src 'none'");
    expect(APP_DOCUMENT_CSP).toContain("form-action 'none'");
    expect(APP_DOCUMENT_CSP).not.toContain("unsafe-eval");
    expect(APP_DOCUMENT_CSP).not.toMatch(/https?:|\*/);
    expect(APP_IFRAME_SANDBOX).toBe("allow-scripts");
  });
  it("escapes state so it cannot break out of the script", () => {
    const doc = buildAppDocument({
      ...base,
      html: "",
      initialState: { x: "</script><script>alert(1)</script>" },
    });
    expect(doc.match(/<\/script>/g)).toHaveLength(1);
    expect(serializeForScript("\u2028<")).toBe('"\\u2028\\u003c"');
  });
  it("validates channel and generation", () => {
    expect(() =>
      buildAppDocument({ ...base, html: "", channel: 'bad"' }),
    ).toThrow();
    expect(() =>
      buildAppDocument({ ...base, html: "", generation: -1 }),
    ).toThrow();
  });
  it("embeds channel and generation", () => {
    const doc = buildAppDocument({
      ...base,
      generation: 7,
      html: "",
      appearance: "dark",
    });
    expect(doc).toContain('"channel":"abcdef0123456789"');
    expect(doc).toContain('"generation":7');
    expect(doc).toContain('data-refact-appearance="dark"');
  });
  it("embeds allowlisted tokens and zero-specificity base styles before app html", () => {
    const doc = buildAppDocument({
      ...base,
      appearance: "dark",
      html: "<style>:root{--rf-bg:hotpink}</style><p>x</p>",
      themeTokens: {
        "--rf-bg": "#0c0d0f",
        "--rf-color-fg": "rgba(255, 255, 255, 0.92)",
        "--rf-app-font-size": "13px",
      },
    });
    expect(doc).toContain(
      '<style id="refact-app-theme-tokens">:root{--rf-bg:#0c0d0f;--rf-color-fg:rgba(255, 255, 255, 0.92);--rf-app-font-size:13px;}</style>',
    );
    expect(doc.indexOf("refact-app-theme-base")).toBeLessThan(
      doc.indexOf("refact-app-theme-tokens"),
    );
    expect(doc.indexOf("refact-app-theme-tokens")).toBeLessThan(
      doc.indexOf("--rf-bg:hotpink"),
    );
    expect(doc).toContain('<meta name="color-scheme" content="dark">');
    expect(APP_THEME_BASE_CSS).toContain("var(--rf-color-fg");
    expect(APP_THEME_BASE_CSS).not.toMatch(/#[0-9a-f]{3,8}\b|rgba?\(/i);
    expect(APP_THEME_BASE_CSS).not.toMatch(/[<>]/);
  });
  it("rejects malicious token names and values", () => {
    const hostile = {
      "--rf-bg": "red;}</style><script>alert(1)</script>",
      "--rf-color-fg": "url(https://evil/x)",
      "--rf-color-muted": "red} body{display:none",
      "--rf-border": "expression(alert(1))",
      "--rf-surface-1": "red /* x",
      "--rf-surface-2": '"unterminated',
      "--rf-surface-3": "env(safe-area-inset-top)",
      "--rf-scrim": "a".repeat(500),
      "--rf-color-accent": "#7f93d8",
      "--evil": "red",
      "font-family": "x",
    };
    expect(sanitizeAppThemeTokens(hostile)).toEqual({
      "--rf-color-accent": "#7f93d8",
    });
    const doc = buildAppDocument({ ...base, html: "", themeTokens: hostile });
    expect(doc).not.toContain("evil");
    expect(doc).not.toContain("alert");
    expect(doc.match(/<\/script>/g)).toHaveLength(1);
    expect(serializeAppThemeTokens(hostile)).toBe(
      ":root{--rf-color-accent:#7f93d8;}",
    );
  });
});
