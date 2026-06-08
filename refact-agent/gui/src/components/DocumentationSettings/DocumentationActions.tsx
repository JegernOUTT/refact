import { useState } from "react";
import { MoreHorizontal } from "lucide-react";

import { Button, Dialog, Field, FieldError, FieldText, Menu } from "../ui";
import type { DocumentationSource } from "./DocumentationSettings";
import styles from "./DocumentationSettings.module.css";

type MaybePromise = Promise<void> | void;

type DocumentationActionsProps = {
  source: DocumentationSource;
  deleteDocumentation: (url: string) => MaybePromise;
  editDocumentation: (
    url: string,
    maxDepth: number,
    maxPages: number,
  ) => MaybePromise;
  refetchDocumentation: (url: string) => MaybePromise;
};

function errorMessage(error: unknown): string {
  if (error instanceof Error) return error.message;
  if (typeof error === "string") return error;
  return "Something went wrong.";
}

function parsePositiveInteger(value: string, label: string): number | string {
  const parsed = Number(value);
  if (!Number.isInteger(parsed) || parsed <= 0) {
    return `${label} must be a positive integer.`;
  }
  return parsed;
}

export const DocumentationActions: React.FC<DocumentationActionsProps> = ({
  source,
  deleteDocumentation,
  editDocumentation,
  refetchDocumentation,
}: DocumentationActionsProps) => {
  const [maxDepth, setMaxDepth] = useState(String(source.maxDepth));
  const [maxPages, setMaxPages] = useState(String(source.maxPages));
  const [isDialogOpen, setIsDialogOpen] = useState(false);
  const [isDropdownOpen, setIsDropdownOpen] = useState(false);
  const [formError, setFormError] = useState<string | null>(null);
  const [isSaving, setIsSaving] = useState(false);

  const resetValues = () => {
    setMaxDepth(String(source.maxDepth));
    setMaxPages(String(source.maxPages));
    setFormError(null);
  };

  const handleSave = async () => {
    const parsedDepth = parsePositiveInteger(maxDepth, "Max depth");
    const parsedPages = parsePositiveInteger(maxPages, "Max pages");

    if (typeof parsedDepth === "string") {
      setFormError(parsedDepth);
      return;
    }

    if (typeof parsedPages === "string") {
      setFormError(parsedPages);
      return;
    }

    setIsSaving(true);
    setFormError(null);
    try {
      await editDocumentation(source.url, parsedDepth, parsedPages);
      setIsDialogOpen(false);
    } catch (error) {
      setFormError(errorMessage(error));
    } finally {
      setIsSaving(false);
    }
  };

  return (
    <>
      <Menu onOpenChange={setIsDropdownOpen}>
        <Menu.Trigger asChild>
          <Button variant="soft" rightIcon={MoreHorizontal}>
            Actions
          </Button>
        </Menu.Trigger>
        <Menu.Content>
          <Menu.Item onSelect={() => setIsDialogOpen(true)}>Edit</Menu.Item>
          <Menu.Item onSelect={() => void refetchDocumentation(source.url)}>
            Refetch
          </Menu.Item>
          <Menu.Separator />
          <Menu.Item onClick={() => void deleteDocumentation(source.url)}>
            Delete
          </Menu.Item>
        </Menu.Content>
      </Menu>
      <Dialog
        open={isDialogOpen && !isDropdownOpen}
        onOpenChange={setIsDialogOpen}
      >
        <Dialog.Content maxWidth="450px">
          <Dialog.Title>{`Edit ${source.url}`}</Dialog.Title>
          <Dialog.Description>
            Update crawl limits for this documentation source.
          </Dialog.Description>
          <div className={styles.dialogBody}>
            {formError ? <FieldError>{formError}</FieldError> : null}
            <Field
              label="Max depth"
              helper="How many link levels to follow from the root."
            >
              <FieldText
                value={maxDepth}
                onChange={setMaxDepth}
                type="number"
                min={1}
                step={1}
                placeholder="Enter the max depth"
              />
            </Field>
            <Field
              label="Max pages"
              helper="The maximum number of pages to index."
            >
              <FieldText
                value={maxPages}
                onChange={setMaxPages}
                type="number"
                min={1}
                step={1}
                placeholder="Enter the max pages"
              />
            </Field>
          </div>

          <div className={styles.dialogActions}>
            <Button
              variant="ghost"
              onClick={() => {
                resetValues();
                setIsDialogOpen(false);
              }}
              disabled={isSaving}
            >
              Cancel
            </Button>
            <Button
              variant="primary"
              onClick={() => void handleSave()}
              loading={isSaving}
            >
              Save
            </Button>
          </div>
        </Dialog.Content>
      </Dialog>
    </>
  );
};
