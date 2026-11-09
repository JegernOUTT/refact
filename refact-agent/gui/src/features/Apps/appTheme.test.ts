import { afterEach, describe, expect, it } from "vitest";
import { APP_THEME_BASE_CSS } from "./appTheme";

afterEach(() => {
  document.head.innerHTML = "";
  document.body.innerHTML = "";
});

function install(html: string, authoredCss = "") {
  document.head.innerHTML = `<style>:root{--rf-space-2:8px;--rf-space-3:12px;--rf-weight-medium:500;--rf-color-accent:blue}</style><style>${APP_THEME_BASE_CSS}</style><style>${authoredCss}</style>`;
  document.body.innerHTML = html;
}

function element(selector: string): Element {
  const found = document.querySelector(selector);
  if (!found) throw new Error(`Missing ${selector}`);
  return found;
}

describe("iframe base layout contract", () => {
  it("leaves viewport padding and scrolling to the app, with explicit document padding opt-in", () => {
    install('<main class="refact-app-content">Counter</main>');
    expect(getComputedStyle(document.body).padding).toBe("0px");
    expect(getComputedStyle(element("main")).padding).toBe("12px");
    expect(APP_THEME_BASE_CSS).not.toMatch(/overflow(?:-[xy])?:(hidden|clip)/);
    expect(APP_THEME_BASE_CSS).not.toContain("scrollbar-gutter");
    expect(APP_THEME_BASE_CSS).toContain("box-sizing:border-box");
    install("", "body{padding:24px;box-sizing:content-box}");
    expect(getComputedStyle(document.body).padding).toBe("24px");
    expect(getComputedStyle(document.body).boxSizing).toBe("content-box");
  });

  it("gives opaque iframe scrollers tokenized Firefox and WebKit defaults", () => {
    expect(APP_THEME_BASE_CSS).toContain("scrollbar-width:thin");
    expect(APP_THEME_BASE_CSS).toContain(
      "scrollbar-color:var(--rf-app-scrollbar-thumb) var(--rf-app-scrollbar-track)",
    );
    expect(APP_THEME_BASE_CSS).toContain("::-webkit-scrollbar{width:var(");
    expect(APP_THEME_BASE_CSS).toContain("::-webkit-scrollbar-track");
    expect(APP_THEME_BASE_CSS).toContain("::-webkit-scrollbar-thumb:hover");
    expect(APP_THEME_BASE_CSS).not.toMatch(/#[0-9a-f]{3,8}\b|rgba?\(/i);
  });

  it("preserves native compact controls and authored overrides", () => {
    install(
      '<input type="checkbox"><input type="range"><input type="file"><progress></progress><details><summary>More</summary></details>',
      "input[type=range]{padding:4px}summary{font-weight:700}",
    );
    expect(getComputedStyle(element("input")).padding).toBe("0px");
    expect(getComputedStyle(element("input[type=range]")).padding).toBe("4px");
    expect(getComputedStyle(element("summary")).fontWeight).toBe("700");
    expect(APP_THEME_BASE_CSS).toContain(
      "::file-selector-button{font:inherit;color:inherit",
    );
    expect(APP_THEME_BASE_CSS).toContain(
      "::-webkit-progress-value{background:var(--rf-color-accent)",
    );
  });
});
