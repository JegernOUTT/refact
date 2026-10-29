import React, { useCallback, useState } from "react";
import { FileText } from "lucide-react";
import { Button } from "../../../components/ui";
import {
  useDeleteDraftMutation,
  useGetDraftQuery,
} from "../../../services/refact/buddy";
import type { BuddyDraft, DraftKind } from "../types";
import {
  BuddyErrorCallout,
  BuddyLoading,
  BuddySection,
  humanizeBuddyIdentifier,
} from "../ui";
import styles from "./BuddyReportsSection.module.css";

const DRAFT_KIND_LABELS: Record<DraftKind, string> = {
  skill: "Skill",
  command: "Command",
  delegate: "Delegate",
  mode: "Mode",
  agents_md: "AGENTS.md",
  defaults_model: "Default models",
  hook: "Hook",
  pulse_report: "Pulse report",
};

const REVIEWABLE_DRAFT_KINDS: DraftKind[] = ["agents_md", "pulse_report"];

type ClipboardWriter = {
  clipboard?: {
    writeText?: (text: string) => Promise<void>;
  };
};

function draftKindLabel(draft: BuddyDraft): string {
  return DRAFT_KIND_LABELS[draft.kind];
}

export const BuddyHomeDraftReview: React.FC<{ draftId: string }> = ({
  draftId,
}) => {
  const { data: draft, isLoading, isError } = useGetDraftQuery(draftId);
  const [deleteDraft, { isLoading: isDeleting }] = useDeleteDraftMutation();
  const [copied, setCopied] = useState(false);
  const [copyError, setCopyError] = useState<string | null>(null);
  const [dismissError, setDismissError] = useState(false);

  const handleCopy = useCallback(async () => {
    if (!draft) return;
    setCopyError(null);
    try {
      const clipboard = (navigator as ClipboardWriter).clipboard;
      if (!clipboard?.writeText) {
        throw new Error("Clipboard API unavailable");
      }
      await clipboard.writeText(draft.yaml_or_json);
      setCopied(true);
    } catch {
      setCopied(false);
      setCopyError(
        "Couldn't copy. Select the draft text and copy it manually.",
      );
    }
  }, [draft]);

  const handleDismiss = useCallback(async () => {
    setDismissError(false);
    try {
      await deleteDraft(draftId).unwrap();
    } catch {
      setDismissError(true);
    }
  }, [deleteDraft, draftId]);

  if (isLoading) {
    return (
      <BuddySection title="Draft review" icon={FileText}>
        <BuddyLoading variant="cards" count={1} label="Loading draft" />
      </BuddySection>
    );
  }

  if (isError || !draft) {
    return (
      <BuddySection title="Draft review" icon={FileText}>
        <BuddyErrorCallout
          thing="this draft"
          title="Couldn't load this draft. It may have expired."
        />
      </BuddySection>
    );
  }

  const reviewable = REVIEWABLE_DRAFT_KINDS.includes(draft.kind);
  const draftRef = humanizeBuddyIdentifier(draft.id);

  return (
    <BuddySection
      title={draft.title}
      icon={FileText}
      description={draft.explanation || undefined}
    >
      <span className={styles.draftMeta} title={draftRef.full}>
        {draftKindLabel(draft)} draft · {draftRef.label}
      </span>
      {!reviewable && (
        <p className={styles.draftText}>
          This draft opens in its own editor from the suggestion.
        </p>
      )}
      <pre className={styles.draftContent}>{draft.yaml_or_json}</pre>
      <div className={styles.draftActions}>
        <Button
          type="button"
          size="sm"
          variant="primary"
          onClick={() => void handleCopy()}
        >
          {copied ? "Copied" : "Copy content"}
        </Button>
        <Button
          type="button"
          size="sm"
          variant="ghost"
          disabled={isDeleting}
          onClick={() => void handleDismiss()}
        >
          Dismiss draft
        </Button>
      </div>
      {copyError && <p className={styles.draftText}>{copyError}</p>}
      {dismissError && (
        <p role="alert" className={styles.draftText}>
          Couldn&apos;t dismiss the draft. Try again.
        </p>
      )}
    </BuddySection>
  );
};
