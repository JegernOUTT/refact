import { describe, it, expect } from "vitest";
import * as fs from "fs";
import * as path from "path";

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
