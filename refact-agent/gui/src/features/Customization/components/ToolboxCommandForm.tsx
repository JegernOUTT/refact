import React, { useCallback } from "react";

import {
  Field,
  FieldSwitch,
  FieldText,
  FieldTextarea,
} from "../../../components/ui";
import { MessageListEditor } from "./MessageListEditor";
import {
  ConfigPatch,
  safeString,
  safeBoolean,
  safeMessageArray,
  safeSelectionRange,
} from "./configUtils";
import styles from "./editors.module.css";

type ToolboxCommandFormProps = {
  config: Record<string, unknown>;
  onPatch: (patch: ConfigPatch) => void;
};

export const ToolboxCommandForm: React.FC<ToolboxCommandFormProps> = ({
  config,
  onPatch,
}) => {
  const description = safeString(config.description);
  const selectionNeeded = safeSelectionRange(config.selection_needed);
  const selectionUnwanted = safeBoolean(config.selection_unwanted);
  const insertAtCursor = safeBoolean(config.insert_at_cursor);
  const messages = safeMessageArray(config.messages);

  const hasSelectionRange = selectionNeeded !== null;
  const selectionMin = hasSelectionRange ? selectionNeeded[0] : 0;
  const selectionMax = hasSelectionRange ? selectionNeeded[1] : 0;

  const patch = useCallback(
    (path: (string | number)[], value: unknown) => {
      onPatch({ path, value });
    },
    [onPatch],
  );

  return (
    <div className={styles.formStack}>
      <Field label="Description">
        <FieldTextarea
          value={description}
          onChange={(value) => patch(["description"], value)}
          placeholder="What this command does..."
          rows={2}
        />
      </Field>

      <Field label="Require Selection">
        <FieldSwitch
          checked={hasSelectionRange}
          onChange={(checked) => {
            if (checked) {
              patch(["selection_needed"], [1, 10000]);
              patch(["selection_unwanted"], false);
            } else {
              patch(["selection_needed"], undefined);
            }
          }}
        />
      </Field>

      {hasSelectionRange && (
        <div className={styles.switchGrid}>
          <Field label="Min chars">
            <FieldText
              type="number"
              value={selectionMin.toString()}
              onChange={(value) => {
                const val = value === "" ? undefined : parseInt(value, 10);
                if (val !== undefined) {
                  patch(["selection_needed"], [val, selectionMax]);
                }
              }}
            />
          </Field>
          <Field label="Max chars">
            <FieldText
              type="number"
              value={selectionMax.toString()}
              onChange={(value) => {
                const val = value === "" ? undefined : parseInt(value, 10);
                if (val !== undefined) {
                  patch(["selection_needed"], [selectionMin, val]);
                }
              }}
            />
          </Field>
        </div>
      )}

      {!hasSelectionRange && (
        <Field
          label="Selection Unwanted"
          helper="Hide command when text is selected."
        >
          <FieldSwitch
            checked={selectionUnwanted}
            onChange={(checked) => patch(["selection_unwanted"], checked)}
          />
        </Field>
      )}

      <Field label="Insert at Cursor" helper="Insert response at cursor position.">
        <FieldSwitch
          checked={insertAtCursor}
          onChange={(checked) => patch(["insert_at_cursor"], checked)}
        />
      </Field>

      <MessageListEditor
        value={messages}
        onChange={(msgs) => patch(["messages"], msgs)}
        label="Messages"
      />
    </div>
  );
};
