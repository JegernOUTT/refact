import { useState } from "react";
import { ArrowLeft, BookOpen, Plus } from "lucide-react";

import { Button, Dialog, Field, FieldText, SettingItem, SettingsShell } from "../ui";
import { DocumentationActions } from "./DocumentationActions";
import styles from "./DocumentationSettings.module.css";

export interface DocumentationSource {
  url: string;
  maxDepth: number;
  maxPages: number;
  pages: number;
}

export type DocumentationSettingsProps = {
  sources: DocumentationSource[];
  addDocumentation: (url: string, maxDepth: number, maxPages: number) => void;
  deleteDocumentation: (url: string) => void;
  refetchDocumentation: (url: string) => void;
  editDocumentation: (url: string, maxDepth: number, maxPages: number) => void;
};

export const DocumentationSettings: React.FC<DocumentationSettingsProps> = ({
  sources,
  addDocumentation,
  deleteDocumentation,
  editDocumentation,
  refetchDocumentation,
}: DocumentationSettingsProps) => {
  const [url, setUrl] = useState("");
  const [maxDepth, setMaxDepth] = useState("2");
  const [maxPages, setMaxPages] = useState("50");

  const resetForm = () => {
    setUrl("");
    setMaxDepth("2");
    setMaxPages("50");
  };

  const handleAdd = () => {
    addDocumentation(url, Number(maxDepth), Number(maxPages));
    resetForm();
  };

  return (
    <SettingsShell
      className={styles.shell}
      sections={[{ id: "sources", label: "Sources", icon: BookOpen }]}
      active="sources"
      onSectionChange={() => undefined}
      title="Documentation"
      description="Manage external documentation that the chat assistant can use for grounded answers."
    >
      <div className={`${styles.content} rf-enter`}>
        <div className={styles.sectionHeader}>
          <h2 className={styles.title}>Documentation sources</h2>
          <p className={styles.description}>Add, refresh, or tune crawl limits for documentation sites.</p>
        </div>

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
              No documentation has been added yet. Add documentation that the chat assistant can use.
            </div>
          )}
        </SettingItem>

        <div className={styles.actions}>
          <Button variant="ghost" leftIcon={ArrowLeft}>
            Back
          </Button>
          <Dialog>
            <Dialog.Trigger asChild>
              <Button variant="primary" leftIcon={Plus}>
                Add documentation
              </Button>
            </Dialog.Trigger>
            <Dialog.Content maxWidth="450px">
              <Dialog.Title>Add documentation</Dialog.Title>
              <Dialog.Description>
                Add a documentation source that the chat assistant can use.
              </Dialog.Description>
              <div className={styles.dialogBody}>
                <Field label="Url" helper="The root documentation URL to crawl.">
                  <FieldText value={url} onChange={setUrl} placeholder="Enter the documentation url" />
                </Field>
                <Field label="Max depth" helper="How many link levels to follow from the root.">
                  <FieldText
                    value={maxDepth}
                    onChange={setMaxDepth}
                    type="number"
                    placeholder="Enter the max depth"
                  />
                </Field>
                <Field label="Max pages" helper="The maximum number of pages to index.">
                  <FieldText
                    value={maxPages}
                    onChange={setMaxPages}
                    type="number"
                    placeholder="Enter the max pages"
                  />
                </Field>
              </div>
              <div className={styles.dialogActions}>
                <Dialog.Close asChild>
                  <Button variant="ghost" onClick={resetForm}>
                    Cancel
                  </Button>
                </Dialog.Close>
                <Dialog.Close asChild>
                  <Button variant="primary" onClick={handleAdd}>
                    Add
                  </Button>
                </Dialog.Close>
              </div>
            </Dialog.Content>
          </Dialog>
        </div>
      </div>
    </SettingsShell>
  );
};
