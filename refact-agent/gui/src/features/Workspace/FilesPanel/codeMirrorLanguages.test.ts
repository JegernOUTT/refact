import { describe, expect, it } from "vitest";

import { resolveLanguageSupport } from "./codeMirrorLanguages";

describe("resolveLanguageSupport", () => {
  it("returns null when the backend reports no language", async () => {
    await expect(resolveLanguageSupport(null)).resolves.toBeNull();
  });

  it("treats plaintext as unhighlighted", async () => {
    await expect(resolveLanguageSupport("plaintext")).resolves.toBeNull();
  });

  it("returns null for languages it cannot map", async () => {
    await expect(
      resolveLanguageSupport("not-a-real-language"),
    ).resolves.toBeNull();
  });

  it.each([
    "typescript",
    "typescriptreact",
    "javascriptreact",
    "markdown",
    "shellscript",
    "python",
    "rust",
    "json",
    "yaml",
    "css",
    "html",
  ])("resolves the backend language id %s", async (language) => {
    await expect(resolveLanguageSupport(language)).resolves.not.toBeNull();
  });
});
