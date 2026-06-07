import React, { useCallback } from "react";

import {
  Field,
  FieldSwitch,
  FieldText,
  SettingsShell,
} from "../../../components/ui";
import { MessageListEditor } from "./MessageListEditor";
import {
  ConfigPatch,
  safeString,
  safeBoolean,
  safeMessageArray,
} from "./configUtils";
import styles from "./editors.module.css";

type CodeLensFormProps = {
  config: Record<string, unknown>;
  onPatch: (patch: ConfigPatch) => void;
};

export const CodeLensForm: React.FC<CodeLensFormProps> = ({
  config,
  onPatch,
}) => {
  const label = safeString(config.label);
  const autoSubmit = safeBoolean(config.auto_submit);
  const newTab = safeBoolean(config.new_tab);
  const messages = safeMessageArray(config.messages);

  const patch = useCallback(
    (path: (string | number)[], value: unknown) => {
      onPatch({ path, value });
    },
    [onPatch],
  );

  return (
    <SettingsShell
      active="code-lens"
      sections={[{ id: "code-lens", label: "Code Lens" }]}
      title="Code Lens"
      description="Configure the editor action, target tab, and generated messages."
      onSectionChange={() => undefined}
    >
      <div className={styles.formStack}>
        <Field
          label="Label"
          helper="Text shown in the code lens above functions/classes."
        >
          <FieldText
            value={label}
            onChange={(value) => patch(["label"], value)}
            placeholder="Display label in editor"
          />
        </Field>

        <div className={styles.switchGrid}>
          <Field label="Auto Submit" helper="Automatically send message when clicked.">
            <FieldSwitch
              checked={autoSubmit}
              onChange={(checked) => patch(["auto_submit"], checked)}
            />
          </Field>

          <Field label="New Tab" helper="Open in a new chat tab.">
            <FieldSwitch
              checked={newTab}
              onChange={(checked) => patch(["new_tab"], checked)}
            />
          </Field>
        </div>

        <MessageListEditor
          value={messages}
          onChange={(msgs) => patch(["messages"], msgs)}
          label="Messages"
        />
      </div>
    </SettingsShell>
  );
};
