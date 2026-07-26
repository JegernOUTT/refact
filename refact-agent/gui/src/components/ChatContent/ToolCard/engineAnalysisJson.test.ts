import { describe, expect, it } from "vitest";
import {
  buildAnalysisReport,
  parseEngineAnalysisJson,
} from "./engineAnalysisJson";

const finding = {
  path: "/home/u/app/src/a.ts",
  biomarker: "entropy",
  category: "git",
  dimension: "defect",
  severity: "Critical",
  line: 7,
  detail: "Changed often",
};

function report(tool: string, value: Record<string, unknown>) {
  const result = buildAnalysisReport(tool, {
    tool,
    summary: `${tool} summary`,
    ...value,
  });
  expect(result).not.toBeNull();
  if (!result) throw new Error("expected report");
  const lines = result.sections.flatMap((section) => [
    section.line,
    ...section.rows.map((row) => row.line),
  ]);
  expect(new Set(lines).size).toBe(lines.length);
  return result;
}

describe("parseEngineAnalysisJson", () => {
  it("accepts an object", () =>
    expect(parseEngineAnalysisJson('{"tool":"dead_code"}')).toEqual({
      tool: "dead_code",
    }));
  it("rejects invalid JSON", () =>
    expect(parseEngineAnalysisJson("not json")).toBeNull());
  it("rejects a JSON scalar", () =>
    expect(parseEngineAnalysisJson("42")).toBeNull());
});

