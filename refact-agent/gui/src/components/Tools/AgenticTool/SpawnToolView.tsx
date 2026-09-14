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
  ready: "Launch requested",
  pending: "No launch outcome confirmed yet.",
  failed: "The launch request failed. No child agent is proven to be running.",
  malformed:
    "The launch request or its response could not be read. Do not assume a child agent started.",
  legacy:
    "This response carries no recognised launch structure. It is shown verbatim and is not a verified receipt.",
  note: "A launch receipt records a request, not completed child work or merged changes.",
};

export function SpawnToolView({
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
      label={AGENTIC_CARD_LABELS.spawn}
      state={state}
      texts={TEXTS}
      model={model}
      content={content}
    >
      <FamilyBody
        model={model}
        report={report}
        state={state}
        factsLabel="Launch facts"
        listLabel="Launched agents"
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
