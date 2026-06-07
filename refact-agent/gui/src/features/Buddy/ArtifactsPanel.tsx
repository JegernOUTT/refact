import React from "react";
import { Badge, Button, Card, Text } from "../../components/ui";
import {
  useApproveBuddyArtifactMutation,
  useGetBuddyArtifactsQuery,
  useRejectBuddyArtifactMutation,
  type ArtifactStatus,
} from "../../services/refact/buddy";
import styles from "./ArtifactsPanel.module.css";

export const ArtifactsPanel: React.FC = () => {
  const { data, isLoading } = useGetBuddyArtifactsQuery(undefined);
  const [approve] = useApproveBuddyArtifactMutation();
  const [reject] = useRejectBuddyArtifactMutation();

  if (isLoading) return null;

  const ops = data?.ops ?? [];

  return (
    <Card className={styles.panel} animated="rise">
      <div className={styles.header}>
        <Text as="strong" size="3" weight="bold">
          📥 Memory Ops
        </Text>
      </div>
      <div className="scrollX">
        <table className={styles.table}>
          <thead>
            <tr>
              <th>Title</th>
              <th>Type</th>
              <th>Status</th>
              <th>Created</th>
              <th>Actions</th>
            </tr>
          </thead>
          <tbody>
            {ops.map((op) => (
              <tr key={op.op_id} className="rf-enter-rise">
                <td>{op.title ?? op.payload?.title ?? op.op_id}</td>
                <td>{op.op_type}</td>
                <td>
                  <Badge tone={statusTone(op.status)}>{op.status}</Badge>
                </td>
                <td>{op.created_at}</td>
                <td>
                  {isPending(op.status) && (
                    <div className={styles.actions}>
                      <Button
                        size="sm"
                        variant="primary"
                        onClick={() => void approve({ op_id: op.op_id })}
                      >
                        Approve
                      </Button>
                      <Button
                        size="sm"
                        variant="danger"
                        onClick={() => void reject({ op_id: op.op_id })}
                      >
                        Reject
                      </Button>
                    </div>
                  )}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </Card>
  );
};

function statusTone(status: ArtifactStatus): "default" | "success" | "danger" | "warning" {
  const normalized = status.toLowerCase();
  if (normalized === "applied") return "success";
  if (normalized === "rejected" || normalized === "failed") return "danger";
  if (normalized === "pending" || normalized === "approved") return "warning";
  return "default";
}

function isPending(status: ArtifactStatus): boolean {
  return status.toLowerCase() === "pending";
}
