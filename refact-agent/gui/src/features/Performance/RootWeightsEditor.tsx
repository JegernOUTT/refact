import { useState } from "react";
import { Button, FieldError, FieldText } from "../../components/ui";
import type { TrajectorySettingValue } from "../../services/refact/performance";
import styles from "./PerformancePage.module.css";

type Props = {
  value: TrajectorySettingValue | undefined;
  onChange: (value: TrajectorySettingValue) => void;
  disabled: boolean;
  maximum?: number;
  maxEntries: number;
};

export function RootWeightsEditor({
  value,
  onChange,
  disabled,
  maximum,
  maxEntries,
}: Props) {
  const [root, setRoot] = useState("");
  const weights = typeof value === "object" && value !== null ? value : {};
  const entries = Object.entries(weights);
  const trimmed = root.trim();
  const canAdd =
    trimmed.length > 0 && !(trimmed in weights) && entries.length < maxEntries;
  return (
    <div role="group" aria-label="Root scheduling weights">
      <p>
        Unlisted roots have weight 1. Higher weights receive more admission
        opportunities, never extra capacity or authority.
      </p>
      {entries.map(([id, weight]) => (
        <div key={id} className={styles.settingControl}>
          <FieldText
            aria-label={`Scheduling weight for ${id}`}
            type="number"
            min={1}
            max={maximum}
            step={1}
            value={typeof weight === "number" ? String(weight) : ""}
            onChange={(next) =>
              onChange({
                ...weights,
                [id]: next.trim() === "" ? null : Number(next),
              })
            }
            disabled={disabled}
          />
          <Button
            variant="ghost"
            size="sm"
            disabled={disabled}
            aria-label={`Remove weight for ${id}`}
            onClick={() =>
              onChange(
                Object.fromEntries(entries.filter(([key]) => key !== id)),
              )
            }
          >
            Remove {id}
          </Button>
        </div>
      ))}
      <FieldText
        aria-label="Root chat ID for scheduling weight"
        value={root}
        onChange={setRoot}
        disabled={disabled}
      />
      {trimmed in weights ? (
        <FieldError>This root already has a weight.</FieldError>
      ) : null}
      <Button
        variant="soft"
        size="sm"
        disabled={disabled || !canAdd}
        onClick={() => {
          onChange({ ...weights, [trimmed]: 1 });
          setRoot("");
        }}
      >
        Add root weight
      </Button>
    </div>
  );
}
