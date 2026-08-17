import React from "react";
import { Text } from "@radix-ui/themes";
import { ShieldCheck } from "lucide-react";

import { Button } from "../../components/ui";
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
  const blocked = blockedPrivacyFilesFromInspection(files, inspection.data);
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

  return (
    <>
      <div className={styles.shield} data-testid="privacy-chat-shield">
        <div className={styles.shieldSummary}>
          <ShieldCheck className={styles.shieldIcon} aria-hidden="true" />
          <div className={styles.shieldCopy}>
            <Text
              className={styles.modelName}
              as="div"
              size="2"
              weight="medium"
            >
              {model}
            </Text>
            <Text className={styles.restriction} as="div" size="1">
              {inspection.isLoading
                ? "Checking what this model may receive…"
                : `${blocked.length} ${
                    blocked.length === 1 ? "thing" : "things"
                  } here can't go to ${model}`}
            </Text>
          </div>
        </div>
        <Button
          size="sm"
          variant="plain"
          onClick={() => setInspectorOpen(true)}
        >
          Inspect destination
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
