import { useEffect, useState } from "react";
import { Trash2 } from "lucide-react";

import {
  Badge,
  Card,
  FieldSelect,
  FieldText,
  IconButton,
} from "../../components/ui";
import type {
  PrivacyShellBehavior,
  PrivacyZone,
} from "../../services/refact/privacy";
import { SHELL_BEHAVIOR_OPTIONS } from "./access";
import { PatternChips } from "./PatternChips";
import styles from "./PrivacySettingsSection.module.css";

export interface ZoneCardProps {
  zone: PrivacyZone;
  matchCount: number;
  saving: boolean;
  removable: boolean;
  takenNames: string[];
  onChange: (patch: Partial<PrivacyZone>) => void;
  onRemove: () => void;
}

export function ZoneCard({
  matchCount,
  onChange,
  onRemove,
  removable,
  saving,
  takenNames,
  zone,
}: ZoneCardProps) {
  const [name, setName] = useState(zone.name);

  useEffect(() => {
    setName(zone.name);
  }, [zone.name]);

  const commitName = () => {
    const next = name.trim();
    if (next.length === 0 || next === zone.name || takenNames.includes(next)) {
      setName(zone.name);
      return;
    }
    onChange({ name: next });
  };

  return (
    <Card className={`${styles.zoneCard} rf-enter`} padding="md">
      <div className={styles.zoneCardHeader}>
        <FieldText
          aria-label={`Zone name for ${zone.name}`}
          className={styles.zoneNameInput}
          disabled={saving}
          value={name}
          onBlur={commitName}
          onChange={setName}
          onKeyDown={(event) => {
            if (event.key === "Enter") {
              event.preventDefault();
              commitName();
            }
            if (event.key === "Escape") {
              event.preventDefault();
              setName(zone.name);
            }
          }}
        />
        <div className={styles.zoneCardHeaderEnd}>
          <Badge size="xs" tone="muted" variant="soft">
            {matchCount} files
          </Badge>
          {removable ? (
            <IconButton
              aria-label={`Remove zone ${zone.name}`}
              disabled={saving}
              icon={Trash2}
              size="sm"
              type="button"
              variant="danger"
              onClick={onRemove}
            />
          ) : (
            <span aria-hidden="true" className={styles.deletePlaceholder} />
          )}
        </div>
      </div>

      <PatternChips
        disabled={saving}
        emptyLabel="No patterns — this zone matches nothing"
        patterns={zone.patterns}
        onChange={(patterns) => onChange({ patterns })}
      />

      <div className={styles.zoneShellRow}>
        <span className={styles.zoneShellLabel}>
          When a shell command reads one of these files
        </span>
        <FieldSelect
          aria-label={`Shell read behavior for ${zone.name}`}
          disabled={saving}
          options={SHELL_BEHAVIOR_OPTIONS}
          value={zone.on_shell_read}
          onChange={(value) =>
            onChange({ on_shell_read: value as PrivacyShellBehavior })
          }
        />
      </div>
    </Card>
  );
}
