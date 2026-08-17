import React from "react";
import { Flex, Text } from "@radix-ui/themes";

import { Badge, Dialog, Select, StatusDot } from "../../components/ui";
import type {
  PrivacyDestination,
  PrivacyFileRecord,
  PrivacyInspectResponse,
} from "../../services/refact/privacy";
import { useInspectPrivacyQuery } from "../../services/refact/privacy";
import styles from "./PrivacyChat.module.css";

type DestinationInspectorProps = {
  chatId: string;
  destinations: PrivacyDestination[];
  initialDestination: PrivacyDestination;
  localFiles: PrivacyFileRecord[];
  open: boolean;
  onOpenChange: (open: boolean) => void;
};

function destinationKey(destination: PrivacyDestination): string {
  return `${destination.kind}:${destination.id}`;
}

function blockedPaths(result: PrivacyInspectResponse): Set<string> {
  return new Set(result.blocked.map((blocked) => blocked.record.path));
}

export const DestinationInspector: React.FC<DestinationInspectorProps> = ({
  chatId,
  destinations,
  initialDestination,
  localFiles,
  open,
  onOpenChange,
}) => {
  const [destination, setDestination] = React.useState(initialDestination);
  const inspection = useInspectPrivacyQuery(
    { chat_id: chatId, destination, records: localFiles },
    { skip: !open },
  );

  React.useEffect(() => {
    if (!open) return;
    setDestination(initialDestination);
  }, [initialDestination, open]);

  const handleDestinationChange = React.useCallback(
    (value: string) => {
      const selected = destinations.find(
        (candidate) => destinationKey(candidate) === value,
      );
      if (selected) setDestination(selected);
    },
    [destinations],
  );

  const records = inspection.data?.records ?? localFiles;
  const blocked = inspection.data
    ? blockedPaths(inspection.data)
    : new Set<string>();
  const sendable = inspection.data?.sendable;

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <Dialog.Content maxWidth="560px">
        <Dialog.Title>Destination inspector</Dialog.Title>
        <Dialog.Description>
          Check which conversation records this destination model can receive.
        </Dialog.Description>
        <div className={styles.inspector}>
          <div className={styles.inspectorHeader}>
            <Select
              value={destinationKey(destination)}
              onValueChange={handleDestinationChange}
            >
              <Select.Trigger aria-label="Inspect destination">
                {destination.display_name}
              </Select.Trigger>
              <Select.Content maxHeight="280px">
                {destinations.map((candidate) => (
                  <Select.Item
                    key={destinationKey(candidate)}
                    value={destinationKey(candidate)}
                  >
                    {candidate.display_name}
                  </Select.Item>
                ))}
              </Select.Content>
            </Select>
            <div className={styles.inspectorStatus} role="status">
              <StatusDot
                status={
                  inspection.isLoading
                    ? "running"
                    : sendable === false
                      ? "warning"
                      : sendable === true
                        ? "success"
                        : "idle"
                }
              />
              <Text size="2">
                {inspection.isLoading
                  ? "Checking this model…"
                  : sendable === false
                    ? `${blocked.size} records cannot go to this model`
                    : sendable === true
                      ? "This model can receive the conversation"
                      : "No inspection result yet"}
              </Text>
            </div>
          </div>

          {inspection.isError && (
            <Text color="red" role="alert">
              The destination check could not be loaded.
            </Text>
          )}

          <ul className={styles.inspectorList} aria-label="Privacy records">
            {records.map((record) => (
              <li
                className={styles.inspectorRecord}
                key={`${record.path}:${record.zone}:${record.attribution}`}
              >
                <Text className={styles.path} title={record.path} size="2">
                  {record.path}
                </Text>
                <Flex gap="1" align="center">
                  <Badge tone={blocked.has(record.path) ? "warning" : "muted"}>
                    {record.zone}
                  </Badge>
                  <Badge tone="muted">{record.attribution}</Badge>
                </Flex>
              </li>
            ))}
          </ul>

          {records.length === 0 && !inspection.isLoading && (
            <Text color="gray">No file records are attached to this chat.</Text>
          )}
        </div>
      </Dialog.Content>
    </Dialog>
  );
};
