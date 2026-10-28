import { AGENTIC_CARD_LABELS, type AgenticFamily } from "./agenticFamilies";
import { parseFamilyReport } from "./familyReport";
import {
  FamilyAgentRows,
  FamilyBody,
  FamilyCard,
  FamilyDetails,
  FamilyResponseText,
  MAX_DETAIL_ROWS,
} from "./FamilyToolCard";
import { familyCardState, type FamilyViewProps } from "./familyCardState";

const LIST_LABELS: Record<AgenticFamily, [string, string]> = {
  spawn: ["Launch facts", "Launched agents"],
  message: ["Message facts", "Recipients"],
  overview: ["Roster facts", "Agents"],
  progress: ["Progress facts", "Entries"],
  inspect: ["Observed facts", "Observed rows"],
  control: ["Control facts", "Affected agents"],
  goal: ["Goal facts", "Entries"],
  plan: ["Plan facts", "Entries"],
  task: ["Board facts", "Board cards"],
  doc: ["Document facts", "Documents"],
  memory: ["Memory facts", "Memories"],
};

export function FamilyReportView({
  model,
  content,
  label,
}: FamilyViewProps & { label?: string }) {
  const report = parseFamilyReport(content);
  const state = familyCardState(model, report);
  const [factsLabel, listLabel] = LIST_LABELS[model.family];
  const cardLabel = label ?? AGENTIC_CARD_LABELS[model.family];
  if (model.family === "overview") {
    const details = model.details.slice(0, MAX_DETAIL_ROWS);
    return (
      <FamilyCard label={cardLabel} state={state} content={content}>
        {details.length > 0 && (
          <FamilyDetails
            label={factsLabel}
            rows={details}
            total={model.details.length}
          />
        )}
        {model.agents.length > 0 && <FamilyAgentRows agents={model.agents} />}
        <FamilyResponseText body={model.body} />
      </FamilyCard>
    );
  }
  return (
    <FamilyCard label={cardLabel} state={state} content={content}>
      <FamilyBody
        model={model}
        report={report}
        state={state}
        factsLabel={factsLabel}
        listLabel={listLabel}
      />
    </FamilyCard>
  );
}
