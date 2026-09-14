import { AGENTIC_CARD_LABELS } from "./agenticFamilies";
import { parseFamilyReport } from "./familyReport";
import {
  FamilyAgentIds,
  FamilyBody,
  FamilyCard,
  FamilyChildLink,
  FamilyRawArgs,
} from "./FamilyToolCard";
import {
  familyCardState,
  type FamilyCardTexts,
  type FamilyViewProps,
} from "./familyCardState";

const TEXTS: FamilyCardTexts = {
  ready: "Observed state",
  pending: "No observation returned yet.",
  failed: "The inspection failed. The agent state below is unknown, not idle.",
  malformed:
    "The inspection response could not be read. No agent state is confirmed here.",
  legacy:
    "This response carries no recognised inspection structure. It is shown verbatim and is not verified state.",
  note: "An observation is a point-in-time snapshot; it neither stops the agent nor proves its work is correct.",
};

export function InspectToolView({
  model,
  content,
  onOpenChildTrajectory,
}: FamilyViewProps & {
  onOpenChildTrajectory?: (childChatId: string) => void;
}) {
  const report = parseFamilyReport(content);
  const state = familyCardState(model, report);
  return (
    <FamilyCard
      label={AGENTIC_CARD_LABELS.inspect}
      state={state}
      texts={TEXTS}
      model={model}
      content={content}
    >
      <FamilyBody
        model={model}
        report={report}
        state={state}
        factsLabel="Observed facts"
        listLabel="Observed rows"
      />
      {model.childChatId !== null && state !== "pending" && (
        <FamilyChildLink
          childChatId={model.childChatId}
          onOpen={onOpenChildTrajectory}
        />
      )}
      <FamilyAgentIds ids={model.agentIds} />
      <FamilyRawArgs json={model.argsJson} />
    </FamilyCard>
  );
}
