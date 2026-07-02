import React, { useMemo, useState } from "react";
import { Badge, Button, Surface, Text } from "../../components/ui";
import {
  useDecideBuddyArtifactsMutation,
  useGetBuddyArtifactsQuery,
  type ArtifactStatus,
} from "../../services/refact/buddy";
import styles from "./ArtifactsPanel.module.css";

type StatusFilter = "pending" | "all" | "applied" | "failed";

const FILTERS: { value: StatusFilter; label: string }[] = [
  { value: "pending", label: "Pending" },
  { value: "all", label: "All" },
  { value: "applied", label: "Applied" },
  { value: "failed", label: "Failed" },
];

const LIMIT_STEP = 50;

export const ArtifactsPanel: React.FC = () => {
  const [statusFilter, setStatusFilter] = useState<StatusFilter>("pending");
  const [limit, setLimit] = useState(LIMIT_STEP);
  const queryArgs = useMemo(
    () => ({
      ...(statusFilter === "all" ? {} : { status: statusFilter }),
      limit,
    }),
    [limit, statusFilter],
  );
  const { data, isLoading } = useGetBuddyArtifactsQuery(queryArgs);
  const [decide] = useDecideBuddyArtifactsMutation();

  if (isLoading || !data) return null;

  if (
    data.total_matching === 0 &&
    data.pending_count === 0 &&
    data.applied_count === 0 &&
    data.failed_count === 0 &&
    data.rejected_count === 0
  ) {
    return null;
  }

  const ops = data.ops;
  const pendingOps = ops.filter((op) => isPending(op.status));
  const canShowMore = ops.length < data.total_matching;

  return (
    <Surface
      className={styles.panel}
      animated="rise"
      data-testid="buddy-artifacts-panel"
      radius="card"
      variant="glass"
    >
      <div className={styles.header}>
        <div className={styles.titleGroup}>
          <Text as="strong" size="3" weight="bold">
            📥 Memory Ops
          </Text>
          {data.pending_count > 0 && (
            <Badge tone="warning">{data.pending_count} pending</Badge>
          )}
        </div>
        <div className={styles.headerActions}>
          <div className={styles.filters}>
            {FILTERS.map((filter) => (
              <Button
                key={filter.value}
                size="sm"
                type="button"
                variant={statusFilter === filter.value ? "primary" : "ghost"}
                onClick={() => {
                  setStatusFilter(filter.value);
                  setLimit(LIMIT_STEP);
                }}
              >
                {filter.label}
              </Button>
            ))}
          </div>
          {statusFilter === "pending" && pendingOps.length > 0 && (
            <Button
              size="sm"
              type="button"
              variant="primary"
              onClick={() =>
                void decide({
                  decisions: pendingOps.map((op) => ({
                    op_id: op.op_id,
                    accept: true,
                  })),
                })
              }
            >
              Approve all
            </Button>
          )}
        </div>
      </div>
      <div className={`scrollX ${styles.tableScroll}`}>
        <table className={styles.table}>
          <thead>
            <tr>
              <th>Title</th>
              <th>Type</th>
              <th>Status</th>
              <th>Confidence</th>
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
                <td>
                  {op.confidence !== undefined && (
                    <Text size="1" color="gray">
                      {Math.round(op.confidence * 100)}%
                    </Text>
                  )}
                </td>
                <td>{op.created_at}</td>
                <td>
                  {isPending(op.status) && (
                    <div className={styles.actions}>
                      <Button
                        size="sm"
                        type="button"
                        variant="primary"
                        onClick={() =>
                          void decide({
                            decisions: [{ op_id: op.op_id, accept: true }],
                          })
                        }
                      >
                        Approve
                      </Button>
                      <Button
                        size="sm"
                        type="button"
                        variant="danger"
                        onClick={() =>
                          void decide({
                            decisions: [{ op_id: op.op_id, accept: false }],
                          })
                        }
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
      {canShowMore && (
        <div className={styles.footerActions}>
          <Button
            size="sm"
            type="button"
            variant="ghost"
            onClick={() => setLimit((current) => current + LIMIT_STEP)}
          >
            Show more
          </Button>
        </div>
      )}
    </Surface>
  );
};

function statusTone(
  status: ArtifactStatus,
): "default" | "success" | "danger" | "warning" {
  const normalized = status.toLowerCase();
  if (normalized === "applied") return "success";
  if (normalized === "rejected" || normalized === "failed") return "danger";
  if (normalized === "pending" || normalized === "approved") return "warning";
  return "default";
}

function isPending(status: ArtifactStatus): boolean {
  return status.toLowerCase() === "pending";
}
