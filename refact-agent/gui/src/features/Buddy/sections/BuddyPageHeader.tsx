import React from "react";
import { AlertTriangle, ArrowLeft } from "lucide-react";
import classNames from "classnames";
import { Badge, Button, Icon, Text } from "../../../components/ui";
import type { OperatorUnavailableReason } from "../../../services/refact/buddyOperator";
import {
  BUDDY_OPERATOR_STATUS,
  BUDDY_TERMS,
  BuddyDisclosure,
  BuddySurface,
  buddyName,
  buddyPossessive,
} from "../ui";
import type { BuddyOperatorFailure, BuddyOperatorMode } from "./shared";
import styles from "./BuddyPageHeader.module.css";

function failureTitle(
  name: string,
  degraded: BuddyOperatorFailure,
  reason: OperatorUnavailableReason | undefined,
): string {
  if (degraded === "error")
    return `${buddyPossessive(name)} report service answered with an error`;
  if (!reason)
    return `${buddyPossessive(name)} report service isn't responding`;
  switch (reason.kind) {
    case "already_owned":
      return "Another Refact process has the reports open";
    case "project_mismatch":
      return "These reports belong to a different project";
    case "not_attempted":
      return `${buddyName(name)} hasn't opened the reports yet`;
    default:
      return `${buddyName(name)} couldn't open the reports`;
  }
}

function retryScheduleCopy(
  name: string,
  retryInSeconds: number | undefined,
): string | null {
  if (retryInSeconds === undefined) return null;
  if (retryInSeconds < 60)
    return `${buddyName(name)} is already trying again in under a minute.`;
  const minutes = Math.ceil(retryInSeconds / 60);
  return `${buddyName(
    name,
  )} is already trying again in about ${minutes} minute${
    minutes === 1 ? "" : "s"
  }.`;
}

const RETRY_OUTCOME_COPY: Record<
  "recovered" | "unavailable" | "failed",
  string
> = {
  recovered: "Reports are available again.",
  unavailable: "Reports are still unavailable. Retrying in the background.",
  failed: "Couldn't retry. Try again later.",
};

export const BuddyStatusPill: React.FC<{
  operatorMode: BuddyOperatorMode;
  degraded: BuddyOperatorFailure | null;
}> = ({ operatorMode, degraded }) => {
  const entry = BUDDY_OPERATOR_STATUS[operatorMode];
  return (
    <Badge
      size="xs"
      variant={degraded ? "outline" : "soft"}
      tone={entry.tone}
      className={classNames(
        styles.statusPill,
        degraded && styles.statusPillAlert,
      )}
      data-testid="buddy-operator-status"
      data-state={operatorMode}
    >
      {degraded && <Icon icon={AlertTriangle} size="sm" tone="danger" />}
      {entry.label}
    </Badge>
  );
};

export const BuddyPageHeader: React.FC<{
  onBack: () => void;
  identity?: {
    name: string;
    statusPill: React.ReactNode;
    projectName?: string | null;
    projectRoot?: string;
  };
  trailing?: React.ReactNode;
}> = ({ onBack, identity, trailing }) => (
  <div className={styles.topBar}>
    <Button variant="ghost" size="sm" leftIcon={ArrowLeft} onClick={onBack}>
      Back
    </Button>
    {identity && (
      <div className={styles.topIdentity}>
        <Text as="h1" size="4" weight="bold" className={styles.topTitle}>
          {identity.name}
        </Text>
        <span className={styles.topMeta}>
          {identity.projectName && (
            <>
              <Text
                size="1"
                color="gray"
                className={styles.topProject}
                title={identity.projectRoot}
              >
                {identity.projectName}
              </Text>
              <span className={styles.metaSeparator} aria-hidden="true">
                ·
              </span>
            </>
          )}
          {identity.statusPill}
        </span>
      </div>
    )}
    {trailing}
  </div>
);

export const BuddyDegradedBanner: React.FC<{
  name: string;
  degraded: BuddyOperatorFailure;
  unavailableReason: OperatorUnavailableReason | undefined;
  isRetrying: boolean;
  retryOutcome: "recovered" | "unavailable" | "failed" | null;
  onRetry: () => Promise<void>;
}> = ({
  name,
  degraded,
  unavailableReason,
  isRetrying,
  retryOutcome,
  onRetry,
}) => {
  const schedule =
    degraded === "unavailable"
      ? retryScheduleCopy(name, unavailableReason?.retry_in_seconds)
      : null;
  const technical =
    degraded === "unavailable" && unavailableReason
      ? [unavailableReason.kind, unavailableReason.detail]
          .filter(Boolean)
          .join(": ")
      : null;
  return (
    <BuddySurface
      as="section"
      level="section"
      className={styles.degradedCard}
      role="alert"
      testId="buddy-operator-degraded"
    >
      <div className={styles.degradedHeader}>
        <Icon icon={AlertTriangle} size="md" tone="danger" />
        <Text as="h2" size="3" weight="bold" className={styles.degradedTitle}>
          {failureTitle(name, degraded, unavailableReason)}
        </Text>
        <Button
          size="sm"
          variant="ghost"
          onClick={() => void onRetry()}
          disabled={isRetrying}
          loading={isRetrying}
        >
          {unavailableReason ? "Retry connection" : "Check connection"}
        </Button>
      </div>
      <Text size="2" color="gray" className={styles.degradedText}>
        This is a connection problem, not an empty list. Chats and files are not
        affected.
        {schedule ? ` ${schedule}` : ""}
      </Text>
      {retryOutcome && (
        <Text size="2" role="status" className={styles.degradedText}>
          {RETRY_OUTCOME_COPY[retryOutcome]}
        </Text>
      )}
      {technical && (
        <BuddyDisclosure label={BUDDY_TERMS.technicalDetails} lazy>
          <Text as="p" size="1" color="gray" className={styles.degradedText}>
            {technical}
          </Text>
        </BuddyDisclosure>
      )}
    </BuddySurface>
  );
};
