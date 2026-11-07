import { useEffect, useState } from "react";
import { Button, Callout, Flex, Text } from "@radix-ui/themes";
import {
  useRecoverBuddyPublicationMutation,
  type OperatorStatus,
  type PendingPublication,
} from "../../services/refact/buddyOperator";

/** Only metadata crosses this boundary; retry never re-executes the workflow. */
export function BuddyPublicationRecovery({
  status,
  identity,
}: {
  status: OperatorStatus;
  identity: string;
}) {
  const [recover, { isLoading }] = useRecoverBuddyPublicationMutation();
  const [pending, setPending] = useState<PendingPublication[]>([]);
  const [failed, setFailed] = useState(false);
  const project = status.project;
  const worker = status.worker;
  useEffect(() => {
    let active = true;
    setPending([]);
    setFailed(false);
    if (status.mode === "enabled" && project && worker) {
      void recover({ endpoint_identity: identity, project, worker })
        .unwrap()
        .then((rows) => {
          if (active) setPending(rows);
        })
        .catch(() => {
          if (active) setFailed(true);
        });
    }
    return () => {
      active = false;
    };
  }, [identity, status.mode, project, worker, recover]);
  if (!project || !worker || (!pending.length && !failed)) return null;
  return (
    <Callout.Root color="amber" role="status">
      <Callout.Text>
        Some workflow results may be waiting to be saved. Retained publications
        are memory-only and are lost on engine restart, runtime reopen, or
        project switch; disk-full errors cannot preserve unwritten bytes.
        Available saved assessments and trajectories remain evidence. Retrying
        publication does not rerun models or tools.
      </Callout.Text>
      <Flex direction="column" gap="2">
        {failed && (
          <Text size="2">
            Publication recovery is unavailable. Check storage and reload Buddy;
            do not rerun side effects to recover a report.
          </Text>
        )}
        {pending.map((row) => (
          <Button
            key={row.run_id}
            disabled={isLoading}
            onClick={() => {
              void recover({
                endpoint_identity: identity,
                project,
                worker,
                run_id: row.run_id,
              })
                .unwrap()
                .then((rows) => {
                  setPending(rows);
                  setFailed(false);
                })
                .catch(() => {
                  setFailed(true);
                });
            }}
          >
            Retry saving {row.workflow_id}
          </Button>
        ))}
      </Flex>
    </Callout.Root>
  );
}
