import React, { useCallback } from "react";
import { Flex, TextField, Text, Switch } from "@radix-ui/themes";
import { MessageListEditor } from "./MessageListEditor";
import {
  ConfigPatch,
  safeString,
  safeBoolean,
  safeMessageArray,
} from "./configUtils";

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
    <Flex direction="column" gap="4">
      <Flex direction="column" gap="2">
        <Text size="2" weight="medium">
          Label
        </Text>
        <TextField.Root
          value={label}
          onChange={(e) => patch(["label"], e.target.value)}
          placeholder="Display label in editor"
        />
        <Text size="1" color="gray">
          Text shown in the code lens above functions/classes
        </Text>
      </Flex>

      <Flex gap="4">
        <Flex direction="column" gap="2" style={{ flex: 1 }}>
          <Flex align="center" gap="2">
            <Switch
              checked={autoSubmit}
              onCheckedChange={(checked) => patch(["auto_submit"], checked)}
            />
            <Text size="2">Auto Submit</Text>
          </Flex>
          <Text size="1" color="gray">
            Automatically send message when clicked
          </Text>
        </Flex>

        <Flex direction="column" gap="2" style={{ flex: 1 }}>
          <Flex align="center" gap="2">
            <Switch
              checked={newTab}
              onCheckedChange={(checked) => patch(["new_tab"], checked)}
            />
            <Text size="2">New Tab</Text>
          </Flex>
          <Text size="1" color="gray">
            Open in a new chat tab
          </Text>
        </Flex>
      </Flex>

      <MessageListEditor
        value={messages}
        onChange={(msgs) => patch(["messages"], msgs)}
        label="Messages"
      />
    </Flex>
  );
};
