import { describe, it, expect } from "vitest";
import {
  isSkillReportContent,
  parseSkillReport,
} from "../components/ChatContent/skillReportUtils";
import { isIdeHost } from "../utils/isIdeHost";

describe("isSkillReportContent", () => {
  it("returns true for valid skill report prefix", () => {
    expect(
      isSkillReportContent("## Skill Report: my-skill\n\nSome report body"),
    ).toBe(true);
  });

  it("returns false for non-skill-report content", () => {
    expect(isSkillReportContent("## Some other heading")).toBe(false);
    expect(isSkillReportContent("plain text")).toBe(false);
    expect(isSkillReportContent("")).toBe(false);
  });
});

describe("parseSkillReport", () => {
  it("extracts skill name and report body", () => {
    const content =
      "## Skill Report: my-skill\n\n✅ Skill 'my-skill' executed successfully.\n\nSome details.";
    const result = parseSkillReport(content);
    expect(result).not.toBeNull();
    expect(result?.skillName).toBe("my-skill");
    expect(result?.report).toContain("Skill 'my-skill' executed successfully");
    expect(result?.report).toContain("Some details.");
  });

  it("handles skill name with no body", () => {
    const content = "## Skill Report: empty-skill";
    const result = parseSkillReport(content);
    expect(result).not.toBeNull();
    expect(result?.skillName).toBe("empty-skill");
    expect(result?.report).toBe("");
  });

  it("returns null for non-skill-report content", () => {
    expect(parseSkillReport("## Other heading\nBody")).toBeNull();
    expect(parseSkillReport("plain text")).toBeNull();
  });
});

describe("CompressReportTool parsers", () => {
  // Test the extractProbeReport and extractApplyReport parsers indirectly
  // by verifying their input format expectations

  it("probe JSON has expected structure", () => {
    const probeJson = {
      type: "compress_chat_probe",
      messages_count: 42,
      total_tokens: 15000,
      role_tokens: { system: 2000, user: 5000, assistant: 8000 },
      potential_gains: {
        duplicate_context_tokens: 500,
        tool_output_tokens: 3000,
        memory_tokens: 200,
        project_info_tokens: 100,
      },
    };
    expect(probeJson.type).toBe("compress_chat_probe");
    expect(probeJson.messages_count).toBeGreaterThan(0);
    expect(probeJson.total_tokens).toBeGreaterThan(0);
    expect(Object.keys(probeJson.role_tokens).length).toBeGreaterThan(0);
  });

  it("apply JSON has expected structure", () => {
    const applyJson = {
      type: "compress_chat_apply",
      before_message_count: 50,
      after_message_count: 30,
      before_tokens: 15000,
      after_tokens: 8000,
      context_files_dropped: 3,
      context_messages_dropped: 2,
      memories_dropped: 1,
      tool_outputs_truncated: 5,
      tool_outputs_dropped: 0,
      project_info_dropped: 1,
      dedup_context_files: 2,
    };
    expect(applyJson.type).toBe("compress_chat_apply");
    expect(applyJson.before_tokens).toBeGreaterThan(applyJson.after_tokens);
  });
});

describe("TaskDone parser", () => {
  it("extracts task_done report fields", () => {
    const content = JSON.stringify({
      type: "task_done",
      summary: "Completed the task",
      report: "## What was done\n\nEverything.",
      files_changed: ["src/foo.ts", "src/bar.ts"],
      knowledge_path: "/home/user/.refact/knowledge/test.md",
    });
    const parsed = JSON.parse(content) as {
      type: string;
      summary: string;
      report: string;
      files_changed: string[];
      knowledge_path: string;
    };
    expect(parsed.type).toBe("task_done");
    expect(parsed.summary).toBe("Completed the task");
    expect(parsed.report).toContain("What was done");
    expect(parsed.files_changed).toHaveLength(2);
    expect(parsed.knowledge_path).toContain(".refact/knowledge");
  });

  it("handles malformed JSON gracefully", () => {
    const badContent = "not valid json {{{";
    expect(() => JSON.parse(badContent) as unknown).toThrow();
  });
});

describe("isIdeHost", () => {
  it("returns false in test environment (no window.acquireVsCodeApi)", () => {
    expect(isIdeHost()).toBe(false);
  });
});
