import React, { useState, useCallback } from "react";
import { File, Globe } from "lucide-react";
import {
  Badge,
  Button,
  Dialog,
  FieldError,
  FieldStack,
  FieldText,
  Icon,
  SegmentedControl,
} from "../../../components/ui";
import {
  useCreateSkillMutation,
  useCreateCommandMutation,
} from "../../../services/refact/extensions";
import styles from "./ExtensionDialog.module.css";

type ItemType = "skill" | "command";

type CreateItemDialogProps = {
  type: ItemType;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onCreated: (name: string) => void;
  hasProjectRoot: boolean;
};

function validateName(name: string): string | null {
  if (!name.trim()) return "Name is required";
  if (/[\s/.]/.test(name))
    return "Name must not contain spaces, slashes, or dots";
  return null;
}

export const CreateItemDialog: React.FC<CreateItemDialogProps> = ({
  type,
  open,
  onOpenChange,
  onCreated,
  hasProjectRoot,
}) => {
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [scope, setScope] = useState<"global" | "local">(
    hasProjectRoot ? "local" : "global",
  );
  const [error, setError] = useState<string | null>(null);

  const [createSkill, { isLoading: isCreatingSkill }] =
    useCreateSkillMutation();
  const [createCommand, { isLoading: isCreatingCommand }] =
    useCreateCommandMutation();
  const isLoading = isCreatingSkill || isCreatingCommand;

  React.useEffect(() => {
    setScope(hasProjectRoot ? "local" : "global");
  }, [hasProjectRoot]);

  React.useEffect(() => {
    if (open) {
      setName("");
      setDescription("");
      setError(null);
    }
  }, [open]);

  const handleCreate = useCallback(async () => {
    setError(null);
    const validationError = validateName(name);
    if (validationError) {
      setError(validationError);
      return;
    }
    try {
      if (type === "skill") {
        await createSkill({ name, scope, description, body: "" }).unwrap();
      } else {
        await createCommand({ name, scope, description }).unwrap();
      }
      onOpenChange(false);
      onCreated(name);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }, [
    type,
    name,
    scope,
    description,
    createSkill,
    createCommand,
    onOpenChange,
    onCreated,
  ]);

  const title = type === "skill" ? "Create Skill" : "Create Command";

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <Dialog.Content maxWidth="400px">
        <Dialog.Title>{title}</Dialog.Title>
        <div className={styles.form}>
          <FieldStack
            label="Name"
            control={
              <FieldText
                placeholder="my_skill"
                value={name}
                onChange={setName}
              />
            }
          />
          <FieldStack
            label="Description (optional)"
            control={
              <FieldText
                placeholder="Brief description"
                value={description}
                onChange={setDescription}
              />
            }
          />
          <FieldStack
            label="Save to"
            control={
              hasProjectRoot ? (
                <SegmentedControl
                  size="sm"
                  value={scope}
                  onValueChange={(value) =>
                    setScope(value as "global" | "local")
                  }
                  options={[
                    {
                      value: "global",
                      label: (
                        <span className={styles.scopeLabel}>
                          <Icon icon={Globe} size="sm" /> Global
                        </span>
                      ),
                    },
                    {
                      value: "local",
                      label: (
                        <span className={styles.scopeLabel}>
                          <Icon icon={File} size="sm" /> Project
                        </span>
                      ),
                    },
                  ]}
                />
              ) : (
                <Badge tone="accent">
                  <span className={styles.scopeLabel}>
                    <Icon icon={Globe} size="sm" /> Global only (no project
                    open)
                  </span>
                </Badge>
              )
            }
          />
          {error && <FieldError>{error}</FieldError>}
        </div>
        <div className={styles.actions}>
          <Dialog.Close asChild>
            <Button variant="soft">Cancel</Button>
          </Dialog.Close>
          <Button
            variant="primary"
            onClick={() => void handleCreate()}
            disabled={isLoading}
            loading={isLoading}
          >
            Create
          </Button>
        </div>
      </Dialog.Content>
    </Dialog>
  );
};
