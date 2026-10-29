import React from "react";
import { MessageCircle } from "lucide-react";
import classNames from "classnames";
import { Button, Text } from "../../../components/ui";
import { useAppSelector } from "../../../hooks";
import { buddyOperatorIdentity } from "../../../services/refact/buddyOperator";
import { useGetBuddyNativeDecisionsQuery } from "../../../services/refact/buddyDecisions";
import { useGetBuddyNativeApprovalsQuery } from "../../../services/refact/buddyApprovals";
import type { ProjectIdentity } from "../operatorTypes";
import type { EditorialFeedback } from "../../../services/refact/buddyEditorial";
import { BuddyRecentChats } from "../BuddyRecentChats";
import { BuddyPresence } from "../BuddyPresence";
import { BuddyNativeDecisions } from "../BuddyNativeDecisions";
import { BuddyNativeApprovals } from "../BuddyNativeApprovals";
import { BuddyNativeWork } from "../BuddyNativeWork";
import {
  BuddyHomeLatestReport,
  BuddyHomeSavedWork,
  BuddyHomeTalk,
  type BuddyHomeWorkMode,
} from "../BuddyHomeOverview";
import {
  BuddyErrorCallout,
  BuddySection,
  BuddySurface,
  buddyPossessive,
} from "../ui";
import { useBuddyTextPresence } from "../useBuddyTextPresence";
import type { BuddySceneSpeech } from "../buddySceneSpeech";
import type { BuddyControl, BuddySemanticState } from "../types";
import type { BuddyHomeSection as BuddyHomeSectionId } from "./shared";
import styles from "./BuddyHomeSection.module.css";

const EDITORIAL_CHOICES: { choice: EditorialFeedback; label: string }[] = [
  { choice: "useful", label: "Helpful" },
  { choice: "noisy", label: "Noisy" },
  { choice: "less_like_this", label: "Less like this" },
];

const BuddyHomeRequests: React.FC = () => {
  const identity = useAppSelector(buddyOperatorIdentity);
  const questions = useGetBuddyNativeDecisionsQuery({
    endpoint_identity: identity,
  });
  const approvals = useGetBuddyNativeApprovalsQuery({
    endpoint_identity: identity,
  });
  const bothFailed = Boolean(questions.error) && Boolean(approvals.error);
  return (
    <div className={styles.requests} data-testid="buddy-home-native">
      {bothFailed && (
        <BuddyErrorCallout
          thing="questions and approvals"
          error={questions.error}
          retrying={questions.isFetching || approvals.isFetching}
          onRetry={() => {
            void questions.refetch();
            void approvals.refetch();
          }}
          testId="buddy-home-requests-error"
        />
      )}
      <BuddyNativeDecisions hideError={bothFailed} />
      <BuddyNativeApprovals hideError={bothFailed} />
    </div>
  );
};

type EditorialProps = {
  canFeedback: boolean;
  speech: { text: string } | null | undefined;
  sendFeedback: (choice: EditorialFeedback) => Promise<void>;
  feedbackStatus: string | null;
};

const BuddyHomeEditorial: React.FC<{
  name: string;
  editorial: EditorialProps;
}> = ({ name, editorial }) =>
  editorial.canFeedback && editorial.speech ? (
    <BuddySection
      title={`${buddyPossessive(name)} note`}
      icon={MessageCircle}
      aria-label="Buddy editorial message"
    >
      <Text as="p" size="2" className={styles.editorialText}>
        {editorial.speech.text}
      </Text>
      <div className={styles.editorialActions}>
        {EDITORIAL_CHOICES.map(({ choice, label }) => (
          <Button
            key={choice}
            size="sm"
            variant="ghost"
            onClick={() => void editorial.sendFeedback(choice)}
          >
            {label}
          </Button>
        ))}
      </div>
      {editorial.feedbackStatus && (
        <Text
          size="1"
          color="gray"
          role="status"
          className={styles.editorialText}
        >
          {editorial.feedbackStatus}
        </Text>
      )}
    </BuddySection>
  ) : null;

export const BuddyHomeSection: React.FC<{
  name: string;
  degraded: boolean;
  world: React.ReactNode;
  state: BuddySemanticState;
  headline: string;
  vibe?: string | null;
  heroSpeech: BuddySceneSpeech | null;
  onSelectSection: (section: BuddyHomeSectionId) => void;
  onSpeechControl: (control: BuddyControl) => Promise<void>;
  editorial: EditorialProps;
  homeWorkMode: BuddyHomeWorkMode;
  project: ProjectIdentity | null;
  onOpenWork: (workId: string, projectId: string) => void;
  onShowReports: () => void;
}> = ({
  name,
  degraded,
  world,
  state,
  headline,
  vibe,
  heroSpeech,
  onSelectSection: selectSection,
  onSpeechControl: handleSpeechControl,
  editorial,
  homeWorkMode,
  project,
  onOpenWork: handleOpenWork,
  onShowReports: handleShowReports,
}) => {
  const [textOnly] = useBuddyTextPresence();
  const showPortrait = !degraded && !textOnly;
  return (
    <div
      data-section-panel="home"
      tabIndex={-1}
      className={styles.section}
      data-testid="buddy-home-overview"
    >
      <BuddySurface
        as="section"
        level="section"
        className={classNames(
          styles.identityCard,
          !showPortrait && styles.identityTextOnly,
        )}
        aria-label={`${name} summary`}
      >
        {showPortrait && (
          <div
            className={styles.portrait}
            id="buddy-home-hero"
            data-testid="buddy-home-hero"
          >
            {world}
          </div>
        )}
        <div className={styles.identityBody}>
          <BuddyPresence
            state={state}
            headline={headline}
            vibe={vibe}
            speech={heroSpeech}
            sceneMounted={!degraded}
            sceneToggleLabel="Open Play"
            onToggleScene={() => selectSection("play")}
            onControl={handleSpeechControl}
            primaryAction={<BuddyHomeTalk name={name} />}
          />
        </div>
      </BuddySurface>

      <BuddyHomeEditorial name={name} editorial={editorial} />

      <div className={styles.homeColumns}>
        <div className={styles.homeMain}>
          <BuddyHomeRequests />
          <BuddyHomeSavedWork
            mode={homeWorkMode}
            project={project}
            onOpen={handleOpenWork}
            onShowReports={handleShowReports}
          />
        </div>
        <div className={styles.homeAside}>
          <BuddyHomeLatestReport
            mode={homeWorkMode}
            project={project}
            onOpen={handleOpenWork}
            onShowReports={handleShowReports}
          />
          <BuddyNativeWork />
          <BuddyRecentChats
            compact
            maxItems={3}
            initialFilter="chat"
            title="Recent conversations"
            onViewAll={() => selectSection("activity")}
          />
        </div>
      </div>
    </div>
  );
};
