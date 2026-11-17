import { describe, expect, it } from "vitest";
import { parseArtifactLink } from "./artifactLinks";

describe("parseArtifactLink", () => {
  it.each([
    ["refact://artifact/chat-1/report.html", "chat-1", "report.html"],
    ["refact://artifact/c_2/reports/q3.html", "c_2", "reports/q3.html"],
    ["refact://artifact/c/My%20Report.md", "c", "My Report.md"],
    [
      "refact://artifact/c/%D0%BE%D1%82%D1%87%D0%B5%D1%82.csv",
      "c",
      "отчет.csv",
    ],
    ["refact://artifact/c/.hidden", "c", ".hidden"],
    ["refact://artifact/c/a..b", "c", "a..b"],
    [`refact://artifact/${"a".repeat(128)}/x`, "a".repeat(128), "x"],
    [`refact://artifact/c/${"x".repeat(1024)}`, "c", "x".repeat(1024)],
  ])("accepts %s", (href, chatId, relPath) => {
    expect(parseArtifactLink(href)).toEqual({ chatId, relPath });
  });

  it.each([
    [""],
    ["https://example.com/a.html"],
    ["refact://chat/chat-1"],
    ["refact://artifact/"],
    ["refact://artifact/chat-1"],
    ["refact://artifact/chat-1/"],
    ["refact://artifact//a.txt"],
    ["refact://artifact/chat.1/a.txt"],
    [`refact://artifact/${"a".repeat(129)}/x`],
    ["refact://artifact/c//a.txt"],
    ["refact://artifact/c/a/"],
    ["refact://artifact/c/./a.txt"],
    ["refact://artifact/c/../a.txt"],
    ["refact://artifact/c/%2E%2E/a.txt"],
    ["refact://artifact/c/a%2Fb.txt"],
    ["refact://artifact/c/a%5Cb.txt"],
    ["refact://artifact/c/a\\b.txt"],
    ["refact://artifact/c/a%00.txt"],
    ["refact://artifact/c/a%0A.txt"],
    ["refact://artifact/c/a%7F.txt"],
    ["refact://artifact/c/%E0%A4%A"],
    ["refact://artifact/c/a.txt?x=1"],
    ["refact://artifact/c/a.txt#frag"],
    ["REFACT://artifact/c/a.txt"],
    [`refact://artifact/c/${"x".repeat(1025)}`],
    [`refact://artifact/c/${"x".repeat(600)}/${"y".repeat(424)}`],
  ])("rejects %s", (href) => {
    expect(parseArtifactLink(href)).toBeNull();
  });

  it("rejects missing hrefs", () => {
    expect(parseArtifactLink(undefined)).toBeNull();
    expect(parseArtifactLink(null)).toBeNull();
  });
});
