import { describe, expect, test } from "vitest";
import {
  questionArtifactHtml,
  readQuestionArtifactAnswer,
} from "./questionArtifact";

describe("question HTML boundary", () => {
  test("injects restrictive policy before author content and keeps the draft-only API", () => {
    const html = questionArtifactHtml(
      '<html><head><base href="https://example.com"><meta http-equiv="refresh" content="0;url=https://example.com"></head><body><button onclick="refactQuestion.setAnswer(\'Mint\')">Mint</button></body></html>',
      "channel-1",
    );
    const doc = new DOMParser().parseFromString(html, "text/html");
    expect(doc.head.firstElementChild?.getAttribute("http-equiv")).toBe(
      "Content-Security-Policy",
    );
    expect(doc.head.firstElementChild?.getAttribute("content")).toContain(
      "connect-src 'none'",
    );
    expect(doc.querySelector("base")).toBeNull();
    expect(doc.querySelector('meta[http-equiv="refresh"]')).toBeNull();
    expect(html).toContain("window.refactQuestion=Object.freeze");
    expect(html).toContain("enabled=false");
    expect(html).not.toContain("submit_question_response");
  });

  test("accepts only bounded string drafts on the matching channel", () => {
    const value = {
      type: "refact-question-answer",
      channel: "mine",
      answer: "Mint",
    };
    expect(readQuestionArtifactAnswer(value, "mine")).toBe("Mint");
    expect(readQuestionArtifactAnswer(value, "other")).toBeNull();
    expect(
      readQuestionArtifactAnswer(
        { ...value, answer: { approved: true } },
        "mine",
      ),
    ).toBeNull();
    expect(
      readQuestionArtifactAnswer(
        { ...value, answer: "x".repeat(8193) },
        "mine",
      ),
    ).toBeNull();
    expect(
      readQuestionArtifactAnswer({ ...value, type: "submit" }, "mine"),
    ).toBeNull();
  });
});
