import React from "react";
import { Text } from "@radix-ui/themes";
import classNames from "classnames";
import { ShieldAlert, ShieldCheck } from "lucide-react";

import { Badge, Button, Dialog, Icon } from "../../components/ui";
import { useAppSelector } from "../../hooks";
import {
  selectModelById,
  selectPrivacyFilesById,
} from "../Chat/Thread/selectors";
import {
  blockedPrivacyFilesFromInspection,
  privacyDestinationForModel,
  useClearPrivacyRecordsMutation,
  useGetPrivacyPolicyQuery,
  useInspectPrivacyQuery,
  type PrivacyFileRecord,
} from "../../services/refact/privacy";
import { DestinationInspector } from "./DestinationInspector";
import styles from "./PrivacyChat.module.css";

type ChatShieldProps = {
  threadId: string;
};

type PendingClear = {
  paths: string[];
  zones: string[];
};

function isClearableRecord(record: PrivacyFileRecord): boolean {
  return record.zone !== "normal" && record.zone !== "blocked";
}

function unique(values: string[]): string[] {
  return Array.from(new Set(values));
}

function clearConfirmationText(pending: PendingClear): string {
  const label =
    pending.paths.length === 1
      ? "Remove this privacy label from the chat?"
      : "Remove these privacy labels from the chat?";
  const zones = pending.zones.map((zone) => `"${zone}"`).join(", ");
  const noun = pending.zones.length === 1 ? "zone" : "zones";
  return `${label} Data already in this chat may then be sent to destinations outside ${noun} ${zones}.`;
}

function clearErrorText(error: unknown): string {
  if (error && typeof error === "object" && "status" in error) {
    const status = (error as { status: unknown }).status;
    if (status === 401 || status === 403) {
      return "You are not allowed to clear privacy labels for this chat.";
    }
    if (status === 404) return "This chat is unknown to the engine.";
  }
  return "The privacy labels could not be cleared.";
}

