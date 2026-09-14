import { AGENTIC_CARD_LABELS } from "./agenticFamilies";
import { parseFamilyReport } from "./familyReport";
import { FamilyBody, FamilyCard, FamilyRawArgs } from "./FamilyToolCard";
import {
  familyCardState,
  type FamilyCardTexts,
  type FamilyViewProps,
} from "./familyCardState";

const TEXTS: FamilyCardTexts = {
  ready: "Document receipt",
  pending: "No document outcome confirmed yet.",
  failed:
    "The document request failed. No document version was written by this call.",
  malformed:
    "The document request or its response could not be read. Do not assume the document changed.",
  legacy:
    "This response carries no recognised document structure. It is shown verbatim and is not a verified version.",
  note: "A document receipt records the stored text, not agreement with it or work performed from it.",
};

export function DocToolView({ model, content }: FamilyViewProps) {
  const report = parseFamilyReport(content);
  const state = familyCardState(model, report);
  return (
    <FamilyCard
      label={AGENTIC_CARD_LABELS.doc}
      state={state}
      texts={TEXTS}
      model={model}
      content={content}
    >
      <FamilyBody
        model={model}
        report={report}
        state={state}
        factsLabel="Document facts"
        listLabel="Document entries"
      />
      <FamilyRawArgs json={model.argsJson} />
    </FamilyCard>
  );
}
