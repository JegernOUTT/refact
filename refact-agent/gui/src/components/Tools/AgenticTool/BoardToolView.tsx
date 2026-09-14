import { AGENTIC_CARD_LABELS } from "./agenticFamilies";
import { parseFamilyReport } from "./familyReport";
import { FamilyBody, FamilyCard, FamilyRawArgs } from "./FamilyToolCard";
import {
  familyCardState,
  type FamilyCardTexts,
  type FamilyViewProps,
} from "./familyCardState";

const TEXTS: FamilyCardTexts = {
  ready: "Board receipt",
  pending: "No board outcome confirmed yet.",
  failed: "The board request failed. The board was not changed by this call.",
  malformed:
    "The board request or its response could not be read. Do not assume a card changed.",
  legacy:
    "This response carries no recognised board structure. It is shown verbatim and is not a verified board state.",
  note: "A board receipt records bookkeeping. Moving a card to done does not verify the work behind it.",
};

export function BoardToolView({ model, content }: FamilyViewProps) {
  const report = parseFamilyReport(content);
  const state = familyCardState(model, report);
  return (
    <FamilyCard
      label={AGENTIC_CARD_LABELS.task}
      state={state}
      texts={TEXTS}
      model={model}
      content={content}
    >
      <FamilyBody
        model={model}
        report={report}
        state={state}
        factsLabel="Board facts"
        listLabel="Board cards"
      />
      <FamilyRawArgs json={model.argsJson} />
    </FamilyCard>
  );
}
