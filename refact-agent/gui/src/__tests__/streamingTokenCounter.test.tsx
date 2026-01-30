import { describe, it, expect } from "vitest";
import * as fs from "fs";
import * as path from "path";

describe("StreamingTokenCounter", () => {
  it("has cleanup return in visibility useEffect", () => {
    const filePath = path.resolve(
      __dirname,
      "../components/UsageCounter/StreamingTokenCounter.tsx",
    );
    const content = fs.readFileSync(filePath, "utf-8");

    expect(content).toContain("return () => {");
    expect(content).toContain("window.clearTimeout(hideTimerRef.current)");
  });

  it("uses formatNumberToFixed for token display", () => {
    const filePath = path.resolve(
      __dirname,
      "../components/UsageCounter/StreamingTokenCounter.tsx",
    );
    const content = fs.readFileSync(filePath, "utf-8");

    expect(content).toContain("formatNumberToFixed");
  });
});
