import { useState } from "react";
import { BookOpen, Plus } from "lucide-react";

import {
  Button,
  Dialog,
  Field,
  FieldText,
  SettingItem,
  SettingsShell,
} from "../ui";
import { DocumentationActions } from "./DocumentationActions";
import styles from "./DocumentationSettings.module.css";

export interface DocumentationSource {
  url: string;
  maxDepth: number;
  maxPages: number;
  pages: number;
}

type MaybePromise = Promise<void> | void;

export type DocumentationSettingsProps = {
  sources: DocumentationSource[];
  addDocumentation: (
    url: string,
    maxDepth: number,
    maxPages: number,
  ) => MaybePromise;
  deleteDocumentation: (url: string) => MaybePromise;
  refetchDocumentation: (url: string) => MaybePromise;
  editDocumentation: (
    url: string,
    maxDepth: number,
    maxPages: number,
  ) => MaybePromise;
  embedded?: boolean;
  hideAddAction?: boolean;
};

export type AddDocumentationActionProps = {
  addDocumentation: (
    url: string,
    maxDepth: number,
    maxPages: number,
  ) => MaybePromise;
  disabled?: boolean;
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

export const AddDocumentationAction: React.FC<AddDocumentationActionProps> = ({
  addDocumentation,
  disabled = false,
}: AddDocumentationActionProps) => {
  const [open, setOpen] = useState(false);
  const [url, setUrl] = useState("");
  const [maxDepth, setMaxDepth] = useState("2");
  const [maxPages, setMaxPages] = useState("50");
  const [formError, setFormError] = useState<string | null>(null);
  const [isSubmitting, setIsSubmitting] = useState(false);

  const resetForm = () => {
    setUrl("");
    setMaxDepth("2");
    setMaxPages("50");
    setFormError(null);
  };

  const handleOpenChange = (nextOpen: boolean) => {
    setOpen(nextOpen);
    if (!nextOpen) {
      resetForm();
    }
  };

  const handleAdd = async () => {
    const normalizedUrl = url.trim();
    const parsedDepth = parsePositiveInteger(maxDepth, "Max depth");
    const parsedPages = parsePositiveInteger(maxPages, "Max pages");

    if (!normalizedUrl) {
      setFormError("URL is required.");
      return;
    }

    if (typeof parsedDepth === "string") {
      setFormError(parsedDepth);
      return;
    }

    if (typeof parsedPages === "string") {
      setFormError(parsedPages);
      return;
    }

    setIsSubmitting(true);
    setFormError(null);
    try {
      await addDocumentation(normalizedUrl, parsedDepth, parsedPages);
      resetForm();
      setOpen(false);
    } catch (error) {
      setFormError(errorMessage(error));
    } finally {
      setIsSubmitting(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={handleOpenChange}>
      <Dialog.Trigger asChild>
        <Button variant="primary" leftIcon={Plus} disabled={disabled}>
          Add documentation
        </Button>
      </Dialog.Trigger>
      <Dialog.Content maxWidth="450px">
        <Dialog.Title>Add documentation</Dialog.Title>
        <Dialog.Description>
          Add a documentation source that the chat assistant can use.
        </Dialog.Description>
        <div className={styles.dialogBody}>
          <Field
            label="URL"
            helper="The root documentation URL to crawl."
            error={formError}
          >
            <FieldText
              value={url}
              onChange={setUrl}
              placeholder="Enter the documentation URL"
            />
          </Field>
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
            onClick={() => handleOpenChange(false)}
            disabled={isSubmitting}
          >
            Cancel
          </Button>
          <Button
            variant="primary"
            onClick={() => void handleAdd()}
            loading={isSubmitting}
          >
            Add
          </Button>
        </div>
      </Dialog.Content>
    </Dialog>
  );
};

export const DocumentationSettings: React.FC<DocumentationSettingsProps> = ({
  sources,
  addDocumentation,
  deleteDocumentation,
  editDocumentation,
  refetchDocumentation,
  embedded = false,
  hideAddAction = false,
}: DocumentationSettingsProps) => {
  const content = (
    <div className={`${styles.content} rf-enter`}>
      <SettingItem
        className="rf-enter"
        title="Sources"
        description="Configured documentation sites are indexed with their current page counts."
        layout="stack"
      >
        {sources.length > 0 ? (
          <div className={`${styles.sourceList} rf-stagger`}>
            {sources.map((source) => (
              <div className={`${styles.sourceRow} rf-enter`} key={source.url}>
                <div className={styles.sourceCopy}>
                  <span className={styles.sourceUrl}>{source.url}</span>
                  <span className={styles.sourceMeta}>
                    Max depth {source.maxDepth} · max pages {source.maxPages}
                  </span>
                </div>
                <span className={styles.pages}>{source.pages} pages</span>
                <DocumentationActions
                  source={source}
                  deleteDocumentation={deleteDocumentation}
                  editDocumentation={editDocumentation}
                  refetchDocumentation={refetchDocumentation}
                />
              </div>
            ))}
          </div>
        ) : (
          <div className={styles.empty}>
            No documentation has been added yet. Add documentation that the chat
            assistant can use.
          </div>
        )}
      </SettingItem>

      {!hideAddAction ? (
        <div className={styles.actions}>
          <AddDocumentationAction addDocumentation={addDocumentation} />
        </div>
      ) : null}
    </div>
  );

  if (embedded) {
    return content;
  }

  return (
    <SettingsShell
      className={styles.shell}
      sections={[{ id: "sources", label: "Sources", icon: BookOpen }]}
      active="sources"
      onSectionChange={() => undefined}
      title="Documentation"
      description="Manage external documentation that the chat assistant can use for grounded answers."
    >
      {content}
    </SettingsShell>
  );
};
