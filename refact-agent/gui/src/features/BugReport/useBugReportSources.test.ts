import { describe, expect, it } from "vitest";

import {
  detectLineLevel,
  lineMatchesFilter,
  type LogLine,
} from "./useBugReportSources";

describe("detectLineLevel", () => {
  it("detects tracing-style levels", () => {
    expect(detectLineLevel("12:00:00 ERROR chat/generation.rs boom")).toBe(
      "error",
    );
    expect(detectLineLevel("12:00:00 WARN slow heartbeat")).toBe("warn");
    expect(detectLineLevel("12:00:00 WARNING slow heartbeat")).toBe("warn");
    expect(detectLineLevel("12:00:00 INFO GET /v1/ping 200")).toBe("info");
    expect(detectLineLevel("12:00:00 DEBUG drained queue")).toBe("debug");
    expect(detectLineLevel("12:00:00 TRACE tick")).toBe("debug");
    expect(detectLineLevel("plain line")).toBe("unknown");
  });

  it("does not match level tokens inside words", () => {
    expect(detectLineLevel("PROCESSERROR5 something")).toBe("unknown");
  });
});

describe("lineMatchesFilter", () => {
  const line: LogLine = {
    text: "12:00:00 ERROR chat/generation.rs Context too large",
    level: "error",
  };

  it("matches all levels with empty filter", () => {
    expect(lineMatchesFilter(line, "", "all")).toBe(true);
  });

  it("filters by level", () => {
    expect(lineMatchesFilter(line, "", "error")).toBe(true);
    expect(lineMatchesFilter(line, "", "warn")).toBe(false);
  });

  it("filters by case-insensitive substring", () => {
    expect(lineMatchesFilter(line, "context TOO", "all")).toBe(true);
    expect(lineMatchesFilter(line, "nomatch", "all")).toBe(false);
  });

  it("combines level and text filters", () => {
    expect(lineMatchesFilter(line, "context", "error")).toBe(true);
    expect(lineMatchesFilter(line, "context", "info")).toBe(false);
  });
});
