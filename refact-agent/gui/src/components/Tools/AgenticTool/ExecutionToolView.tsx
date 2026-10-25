import { parseFamilyReport } from "./familyReport";
import { FamilyBody, FamilyCard, FamilyRawArgs } from "./FamilyToolCard";
import { familyCardState, type FamilyViewProps } from "./familyCardState";

export function ExecutionToolView({ model, content }: FamilyViewProps) {
  const report = parseFamilyReport(content);
  const state = familyCardState(model, report);
  const waiting = model.toolName === "wait";
  const outcome = report.fields.find((field) => field.label === "outcome")
    ?.value;
  const waitLabel =
    outcome === "completed"
      ? "Wait completed"
      : outcome === "timed_out"
        ? "Wait timed out"
        : outcome === "interrupted"
          ? "Wait interrupted"
          : "Wait returned";
  const note = waiting
    ? "A completed wait means the wait returned, not that a worker succeeded. A timeout leaves the target unchanged; parked task agents are still nonterminal."
    : model.toolName === "schedule"
      ? "This is a schedule receipt, not proof that scheduled work ran or succeeded."
      : "Pause holds execution; resume releases a hold. Neither operation recovers a stopped child or proves execution has finished.";
  return (
    <FamilyCard
      label="Execution receipt"
      state={state}
      texts={{
        ready: waiting ? waitLabel : "Execution request recorded",
        pending: "Awaiting the execution receipt.",
        failed:
          "The execution request failed. Do not assume the target changed state.",
        malformed:
          "The execution receipt could not be read. No outcome is confirmed.",
        legacy:
          "Unrecognised execution response; shown without inferring an outcome.",
        note,
      }}
      model={model}
      content={content}
    >
      <FamilyBody
        model={model}
        report={report}
        state={state}
        factsLabel="Execution facts"
        listLabel="Observed targets"
      />
      <FamilyRawArgs json={model.argsJson} />
    </FamilyCard>
  );
}