export const ChatShield: React.FC<ChatShieldProps> = ({ threadId }) => {
  const [inspectorOpen, setInspectorOpen] = React.useState(false);
  const [labelsOpen, setLabelsOpen] = React.useState(false);
  const [pendingClear, setPendingClear] = React.useState<PendingClear | null>(
    null,
  );
  const [clearError, setClearError] = React.useState<string | null>(null);
  const [saveWarning, setSaveWarning] = React.useState(false);
  const [clearRecords, clearState] = useClearPrivacyRecordsMutation();
  const model = useAppSelector((state) => selectModelById(state, threadId));
  const files = useAppSelector((state) =>
    selectPrivacyFilesById(state, threadId),
  );
  const policy = useGetPrivacyPolicyQuery(undefined);
  const destination = privacyDestinationForModel(model);
  const inspection = useInspectPrivacyQuery(
    { chat_id: threadId, destination, records: files },
    { skip: !model },
  );
  const inspectionMatchesDestination =
    inspection.currentData?.destination.kind === destination.kind &&
    inspection.currentData.destination.id === destination.id;
  const currentInspection = inspectionMatchesDestination
    ? inspection.currentData
    : undefined;
  const checking =
    inspection.isLoading ||
    (inspection.isFetching && !currentInspection) ||
    (!currentInspection && !inspection.isError);
  const blocked = currentInspection
    ? blockedPrivacyFilesFromInspection(files, currentInspection)
    : [];
  const destinations = React.useMemo(() => {
    const candidates = policy.data?.destinations ?? [];
    return candidates.some(
      (candidate) =>
        candidate.kind === destination.kind && candidate.id === destination.id,
    )
      ? candidates
      : [destination, ...candidates];
  }, [destination, policy.data?.destinations]);
  const records = currentInspection?.records ?? files;
  const clearable = records.filter(isClearableRecord);

  const confirmClear = React.useCallback(async () => {
    if (!pendingClear) return;
    const request = { chat_id: threadId, paths: pendingClear.paths };
    setPendingClear(null);
    setClearError(null);
    try {
      const response = await clearRecords(request).unwrap();
      setSaveWarning(response.persisted === false);
      void inspection.refetch();
    } catch (error) {
      setClearError(clearErrorText(error));
    }
  }, [clearRecords, inspection, pendingClear, threadId]);

  if (!model) return null;

  const withheld = blocked.length;
  const noun = withheld === 1 ? "item" : "items";
  const note = checking
    ? "checking…"
    : withheld > 0
      ? `${withheld} ${noun} withheld`
      : null;
  const summary = checking
    ? `Checking what ${model} may receive`
    : withheld > 0
      ? `${withheld} ${noun} here can't go to ${model}`
      : `Everything here can go to ${model}`;

  return (
    <>
      <div
        className={classNames(
          styles.shield,
          withheld > 0 && styles.shieldAlert,
        )}
        data-testid="privacy-chat-shield"
        title={summary}
      >
        <Icon
          className={styles.shieldIcon}
          icon={withheld > 0 ? ShieldAlert : ShieldCheck}
          size="sm"
          tone={withheld > 0 ? "warning" : "faint"}
        />
        <Text className={styles.modelName} as="span" size="1">
          {model}
        </Text>
        {note !== null && (
          <Text className={styles.note} as="span" size="1">
            {note}
          </Text>
        )}
        <span className={styles.srOnly}>{summary}</span>
        {clearable.length > 0 && (
          <Button
            className={styles.action}
            size="sm"
            variant="ghost"
            aria-label="Manage privacy labels"
            onClick={() => setLabelsOpen(true)}
          >
            Labels
          </Button>
        )}
        <Button
          className={styles.action}
          size="sm"
          variant="ghost"
          aria-label="Inspect destination"
          onClick={() => setInspectorOpen(true)}
        >
          Inspect
        </Button>
      </div>
      <DestinationInspector
        chatId={threadId}
        destinations={destinations}
        initialDestination={destination}
        localFiles={files}
        open={inspectorOpen}
        onOpenChange={setInspectorOpen}
      />
      <Dialog open={labelsOpen} onOpenChange={setLabelsOpen}>
        <Dialog.Content maxWidth="560px">
          <Dialog.Title>Privacy labels</Dialog.Title>
          <Dialog.Description>
            Files this chat has touched and the privacy zone each one carries.
          </Dialog.Description>
          <div className={styles.inspector}>
            {saveWarning && (
              <Text color="orange" role="status">
                Labels were removed for this session but could not be saved yet;
                they will be saved on the next successful save.
              </Text>
            )}
            {clearError !== null && (
              <Text color="red" role="alert">
                {clearError}
              </Text>
            )}
            <ul className={styles.inspectorList} aria-label="Privacy labels">
              {records.map((record) => (
                <li
                  className={styles.inspectorRecord}
                  key={`${record.path}:${record.zone}:${record.attribution}`}
                >
                  <Text className={styles.path} title={record.path} size="2">
                    {record.path}
                  </Text>
                  <div className={styles.actions}>
                    <Badge tone="muted">{record.zone}</Badge>
                    {isClearableRecord(record) && (
                      <Button
                        size="sm"
                        variant="ghost"
                        aria-label={`Clear privacy label for ${record.path}`}
                        disabled={clearState.isLoading}
                        onClick={() =>
                          setPendingClear({
                            paths: [record.path],
                            zones: [record.zone],
                          })
                        }
                      >
                        Clear
                      </Button>
                    )}
                  </div>
                </li>
              ))}
            </ul>
          </div>
          <Dialog.Footer>
            <Dialog.Close asChild>
              <Button size="sm" variant="ghost">
                Close
              </Button>
            </Dialog.Close>
            <Button
              size="sm"
              variant="danger"
              disabled={clearable.length === 0 || clearState.isLoading}
              onClick={() =>
                setPendingClear({
                  paths: unique(clearable.map((record) => record.path)),
                  zones: unique(clearable.map((record) => record.zone)),
                })
              }
            >
              Clear all
            </Button>
          </Dialog.Footer>
        </Dialog.Content>
      </Dialog>
      <Dialog
        open={pendingClear !== null}
        onOpenChange={(open) => {
          if (!open) setPendingClear(null);
        }}
      >
        <Dialog.Content maxWidth="420px">
          <Dialog.Title>Clear privacy label</Dialog.Title>
          <Dialog.Description>
            {pendingClear ? clearConfirmationText(pendingClear) : ""}
          </Dialog.Description>
          <Dialog.Footer>
            <Dialog.Close asChild>
              <Button size="sm" variant="ghost">
                Cancel
              </Button>
            </Dialog.Close>
            <Button
              size="sm"
              variant="danger"
              onClick={() => void confirmClear()}
            >
              Remove label
            </Button>
          </Dialog.Footer>
        </Dialog.Content>
      </Dialog>
    </>
  );
};