describe("buildAnalysisReport", () => {
  it("maps codegraph_overview structurally", () => {
    const result = report("codegraph_overview", {
      counts: { nodes: 10, edges: 20, files: 2 },
      index_state: { queued: 0, cross_file_edges: 5, cross_file_ready: true },
      scc_count: 1,
      largest_scc: 3,
      component_count: 2,
      community_count: 1,
      dead_code_count: 1,
      partial: false,
      top_pagerank: [
        { symbol: "main", path: "/home/u/app/src/main.ts", score: 0.01234 },
      ],
      top_betweenness: [],
      file_centrality: { top_pagerank: [], top_betweenness: [] },
      communities: [],
      execution_flows: [],
      dead_code: [],
      entry_points: ["/home/u/app/src/main.ts"],
      api_contract_files: ["/home/u/app/src/api.ts"],
    });
    expect(result.facts).toContainEqual({ key: "Nodes", value: "10" });
    expect(result.sections.map((section) => section.title)).toContain(
      "Most central symbols (PageRank)",
    );
    expect(result.sections[0]?.rows[0]).toMatchObject({
      title: "main",
      paths: ["/home/u/app/src/main.ts"],
      lead: "0.0123",
    });
    expect(result.pathPrefix).toBe("/home/u/app/src/");
  });

  it("maps git_risk tags, severity, and details", () => {
    const result = report("git_risk", {
      commits_analyzed: 100,
      agent_authored_pct: 2,
      hotspots: [
        {
          path: "/home/u/app/src/a.ts",
          churn: 9,
          risk: 0.8,
          churn_risk: 0.7,
          churn_percentile: 0.9,
          temporal_score: 2,
          change_entropy: 1,
          change_entropy_pct: 0.5,
          bus_factor: 1,
          ownership_risk: true,
          knowledge_loss: true,
        },
      ],
      ownership: [],
      co_change: [],
      coupling: [],
      reviewers: [],
      findings: [finding],
      recent_commit_risks: [
        {
          sha: "abc",
          summary: "large change",
          risk: 0.9,
          top_factor_names: ["entropy"],
        },
      ],
    });
    expect(result.facts[0]).toEqual({ key: "Commits analyzed", value: "100" });
    expect(result.sections[0]?.rows[0]?.tags).toEqual([
      "ownership-risk",
      "knowledge-loss",
    ]);
    expect(
      result.sections.find(
        (section) => section.title === "Git-driven biomarkers",
      )?.rows[0],
    ).toMatchObject({
      title: "entropy",
      detail: "Changed often",
      severity: "Critical",
      paths: ["/home/u/app/src/a.ts"],
    });
    expect(
      result.sections.find(
        (section) => section.title === "Recent commit change-risk",
      )?.rows[0]?.tags,
    ).toEqual(["entropy"]);
  });

  it("maps security_scan findings", () => {
    const result = report("security_scan", {
      path: "/home/u/app/src/server.ts",
      lang: "TypeScript",
      finding_count: 1,
      counts: { Critical: 1, High: 0, Medium: 0, Low: 0 },
      findings: [
        {
          rule: "dangerous-eval",
          severity: "Critical",
          line: 12,
          snippet: "eval(input)",
        },
      ],
      omitted: 0,
    });
    expect(result.facts).toContainEqual({ key: "Findings", value: "1" });
    expect(result.sections[0]?.title).toBe("Findings");
    expect(result.sections[0]?.rows[0]).toMatchObject({
      title: "dangerous-eval",
      detail: "eval(input)",
      severity: "Critical",
      paths: ["/home/u/app/src/server.ts"],
    });
  });

  it("maps pr_blast impacts", () => {
    const result = report("pr_blast", {
      max_depth: 3,
      changed_files: ["src/main.ts"],
      directly_impacted: [
        {
          path: "/home/u/app/src/a.ts",
          symbol: "render",
          distance: 1,
          via: "calls",
          kind: "behavioral",
        },
      ],
      transitively_impacted: [],
      impacted_file_count: 1,
      risk_score: 0.62,
      suggested_reviewers: [],
      index_state: { queued: 1 },
      partial: true,
      warning: "index building",
    });
    expect(result.facts).toContainEqual({ key: "Risk", value: "0.62" });
    expect(result.warnings).toEqual(["index building"]);
    expect(result.sections[0]?.rows[0]).toMatchObject({
      title: "render",
      detail: "Reached via calls (behavioral)",
      paths: ["/home/u/app/src/a.ts"],
      tags: ["behavioral"],
    });
  });

  it("groups dead_code by path", () => {
    const result = report("dead_code", {
      entries: [
        {
          name: "unused",
          path: "/home/u/app/src/a.ts",
          line: 9,
          reason: "No callers",
          confidence: 0.91,
          git_recency: 40,
          incoming_edges: 0,
        },
      ],
      shown: 1,
      total_candidates: 4,
      index_state: {
        queued: 0,
        dirty_paths: 0,
        pending_refs: 0,
        cross_file_edges: 3,
        cross_file_ready: true,
      },
      partial: false,
    });
    expect(result.facts).toContainEqual({ key: "Matching", value: "4" });
    expect(result.sections[0]).toMatchObject({
      title: "/home/u/app/src/a.ts",
      titleIsPath: true,
    });
    expect(result.sections[0]?.rows[0]).toMatchObject({
      title: "unused",
      detail: "No callers",
    });
  });

  it("maps code_health functions and call graph", () => {
    const result = report("code_health", {
      index_state: { queued: 0 },
      aggregate: {
        file_count: 1,
        function_count: 1,
        avg_score: 88,
        grade: "A",
        max_complexity: 4,
        avg_maintainability: 80,
        avg_maintainability_index: 80,
        avg_maintainability_signal: 1,
        avg_duplication_pct: 0,
        biomarker_count: 0,
        refactoring_count: 0,
      },
      files: [
        {
          path: "/home/u/app/src/a.ts",
          lang: "TypeScript",
          score: 88,
          grade: "A",
          complexity: 4,
          maintainability: 80,
          maintainability_index: 80,
          maintainability_signal: 1,
          max_complexity: 4,
          avg_maintainability: 80,
          function_count: 1,
          duplication_pct: 0,
          dry_violation: false,
          defect_score: 0,
          maintainability_score: 1,
          performance_score: 1,
          biomarker_count: 0,
          refactoring_count: 0,
          functions: [
            {
              name: "run",
              line1: 3,
              complexity: 2,
              nesting: 1,
              loc: 8,
              maintainability: 80,
              maintainability_index: 80,
            },
          ],
          findings: [],
          health_impact: [],
          cache_hit: false,
          refactorings: [],
        },
      ],
      call_graph: [{ caller: "run", callee: "save" }],
      warm_cache: false,
    });
    expect(result.facts).toContainEqual({ key: "Grade", value: "A" });
    expect(
      result.sections.find((section) => section.title === "Functions")?.rows[0],
    ).toMatchObject({
      title: "run",
      detail: "Defined at line 3",
      paths: ["/home/u/app/src/a.ts"],
    });
    expect(result.sections.map((section) => section.title)).toContain(
      "Call graph",
    );
  });

  it("maps code_health biomarkers, contributors and refactorings without a per-finding path", () => {
    const result = report("code_health", {
      index_state: { queued: 0 },
      file_category: "code",
      file_role: "entrypoint",
      aggregate: {
        file_count: 1,
        function_count: 1,
        avg_score: 9,
        grade: "A",
        max_complexity: 8,
        avg_maintainability: 80,
        avg_maintainability_index: 84.9,
        avg_maintainability_signal: 9.4,
        avg_duplication_pct: 0.06,
        biomarker_count: 2,
        refactoring_count: 1,
      },
      files: [
        {
          path: "/home/u/app/src/a.ts",
          lang: "TypeScript",
          score: 9,
          grade: "A",
          complexity: 8,
          maintainability: 80,
          maintainability_index: 84.9,
          maintainability_signal: 9.4,
          max_complexity: 8,
          avg_maintainability: 80,
          function_count: 1,
          duplication_pct: 0.06,
          dry_violation: false,
          defect_score: 9,
          maintainability_score: 9.4,
          performance_score: 10,
          biomarker_count: 2,
          refactoring_count: 1,
          functions: [],
          findings: [
            {
              biomarker: "prior_defect",
              category: "history",
              dimension: "Defect",
              severity: "Medium",
              line: 1,
              detail: "2 prior defects in window_days=180",
              hot_path: true,
            },
          ],
          health_impact: [
            {
              biomarker: "error_handling",
              category: "robustness",
              dimension: "Maintainability",
              severity: "Low",
              line: 119,
              detail: "catch-all exception hides every error",
              deduction: 0.1,
              capped: false,
            },
          ],
          cache_hit: true,
          refactorings: [
            {
              kind: "SplitFile",
              target: "a.ts",
              line: 1,
              rationale: "File contains 15 detected functions",
              impact: 4.3,
              effort: "high",
            },
          ],
        },
      ],
      call_graph: [],
      coverage: {
        label: "lcov",
        line_pct: 72,
        branch_pct: 55,
        files_below_50: 3,
      },
      warm_cache: true,
    });

    const biomarkers = result.sections.find(
      (section) => section.title === "Biomarkers",
    );
    expect(biomarkers?.rows[0]).toMatchObject({
      title: "prior_defect",
      detail: "2 prior defects in window_days=180",
      severity: "Medium",
      paths: ["/home/u/app/src/a.ts"],
    });
    expect(biomarkers?.rows[0].tags).toContain("hot-path");

    const contributors = result.sections.find(
      (section) => section.title === "Top health impact contributors",
    );
    expect(contributors?.rows[0]).toMatchObject({
      title: "error_handling",
      severity: "Low",
    });

    const refactorings = result.sections.find(
      (section) => section.title === "Refactoring targets",
    );
    expect(refactorings?.rows[0]).toMatchObject({
      title: "SplitFile",
      detail: "File contains 15 detected functions",
    });
    expect(refactorings?.rows[0].tags).toContain("high effort");

    expect(result.facts).toContainEqual({ key: "Category", value: "code" });
    expect(result.facts).toContainEqual({ key: "Role", value: "entrypoint" });
    expect(result.facts).toContainEqual({
      key: "Coverage lines %",
      value: "72",
    });
    expect(result.indexState).toContainEqual({
      key: "warm cache",
      value: "hit",
    });
    expect(result.sections.map((section) => section.title)).not.toContain(
      "Call graph",
    );
  });

  it("reports duplication and agent authorship as real percentages", () => {
    const duplication = report("code_duplication", {
      aggregate: {
        file_count: 10,
        clone_pair_count: 2,
        duplication_pct: 0.075,
        duplication_percent: 7.5,
      },
      clones: [],
      dry_violations: [],
      test_smells: [],
    });
    expect(duplication.facts).toContainEqual({
      key: "Duplication %",
      value: "7.5",
    });

    const risk = report("git_risk", {
      commits_analyzed: 1000,
      agent_authored_pct: 0.02,
      hotspots: [],
      ownership: [],
      co_change: [],
      coupling: [],
      reviewers: [],
      findings: [],
      recent_commit_risks: [],
    });
    expect(risk.facts).toContainEqual({ key: "Agent authored %", value: "2" });
  });

  it("rejects unknown tools and malformed payloads", () => {
    expect(
      buildAnalysisReport("unknown", { tool: "unknown", summary: "x" }),
    ).toBeNull();
    expect(
      buildAnalysisReport("security_scan", {
        tool: "security_scan",
        summary: "x",
      }),
    ).toBeNull();
  });
});
