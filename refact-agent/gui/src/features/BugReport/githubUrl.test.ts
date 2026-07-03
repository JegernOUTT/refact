import { describe, expect, it } from "vitest";

import {
  BUG_REPORT_REPO,
  GITHUB_BODY_CHAR_LIMIT,
  GITHUB_TITLE_CHAR_LIMIT,
  buildGithubIssueUrl,
  truncateIssueBody,
} from "./githubUrl";

describe("buildGithubIssueUrl", () => {
  it("builds a prefilled new-issue url with title, labels and body", () => {
    const url = new URL(
      buildGithubIssueUrl({
        title: "Chat dies with context loop",
        labels: ["type/bug", "needs-triage"],
        body: "### What happened\nboom",
      }),
    );
    expect(url.origin).toBe("https://github.com");
    expect(url.pathname).toBe(`/${BUG_REPORT_REPO}/issues/new`);
    expect(url.searchParams.get("title")).toBe("Chat dies with context loop");
    expect(url.searchParams.get("labels")).toBe("type/bug,needs-triage");
    expect(url.searchParams.get("body")).toBe("### What happened\nboom");
  });

  it("omits empty title and labels", () => {
    const url = new URL(
      buildGithubIssueUrl({ title: "  ", labels: [], body: "b" }),
    );
    expect(url.searchParams.has("title")).toBe(false);
    expect(url.searchParams.has("labels")).toBe(false);
  });

  it("truncates oversized bodies with a notice", () => {
    const body = "x".repeat(GITHUB_BODY_CHAR_LIMIT + 500);
    const truncated = truncateIssueBody(body);
    expect(truncated.length).toBeLessThanOrEqual(GITHUB_BODY_CHAR_LIMIT);
    expect(truncated.endsWith("bundle)")).toBe(true);
    const url = new URL(buildGithubIssueUrl({ title: "t", labels: [], body }));
    expect(url.searchParams.get("body")).toBe(truncated);
  });

  it("keeps short bodies unchanged", () => {
    expect(truncateIssueBody("short")).toBe("short");
  });

  it("caps oversized titles", () => {
    const url = new URL(
      buildGithubIssueUrl({
        title: "t".repeat(GITHUB_TITLE_CHAR_LIMIT + 100),
        labels: [],
        body: "b",
      }),
    );
    expect(url.searchParams.get("title")).toHaveLength(GITHUB_TITLE_CHAR_LIMIT);
  });
});
