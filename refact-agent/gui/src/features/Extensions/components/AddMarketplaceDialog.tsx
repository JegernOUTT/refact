import React, { useState, useCallback } from "react";
import { AlertTriangle } from "lucide-react";
import {
  Button,
  Dialog,
  FieldError,
  FieldStack,
  FieldText,
  Icon,
} from "../../../components/ui";
import { useAddMarketplaceMutation } from "../../../services/refact/plugins";
import featureStyles from "../../featureUi.module.css";
import styles from "./ExtensionDialog.module.css";

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
      return;
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
    <Dialog open={open} onOpenChange={handleOpenChange}>
      <Dialog.Content maxWidth="440px">
        <Dialog.Title>Add Marketplace</Dialog.Title>
        <Dialog.Description>
          Enter a GitHub repository (owner/repo) or local path to a marketplace.
        </Dialog.Description>

        <FieldStack
          label="Marketplace source"
          control={
            <FieldText
              placeholder="owner/repo or /path/to/marketplace"
              value={source}
              onChange={setSource}
              onKeyDown={(event) => {
                if (event.key === "Enter") {
                  void handleAdd();
                }
              }}
            />
          }
        />

        {errorMessage && (
          <div
            className={`${featureStyles.callout} ${featureStyles.calloutDanger}`}
          >
            <Icon icon={AlertTriangle} size="sm" tone="danger" />
            <FieldError>{errorMessage}</FieldError>
          </div>
        )}

        <div className={styles.actions}>
          <Dialog.Close asChild>
            <Button variant="soft">Cancel</Button>
          </Dialog.Close>
          <Button
            variant="primary"
            onClick={() => void handleAdd()}
            disabled={!source.trim() || isLoading}
            loading={isLoading}
          >
            Add
          </Button>
        </div>
      </Dialog.Content>
    </Dialog>
  );
};
