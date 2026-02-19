import React, { useCallback } from "react";
import { Flex, TextField, Text, Switch, TextArea } from "@radix-ui/themes";
import { MessageListEditor } from "./MessageListEditor";
import {
  ConfigPatch,
  safeString,
  safeBoolean,
  safeMessageArray,
  safeSelectionRange,
} from "./configUtils";

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
    <Flex direction="column" gap="4">
      <Flex direction="column" gap="2">
        <Text size="2" weight="medium">
          Description
        </Text>
        <TextArea
          value={description}
          onChange={(e) => patch(["description"], e.target.value)}
          placeholder="What this command does..."
          rows={2}
        />
      </Flex>

      <Flex direction="column" gap="3">
        <Text size="2" weight="medium">
          Selection Requirements
        </Text>

        <Flex align="center" gap="2">
          <Switch
            checked={hasSelectionRange}
            onCheckedChange={(checked) => {
              if (checked) {
                patch(["selection_needed"], [1, 10000]);
                patch(["selection_unwanted"], false);
              } else {
                patch(["selection_needed"], undefined);
              }
            }}
          />
          <Text size="2">Require Selection</Text>
        </Flex>

        {hasSelectionRange && (
          <Flex gap="3" align="center">
            <Flex direction="column" gap="1">
              <Text size="1" color="gray">
                Min chars
              </Text>
              <TextField.Root
                type="number"
                value={selectionMin.toString()}
                onChange={(e) => {
                  const val =
                    e.target.value === ""
                      ? undefined
                      : parseInt(e.target.value);
                  if (val !== undefined) {
                    patch(["selection_needed"], [val, selectionMax]);
                  }
                }}
                style={{ width: 100 }}
              />
            </Flex>
            <Flex direction="column" gap="1">
              <Text size="1" color="gray">
                Max chars
              </Text>
              <TextField.Root
                type="number"
                value={selectionMax.toString()}
                onChange={(e) => {
                  const val =
                    e.target.value === ""
                      ? undefined
                      : parseInt(e.target.value);
                  if (val !== undefined) {
                    patch(["selection_needed"], [selectionMin, val]);
                  }
                }}
                style={{ width: 100 }}
              />
            </Flex>
          </Flex>
        )}

        {!hasSelectionRange && (
          <Flex align="center" gap="2">
            <Switch
              checked={selectionUnwanted}
              onCheckedChange={(checked) =>
                patch(["selection_unwanted"], checked)
              }
            />
            <Text size="2">Selection Unwanted</Text>
            <Text size="1" color="gray">
              (hide command when text is selected)
            </Text>
          </Flex>
        )}
      </Flex>

      <Flex align="center" gap="2">
        <Switch
          checked={insertAtCursor}
          onCheckedChange={(checked) => patch(["insert_at_cursor"], checked)}
        />
        <Text size="2">Insert at Cursor</Text>
        <Text size="1" color="gray">
          (insert response at cursor position)
        </Text>
      </Flex>

      <MessageListEditor
        value={messages}
        onChange={(msgs) => patch(["messages"], msgs)}
        label="Messages"
      />
    </Flex>
  );
};
