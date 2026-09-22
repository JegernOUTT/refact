import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, test } from "vitest";
import {
  InvalidOperatorContract,
  parseOperatorContractFixture,
  parseRunOutcome,
  parseWorkItem,
  parseArtifactRevision,
  parseActionSpec,
  parseActionExecution,
  parseDecisionProjection,
  parseMandate,
  parseDelivery,
  parseReadState,
  parseImprovementCandidate,
  UnsupportedOperatorSchemaVersion,
} from "./operatorTypes";

const fixture: unknown = JSON.parse(
  readFileSync(
    join(
      process.cwd(),
      "../engine/crates/refact-buddy-core/tests/fixtures/operator_contract_v1.json",
    ),
    "utf8",
  ),
);
const triagedFixture: Record<string, unknown> = JSON.parse(
  readFileSync(
    join(
      process.cwd(),
      "../engine/crates/refact-buddy-core/tests/fixtures/work_item_v1_triaged.json",
    ),
    "utf8",
  ),
);

const clone = <T>(value: T): T => JSON.parse(JSON.stringify(value)) as T;

describe("operator contract v1", () => {
  const entityParsers = [
    ["work_item", parseWorkItem],
    ["run_outcome", parseRunOutcome],
    ["artifact_revision", parseArtifactRevision],
    ["action_spec", parseActionSpec],
    ["action_execution", parseActionExecution],
    ["decision_projection", parseDecisionProjection],
    ["mandate", parseMandate],
    ["delivery", parseDelivery],
    ["read_state", parseReadState],
    ["improvement_candidate", parseImprovementCandidate],
  ] as const;

  test.each(entityParsers)(
    "%s rejects unknown fields without requiring the aggregate parser",
    (key, parse) => {
      const value = parseOperatorContractFixture(fixture)[key];
      expect(() => parse({ ...value, future_authority: true })).toThrow(
        InvalidOperatorContract,
      );
      const project =
        "project" in value ? value.project : value.decision.project;
      const invalid =
        "project" in value
          ? { ...value, project: { ...project, future_scope: true } }
          : {
              ...value,
              decision: {
                ...value.decision,
                project: { ...project, future_scope: true },
              },
            };
      expect(() => parse(invalid)).toThrow(InvalidOperatorContract);
    },
  );

  test("round trips the shared golden fixture", () => {
    const parsed = parseOperatorContractFixture(fixture);
    const serialized = JSON.parse(JSON.stringify(parsed)) as {
      work_item: {
        severity?: null;
        problem_summary?: null;
        scope?: null;
      };
    };
    delete serialized.work_item.severity;
    delete serialized.work_item.problem_summary;
    delete serialized.work_item.scope;
    expect(serialized).toEqual(fixture);
    expect(parsed.action_execution.status).toBe("reconciling");
    expect(parsed.decision_projection.decision.native.source_kind).toBe(
      "question",
    );
  });

  test("rejects unknown schemas", () => {
    const invalid = clone(fixture) as { work_item: { schema_version: number } };
    invalid.work_item.schema_version = 2;
    expect(() => parseOperatorContractFixture(invalid)).toThrow(
      UnsupportedOperatorSchemaVersion,
    );
  });

  test("rejects empty identities", () => {
    const invalid = clone(fixture) as { work_item: { id: string } };
    invalid.work_item.id = " ";
    expect(() => parseOperatorContractFixture(invalid)).toThrow(
      InvalidOperatorContract,
    );
  });

  test("rejects empty nested identities and unknown fields", () => {
    const invalid = clone(fixture) as {
      work_item: { project: { project_id: string }; unknown?: boolean };
    };
    invalid.work_item.project.project_id = " ";
    expect(() => parseOperatorContractFixture(invalid)).toThrow(
      InvalidOperatorContract,
    );

    const unknown = clone(fixture) as { work_item: { unknown?: boolean } };
    unknown.work_item.unknown = true;
    expect(() => parseOperatorContractFixture(unknown)).toThrow(
      InvalidOperatorContract,
    );
  });

  test("accepts missing optional fields and bounds u32 revisions", () => {
    const missing = clone(fixture) as {
      work_item: { current_artifact_id?: string | null };
    };
    delete missing.work_item.current_artifact_id;
    expect(
      parseOperatorContractFixture(missing).work_item.current_artifact_id,
    ).toBeNull();

    const overflow = clone(fixture) as {
      artifact_revision: { revision: number };
    };
    overflow.artifact_revision.revision = 0x1_0000_0000;
    expect(() => parseOperatorContractFixture(overflow)).toThrow(
      InvalidOperatorContract,
    );
  });

  test("accepts named triage fields but still rejects unrelated fields", () => {
    expect(parseWorkItem(triagedFixture)).toMatchObject({
      severity: "high",
      problem_summary: "Reconnect provider",
      scope: "providers/primary",
    });
    expect(() =>
      parseWorkItem({ ...triagedFixture, unexpected_field: true }),
    ).toThrow(InvalidOperatorContract);
  });

  test("round trips the inconclusive run outcome", () => {
    expect(parseOperatorContractFixture(fixture).run_outcome.status).toBe(
      "inconclusive",
    );
  });

  test("rejects malformed enum values", () => {
    const invalid = clone(fixture) as { work_item: { state: string } };
    invalid.work_item.state = "invalid_state";
    expect(() => parseWorkItem(invalid.work_item)).toThrow(
      InvalidOperatorContract,
    );
  });

  test("rejects unsafe worker epochs and budget values", () => {
    const invalidEpoch = clone(fixture) as {
      run_outcome: { worker: { epoch: number } };
    };
    invalidEpoch.run_outcome.worker.epoch = Number.MAX_SAFE_INTEGER + 1;
    expect(() => parseRunOutcome(invalidEpoch.run_outcome)).toThrow(
      InvalidOperatorContract,
    );

    const invalidBudget = clone(fixture) as {
      mandate: { budget_limit: { max_tokens: number } };
    };
    invalidBudget.mandate.budget_limit.max_tokens = Number.MAX_SAFE_INTEGER + 1;
    expect(() => parseOperatorContractFixture(invalidBudget)).toThrow(
      InvalidOperatorContract,
    );
  });
});
