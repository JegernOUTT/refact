import React, { useState, useCallback } from "react";
import {
  Button,
  Callout,
  Dialog,
  Flex,
  Spinner,
  Text,
  TextField,
} from "@radix-ui/themes";
import { ExclamationTriangleIcon } from "@radix-ui/react-icons";
import { useAddMarketplaceMutation } from "../../../services/refact/plugins";

export type AddMarketplaceDialogProps = {
  open: boolean;
  onClose: () => void;
};

export const AddMarketplaceDialog: React.FC<AddMarketplaceDialogProps> = ({
  open,
  onClose,
}) => {
  const [source, setSource] = useState("");
  const [addMarketplace, { isLoading, error }] = useAddMarketplaceMutation();

  const handleAdd = useCallback(async () => {
    if (!source.trim()) return;
    try {
      await addMarketplace({ source: source.trim() }).unwrap();
      setSource("");
      onClose();
    } catch {
      // error is shown via the `error` state from RTK Query
    }
  }, [addMarketplace, source, onClose]);

  const handleOpenChange = useCallback(
    (isOpen: boolean) => {
      if (!isOpen) {
        setSource("");
        onClose();
      }
    },
    [onClose],
  );

  const errorMessage =
    error != null
      ? String(
          "data" in error
            ? error.data
            : "message" in error
              ? error.message
              : "Unknown error",
        )
      : null;

  return (
    <Dialog.Root open={open} onOpenChange={handleOpenChange}>
      <Dialog.Content style={{ maxWidth: 440 }}>
        <Dialog.Title>Add Marketplace</Dialog.Title>
        <Dialog.Description size="2" mb="4">
          Enter a GitHub repository (owner/repo) or local path to a marketplace.
        </Dialog.Description>

        <Flex direction="column" gap="3">
          <Flex direction="column" gap="1">
            <Text as="label" size="2" weight="medium">
              Marketplace source
            </Text>
            <TextField.Root
              placeholder="owner/repo or /path/to/marketplace"
              value={source}
              onChange={(e) => setSource(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") {
                  void handleAdd();
                }
              }}
            />
          </Flex>

          {errorMessage && (
            <Callout.Root color="red" size="1">
              <Callout.Icon>
                <ExclamationTriangleIcon />
              </Callout.Icon>
              <Callout.Text>{errorMessage}</Callout.Text>
            </Callout.Root>
          )}
        </Flex>

        <Flex gap="3" mt="4" justify="end">
          <Dialog.Close>
            <Button variant="soft" color="gray">
              Cancel
            </Button>
          </Dialog.Close>
          <Button
            onClick={() => void handleAdd()}
            disabled={!source.trim() || isLoading}
          >
            {isLoading ? <Spinner size="1" /> : "Add"}
          </Button>
        </Flex>
      </Dialog.Content>
    </Dialog.Root>
  );
};
