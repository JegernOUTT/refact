import { AGENTIC_CARD_LABELS } from "./agenticFamilies";
import { parseFamilyReport } from "./familyReport";
import { FamilyBody, FamilyCard, FamilyRawArgs } from "./FamilyToolCard";
import {
  familyCardState,
  type FamilyCardTexts,
  type FamilyViewProps,
} from "./familyCardState";

const TEXTS: FamilyCardTexts = {
  ready: "Control request accepted",
  pending: "No control outcome confirmed yet.",
  failed:
    "The control request failed. The target agent may still be running unchanged.",
  malformed:
    "The control request or its response could not be read. Do not assume the agent changed state.",
  legacy:
    "This response carries no recognised control structure. It is shown verbatim and is not a verified outcome.",
  note: "An accepted control request is not proof that the agent already stopped, resumed, or merged.",
};

export function ControlToolView({ model, content }: FamilyViewProps) {
  const report = parseFamilyReport(content);
  const state = familyCardState(model, report);
  return (
    <FamilyCard
      label={AGENTIC_CARD_LABELS.control}
      state={state}
      texts={TEXTS}
      model={model}
      content={content}
    >
      <FamilyBody
        model={model}
        report={report}
        state={state}
        factsLabel="Control facts"
        listLabel="Affected agents"
      />
      <FamilyRawArgs json={model.argsJson} />
    </FamilyCard>
  );
}
