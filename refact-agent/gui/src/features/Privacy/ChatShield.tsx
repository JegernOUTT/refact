import React from "react";
import { Text } from "@radix-ui/themes";
import classNames from "classnames";
import { ShieldAlert, ShieldCheck } from "lucide-react";

import { Button, Icon } from "../../components/ui";
import { useAppSelector } from "../../hooks";
import {
  selectModelById,
  selectPrivacyFilesById,
} from "../Chat/Thread/selectors";
import {
  blockedPrivacyFilesFromInspection,
  privacyDestinationForModel,
  useGetPrivacyPolicyQuery,
  useInspectPrivacyQuery,
} from "../../services/refact/privacy";
import { DestinationInspector } from "./DestinationInspector";
import styles from "./PrivacyChat.module.css";

type ChatShieldProps = {
  threadId: string;
};

export const ChatShield: React.FC<ChatShieldProps> = ({ threadId }) => {
  const [inspectorOpen, setInspectorOpen] = React.useState(false);
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
    </>
  );
};
