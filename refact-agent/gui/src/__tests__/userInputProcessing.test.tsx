import { describe, it, expect } from "vitest";
import * as fs from "fs";
import * as path from "path";

describe("UserInput processing", () => {
  it("uses functional array methods (filter/map) for text extraction", () => {
    const filePath = path.resolve(
      __dirname,
      "../components/ChatContent/UserInput.tsx",
    );
    const content = fs.readFileSync(filePath, "utf-8");

    // Current implementation uses filter/map for processing
    expect(content).toContain(".filter(");
    expect(content).toContain(".map(");
    // Should not use recursive patterns that could cause stack overflow
    expect(content).not.toMatch(
      /function processLines\([^)]*\):[^{]*\{[^}]*return processLines\(/,
    );
  });

  it("uses useMemo for memoized content extraction", () => {
    const filePath = path.resolve(
      __dirname,
      "../components/ChatContent/UserInput.tsx",
    );
    const content = fs.readFileSync(filePath, "utf-8");

    // Current implementation uses useMemo for performance
    expect(content).toContain("useMemo");
    expect(content).toContain("textContent");
  });

  it("extracts images separately from text content", () => {
    const filePath = path.resolve(
      __dirname,
      "../components/ChatContent/UserInput.tsx",
    );
    const content = fs.readFileSync(filePath, "utf-8");

    // Should have separate image extraction logic
    expect(content).toContain("images");
    expect(content).toContain("image_url");
  });
});

describe("URL sanitization in AssistantInput", () => {
  it("filters citations by URL protocol", () => {
    const filePath = path.resolve(
      __dirname,
      "../components/ChatContent/AssistantInput.tsx",
    );
    const content = fs.readFileSync(filePath, "utf-8");

    expect(content).toContain('url.protocol === "http:"');
    expect(content).toContain('url.protocol === "https:"');
  });
});

describe("DiffTitle uses numeric counts", () => {
  it("displays counts instead of repeated characters", () => {
    const filePath = path.resolve(
      __dirname,
      "../components/ChatContent/DiffContent.tsx",
    );
    const content = fs.readFileSync(filePath, "utf-8");

    expect(content).toContain("addCount");
    expect(content).toContain("removeCount");
    const greenIdx = content.indexOf("+{addCount}");
    const redIdx = content.indexOf("-{removeCount}");
    expect(greenIdx).toBeLessThan(redIdx);
    expect(content).not.toContain('"+".repeat');
    expect(content).not.toContain('"-".repeat');
  });
});
