import React, { useState, useCallback, useEffect } from "react";
import { ArrowLeft, Code, Info, SlidersHorizontal } from "lucide-react";
import { skipToken } from "@reduxjs/toolkit/query";
import {
  Button,
  FieldError,
  FieldStack,
  FieldText,
  FieldTextarea,
  Icon,
  SegmentedControl,
} from "../../../components/ui";
import {
  useGetCommandQuery,
  useSaveCommandMutation,
  type CommandDetail,
} from "../../../services/refact/extensions";
import { useGetDraftQuery } from "../../../services/refact/buddy";
import { StringListEditor } from "../../Customization/components/StringListEditor";
import { Spinner } from "../../../components/Spinner";
import { BuddyDraftPreview } from "../../Buddy/BuddyDraftPreview";
import styles from "./CommandEditor.module.css";
import featureStyles from "../../featureUi.module.css";

type EditorView = "form" | "raw";

type CommandFormProps = {
  data: CommandDetail;
  onChange: (patch: Partial<CommandDetail>) => void;
  disabled: boolean;
};

const CommandForm: React.FC<CommandFormProps> = ({
  data,
  onChange,
  disabled,
}) => {
  return (
    <div className={styles.formContent}>
      <FieldStack
        label="Name"
        control={
          <FieldText value={data.name} onChange={(value) => value} disabled />
        }
      />

      <FieldStack
        label="Description"
        control={
          <FieldTextarea
            value={data.description}
            onChange={(description) => onChange({ description })}
            placeholder="Describe what this command does..."
            disabled={disabled}
          />
        }
      />

      <FieldStack
        label="Argument Hint"
        control={
          <FieldText
            value={data.argument_hint}
            onChange={(argument_hint) => onChange({ argument_hint })}
            placeholder="e.g., [file_path]"
            disabled={disabled}
          />
        }
      />

      <StringListEditor
        value={data.allowed_tools}
        onChange={(tools) => onChange({ allowed_tools: tools })}
        label="Allowed Tools"
        placeholder="Add tool..."
      />

      <FieldStack
        label="Model (optional)"
        control={
          <FieldText
            value={data.model ?? ""}
            onChange={(model) => onChange({ model: model || null })}
            placeholder="Leave blank to use default"
            disabled={disabled}
          />
        }
      />

      <FieldStack
        label="Body"
        helper="Placeholders: $ARGUMENTS, $1, $2, $3"
        control={
          <textarea
            className={styles.bodyTextarea}
            value={data.body}
            onChange={(event) => onChange({ body: event.target.value })}
            placeholder="Markdown template with $ARGUMENTS placeholder..."
            disabled={disabled}
            spellCheck={false}
          />
        }
      />
    </div>
  );
};

type CommandEditorProps = {
  name: string;
  onBack: () => void;
  draftId?: string;
};

export const CommandEditor: React.FC<CommandEditorProps> = ({
  name,
  onBack,
  draftId,
}) => {
  const { data, isLoading, error } = useGetCommandQuery({ name });
  const {
    data: draft,
    isLoading: draftLoading,
    error: draftError,
  } = useGetDraftQuery(draftId ?? skipToken);
  const [saveCommand, { isLoading: isSaving }] = useSaveCommandMutation();
  const [view, setView] = useState<EditorView>("form");
  const [localData, setLocalData] = useState<CommandDetail | null>(null);
  const [rawContent, setRawContent] = useState("");
  const [saveError, setSaveError] = useState<string | null>(null);
  const [draftExpired, setDraftExpired] = useState(false);

  useEffect(() => {
    if (draftError) {
      setDraftExpired(true);
    }
  }, [draftError]);

  useEffect(() => {
    if (draft && draft.kind === "command") {
      setRawContent(draft.yaml_or_json);
      setView("raw");
    }
  }, [draft]);

  useEffect(() => {
    if (data) {
      setLocalData(data);
      if (!draft || draft.kind !== "command") {
        setRawContent(data.raw_content);
      }
    }
  }, [data, draft]);

  const handleFormChange = useCallback((patch: Partial<CommandDetail>) => {
    setLocalData((prev) => (prev ? { ...prev, ...patch } : prev));
  }, []);

  const handleSave = useCallback(async () => {
    setSaveError(null);
    if (!localData) return;
    try {
      if (view === "raw") {
        await saveCommand({
          name,
          body: { raw_content: rawContent, draft_id: draftId },
        }).unwrap();
      } else {
        await saveCommand({
          name,
          body: {
            description: localData.description,
            argument_hint: localData.argument_hint,
            allowed_tools: localData.allowed_tools,
            model: localData.model,
            body: localData.body,
            draft_id: draftId,
          },
        }).unwrap();
      }
    } catch (e) {
      setSaveError(e instanceof Error ? e.message : String(e));
    }
  }, [name, view, localData, rawContent, saveCommand, draftId]);

  if (isLoading || draftLoading) return <Spinner spinning />;
  if (!localData) {
    return (
      <div
        className={`${featureStyles.callout} ${featureStyles.calloutDanger}`}
      >
        <Icon icon={Info} size="sm" tone="danger" />
        {error !== undefined ? "Failed to load command" : "Loading..."}
      </div>
    );
  }

  if (draft && draft.kind !== "command") {
    return (
      <div
        className={`${featureStyles.callout} ${featureStyles.calloutDanger}`}
      >
        <Icon icon={Info} size="sm" tone="danger" />
        Draft kind mismatch: expected command draft
      </div>
    );
  }

  const isReadOnly = localData.source.startsWith("plugin:");

  return (
    <div className={`${styles.editor} rf-enter`}>
      <Button
        variant="ghost"
        size="sm"
        onClick={onBack}
        className={styles.backButton}
        leftIcon={ArrowLeft}
      >
        Back to list
      </Button>

      {draftExpired && (
        <div
          className={`${featureStyles.callout} ${featureStyles.calloutWarning}`}
        >
          <Icon icon={Info} size="sm" tone="warning" />
          Draft expired
        </div>
      )}

      {draft && <BuddyDraftPreview draft={draft} />}

      {isReadOnly && (
        <div
          className={`${featureStyles.callout} ${featureStyles.calloutAccent}`}
        >
          <Icon icon={Info} size="sm" tone="accent" />
          This item is from an installed plugin and cannot be edited.
        </div>
      )}

      <div className={styles.header}>
        <h2 className={styles.title}>{name}</h2>
        <div className={styles.headerActions}>
          <SegmentedControl
            size="sm"
            value={view}
            onValueChange={(value) => setView(value as EditorView)}
            options={[
              {
                value: "form",
                label: <Icon icon={SlidersHorizontal} size="sm" />,
              },
              { value: "raw", label: <Icon icon={Code} size="sm" /> },
            ]}
          />
          {!isReadOnly && (
            <Button
              variant="primary"
              size="sm"
              onClick={() => void handleSave()}
              disabled={isSaving}
              loading={isSaving}
            >
              Save
            </Button>
          )}
        </div>
      </div>

      {saveError && <FieldError>{saveError}</FieldError>}

      {view === "form" ? (
        <CommandForm
          data={localData}
          onChange={handleFormChange}
          disabled={isReadOnly}
        />
      ) : (
        <textarea
          className={styles.rawTextarea}
          value={rawContent}
          onChange={(e) => setRawContent(e.target.value)}
          disabled={isReadOnly}
          spellCheck={false}
        />
      )}
    </div>
  );
};
