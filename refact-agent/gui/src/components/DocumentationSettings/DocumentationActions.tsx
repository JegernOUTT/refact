import { useState } from "react";
import { MoreHorizontal } from "lucide-react";

import { Button, Dialog, Field, FieldText, Menu } from "../ui";
import type { DocumentationSource } from "./DocumentationSettings";
import styles from "./DocumentationSettings.module.css";

type DocumentationActionsProps = {
  source: DocumentationSource;
  deleteDocumentation: (url: string) => void;
  editDocumentation: (url: string, maxDepth: number, maxPages: number) => void;
  refetchDocumentation: (url: string) => void;
};

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

  const resetValues = () => {
    setMaxDepth(String(source.maxDepth));
    setMaxPages(String(source.maxPages));
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
          <Menu.Item onSelect={() => refetchDocumentation(source.url)}>Refetch</Menu.Item>
          <Menu.Separator />
          <Menu.Item onClick={() => deleteDocumentation(source.url)}>Delete</Menu.Item>
        </Menu.Content>
      </Menu>
      <Dialog open={isDialogOpen && !isDropdownOpen} onOpenChange={setIsDialogOpen}>
        <Dialog.Content maxWidth="450px">
          <Dialog.Title>{`Edit ${source.url}`}</Dialog.Title>
          <Dialog.Description>Update crawl limits for this documentation source.</Dialog.Description>
          <div className={styles.dialogBody}>
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
              <Button variant="ghost" onClick={resetValues}>
                Cancel
              </Button>
            </Dialog.Close>
            <Dialog.Close asChild>
              <Button
                variant="primary"
                onClick={() => editDocumentation(source.url, Number(maxDepth), Number(maxPages))}
              >
                Save
              </Button>
            </Dialog.Close>
          </div>
        </Dialog.Content>
      </Dialog>
    </>
  );
};
