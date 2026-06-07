import React from "react";
import { DashboardText as Text } from "../DashboardPrimitives";
import styles from "./SuccessGauge.module.css";

type SuccessGaugeProps = {
  successful: number;
  total: number;
};

export const SuccessGauge: React.FC<SuccessGaugeProps> = ({
  successful,
  total,
}) => {
  if (total === 0) {
    return (
      <div className={styles.gauge}>
        <Text size="2" tone="muted">
          —
        </Text>
        <div className={styles.bar}>
          <div
            className={styles.fill}
            style={{ width: "0%", background: "var(--rf-border-strong)" }}
          />
        </div>
      </div>
    );
  }

  const rate = Math.round((successful / total) * 100);
  const color =
    rate >= 95
      ? "var(--rf-color-success)"
      : rate >= 80
        ? "var(--rf-color-warning)"
        : "var(--rf-color-danger)";

  return (
    <div className={styles.gauge}>
      <Text size="3" weight="bold" style={{ color }}>
        {rate}%
      </Text>
      <div className={styles.bar}>
        <div
          className={styles.fill}
          style={{ width: `${rate}%`, background: color }}
        />
      </div>
    </div>
  );
};
