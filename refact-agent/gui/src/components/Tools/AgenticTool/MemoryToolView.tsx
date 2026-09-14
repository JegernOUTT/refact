import { AGENTIC_CARD_LABELS } from "./agenticFamilies";
import { parseFamilyReport } from "./familyReport";
import { FamilyBody, FamilyCard, FamilyRawArgs } from "./FamilyToolCard";
import {
  familyCardState,
  type FamilyCardTexts,
  type FamilyViewProps,
} from "./familyCardState";

const TEXTS: FamilyCardTexts = {
  ready: "Memory receipt",
  pending: "No memory outcome confirmed yet.",
  failed: "The memory request failed. No memory was written or returned.",
  malformed:
    "The memory request or its response could not be read. Do not assume a memory was stored.",
  legacy:
    "This response carries no recognised memory structure. It is shown verbatim and is not a verified memory.",
  note: "Stored memories are self-reported notes from agents, not verified facts about the repository.",
};

export function MemoryToolView({ model, content }: FamilyViewProps) {
  const report = parseFamilyReport(content);
  const state = familyCardState(model, report);
  return (
    <FamilyCard
      label={AGENTIC_CARD_LABELS.memory}
      state={state}
      texts={TEXTS}
      model={model}
      content={content}
    >
      <FamilyBody
        model={model}
        report={report}
        state={state}
        factsLabel="Memory facts"
        listLabel="Memory entries"
      />
      <FamilyRawArgs json={model.argsJson} />
    </FamilyCard>
  );
}
