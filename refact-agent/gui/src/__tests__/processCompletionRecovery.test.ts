import { describe, it, expect } from "vitest";
import { planProcessCompletionRecovery } from "../app/middleware";
import type { ChatMessage } from "../services/refact";

function processEvent(processId: string): ChatMessage {
  return {
    message_id: `m-${processId}`,
    role: "event",
    content: `Background process '${processId}' exited (exit 0)`,
    extra: {
      event: {
        subkind: "process_completed",
        source: "exec.registry",
        payload: {
          process_id: processId,
          status: "exited",
          exit_code: 0,
          short_description: processId,
          mode: "background",
        },
      },
    },
  } as unknown as ChatMessage;
}

function cronEvent(): ChatMessage {
  return {
    message_id: "m-cron",
    role: "event",
    content: "cron fired",
    extra: {
      event: { subkind: "cron_fire", source: "scheduler.cron", payload: {} },
    },
  } as unknown as ChatMessage;
}

describe("planProcessCompletionRecovery", () => {
  it("does not recover before a prior snapshot baseline exists", () => {
    expect(
      planProcessCompletionRecovery("A", "B", false, [], [processEvent("p1")]),
    ).toEqual([]);
  });

  it("does not recover for the currently focused thread", () => {
    expect(
      planProcessCompletionRecovery("A", "A", true, [], [processEvent("p1")]),
    ).toEqual([]);
  });

  it("recovers a newly-arrived completion for a non-focused thread", () => {
    const recovered = planProcessCompletionRecovery(
      "A",
      "B",
      true,
      [],
      [processEvent("p1")],
    );
    expect(recovered).toHaveLength(1);
    expect(recovered[0].processId).toBe("p1");
    expect(recovered[0].shortDescription).toBe("p1");
  });

  it("recovers for a null current thread", () => {
    expect(
      planProcessCompletionRecovery("A", null, true, [], [processEvent("p1")]),
    ).toHaveLength(1);
  });

  it("ignores completions already present in the previous messages", () => {
    const recovered = planProcessCompletionRecovery(
      "A",
      "B",
      true,
      [processEvent("p1")],
      [processEvent("p1"), processEvent("p2")],
    );
    expect(recovered.map((c) => c.processId)).toEqual(["p2"]);
  });

  it("dedups repeated process ids in the snapshot", () => {
    const recovered = planProcessCompletionRecovery(
      "A",
      "B",
      true,
      [],
      [processEvent("p1"), processEvent("p1")],
    );
    expect(recovered).toHaveLength(1);
  });

  it("ignores non-process event messages", () => {
    const recovered = planProcessCompletionRecovery(
      "A",
      "B",
      true,
      [],
      [cronEvent(), processEvent("p1")],
    );
    expect(recovered.map((c) => c.processId)).toEqual(["p1"]);
  });
});
