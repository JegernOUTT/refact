import { afterEach, describe, expect, test } from "vitest";
import { resolveConcreteAppearance } from "./useAppearance";

describe("resolveConcreteAppearance", () => {
  afterEach(() => {
    document.body.className = "";
  });

  test("passes explicit values through", () => {
    expect(resolveConcreteAppearance("dark", false)).toBe("dark");
    expect(resolveConcreteAppearance("light", true)).toBe("light");
  });

  test("resolves inherit from vscode body classes", () => {
    document.body.classList.add("vscode-dark");
    expect(resolveConcreteAppearance("inherit", false)).toBe("dark");

    document.body.className = "";
    document.body.classList.add("vscode-light");
    expect(resolveConcreteAppearance("inherit", true)).toBe("light");

    document.body.className = "";
    document.body.classList.add("vscode-high-contrast");
    expect(resolveConcreteAppearance("inherit", false)).toBe("dark");

    document.body.className = "";
    document.body.classList.add("vscode-high-contrast-light");
    expect(resolveConcreteAppearance("inherit", true)).toBe("light");
  });

  test("falls back to the system preference for inherit", () => {
    expect(resolveConcreteAppearance("inherit", true)).toBe("dark");
    expect(resolveConcreteAppearance("inherit", false)).toBe("light");
  });

  test("treats undefined like inherit", () => {
    expect(resolveConcreteAppearance(undefined, true)).toBe("dark");
    document.body.classList.add("vscode-light");
    expect(resolveConcreteAppearance(undefined, true)).toBe("light");
  });
});
