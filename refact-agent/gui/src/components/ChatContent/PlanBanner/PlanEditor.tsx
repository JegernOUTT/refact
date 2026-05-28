import React, { useEffect, useState } from "react";
import { Button, Dialog, Flex, Text, TextArea } from "@radix-ui/themes";
import { useChatActions } from "../../../hooks/useChatActions";
import styles from "./PlanBanner.module.css";

type PlanEditorProps = {
  open: boolean;
  content: string;
  onOpenChange: (open: boolean) => void;
};

export const PlanEditor: React.FC<PlanEditorProps> = ({
  open,
  content,
  onOpenChange,
}) => {
  const { submit } = useChatActions();
  const [editorValue, setEditorValue] = useState(content);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    if (open) {
      setEditorValue(content);
    }
  }, [content, open]);

  const handleSave = async () => {
    if (editorValue.trim().length === 0 || saving) return;

    setSaving(true);
    try {
      await submit(
        `Call set_plan with these exact arguments:\n${JSON.stringify(
          { content: editorValue, summary: "Manual edit" },
          null,
          2,
        )}`,
        true,
      );
      onOpenChange(false);
    } finally {
      setSaving(false);
    }
  };

  return (
    <Dialog.Root open={open} onOpenChange={onOpenChange}>
      <Dialog.Content className={styles.modalContent}>
        <Dialog.Title>Edit plan</Dialog.Title>
        <Dialog.Description size="2" color="gray">
          Update the current plan for this chat.
        </Dialog.Description>

        <Flex direction="column" gap="2" mt="3">
          <Text
            as="label"
            size="2"
            weight="medium"
            htmlFor="plan-editor-content"
          >
            Plan content
          </Text>
          <TextArea
            id="plan-editor-content"
            aria-label="Plan content"
            value={editorValue}
            onChange={(event) => setEditorValue(event.target.value)}
            className={styles.editorArea}
          />
        </Flex>

        <Flex justify="end" gap="2" mt="4">
          <Dialog.Close>
            <Button type="button" variant="soft" color="gray" disabled={saving}>
              Cancel
            </Button>
          </Dialog.Close>
          <Button
            type="button"
            onClick={() => void handleSave()}
            disabled={saving || editorValue.trim().length === 0}
          >
            Save
          </Button>
        </Flex>
      </Dialog.Content>
    </Dialog.Root>
  );
};
