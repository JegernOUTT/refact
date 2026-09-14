import { AGENTIC_CARD_LABELS } from "./agenticFamilies";
import { parseFamilyReport } from "./familyReport";
import {
  FamilyAgentRows,
  FamilyCard,
  FamilyDetails,
  FamilyRawArgs,
  FamilyResponseText,
  MAX_DETAIL_ROWS,
} from "./FamilyToolCard";
import {
  familyCardState,
  type FamilyCardTexts,
  type FamilyViewProps,
} from "./familyCardState";
import styles from "./AgenticTool.module.css";

const TEXTS: FamilyCardTexts = {
  ready: "Roster snapshot",
  pending: "No roster snapshot returned yet.",
  failed: "The roster request failed. Treat the agent list as unknown.",
  malformed:
    "The roster response could not be read. No agent state is confirmed here.",
  legacy:
    "This response carries no recognised roster structure. It is shown verbatim and is not a verified snapshot.",
  note: "A roster is a historical snapshot. Observing an agent does not grant authority over it.",
};

export function OverviewToolView({ model, content }: FamilyViewProps) {
  const report = parseFamilyReport(content);
  const state = familyCardState(model, report);
  const visible =
    state !== "pending" && state !== "failed" && state !== "malformed";
  const details = model.details.slice(0, MAX_DETAIL_ROWS);
  return (
    <FamilyCard
      label={AGENTIC_CARD_LABELS.overview}
      state={state}
      texts={TEXTS}
      model={model}
      content={content}
    >
      {visible && report.title !== null && (
        <h4 className={styles.sectionLabel} data-testid="agentic-tool-title">
          {report.title}
        </h4>
      )}
      {visible && details.length > 0 && (
        <FamilyDetails
          label="Roster facts"
          rows={details}
          total={model.details.length}
        />
      )}
      {visible && model.agents.length > 0 && (
        <FamilyAgentRows agents={model.agents} />
      )}
      {visible && model.agents.length === 0 && (
        <p className={styles.note}>No agents were listed in this snapshot.</p>
      )}
      {visible && <FamilyResponseText body={model.body} />}
      <FamilyRawArgs json={model.argsJson} />
    </FamilyCard>
  );
}
