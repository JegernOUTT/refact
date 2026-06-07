import React, { useState, useCallback, useEffect } from "react";
import { ArrowLeft, Code, Info, SlidersHorizontal } from "lucide-react";
import { skipToken } from "@reduxjs/toolkit/query";
import {
  Button,
  FieldError,
  FieldSelect,
  FieldStack,
  FieldSwitch,
  FieldText,
  FieldTextarea,
  Icon,
  SegmentedControl,
} from "../../../components/ui";
import {
  useGetSkillQuery,
  useSaveSkillMutation,
  type SkillDetail,
} from "../../../services/refact/extensions";
import { useGetDraftQuery } from "../../../services/refact/buddy";
import { StringListEditor } from "../../Customization/components/StringListEditor";
import { Spinner } from "../../../components/Spinner";
import { BuddyDraftPreview } from "../../Buddy/BuddyDraftPreview";
import styles from "./SkillEditor.module.css";
import featureStyles from "../../featureUi.module.css";

type EditorView = "form" | "raw";

type SkillFormProps = {
  data: SkillDetail;
  onChange: (patch: Partial<SkillDetail>) => void;
  disabled: boolean;
};

const SkillForm: React.FC<SkillFormProps> = ({ data, onChange, disabled }) => {
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
            placeholder="Describe what this skill does..."
            disabled={disabled}
          />
        }
      />

      <div className={styles.switchRow}>
        <FieldStack
          label="User Invocable"
          control={
            <FieldSwitch
              checked={data.user_invocable}
              onChange={(user_invocable) => onChange({ user_invocable })}
              disabled={disabled}
            />
          }
        />
        <FieldStack
          label="Disable Model Invocation"
          control={
            <FieldSwitch
              checked={data.disable_model_invocation}
              onChange={(disable_model_invocation) =>
                onChange({ disable_model_invocation })
              }
              disabled={disabled}
            />
          }
        />
      </div>

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
        label="Context"
        control={
          <FieldSelect
            value={data.context ?? "none"}
            onChange={(value) =>
              onChange({ context: value === "none" ? null : value })
            }
            disabled={disabled}
            options={[
              { value: "none", label: "None" },
              { value: "fork", label: "Fork (run in subagent)" },
            ]}
          />
        }
      />

      {data.context === "fork" && (
        <FieldStack
          label="Agent (optional)"
          control={
            <FieldText
              value={data.agent ?? ""}
              onChange={(agent) => onChange({ agent: agent || null })}
              placeholder="subagent"
              disabled={disabled}
            />
          }
        />
      )}

      <FieldStack
        label="Body"
        control={
          <textarea
            className={styles.bodyTextarea}
            value={data.body}
            onChange={(event) => onChange({ body: event.target.value })}
            placeholder="Markdown content for the skill..."
            disabled={disabled}
            spellCheck={false}
          />
        }
      />
    </div>
  );
};

type SkillEditorProps = {
  name: string;
  onBack: () => void;
  draftId?: string;
};

export const SkillEditor: React.FC<SkillEditorProps> = ({
  name,
  onBack,
  draftId,
}) => {
  const { data, isLoading, error } = useGetSkillQuery({ name });
  const {
    data: draft,
    isLoading: draftLoading,
    error: draftError,
  } = useGetDraftQuery(draftId ?? skipToken);
  const [saveSkill, { isLoading: isSaving }] = useSaveSkillMutation();
  const [view, setView] = useState<EditorView>("form");
  const [localData, setLocalData] = useState<SkillDetail | null>(null);
  const [rawContent, setRawContent] = useState("");
  const [saveError, setSaveError] = useState<string | null>(null);
  const [draftExpired, setDraftExpired] = useState(false);

  useEffect(() => {
    if (draftError) {
      setDraftExpired(true);
    }
  }, [draftError]);

  useEffect(() => {
    if (draft && draft.kind === "skill") {
      setRawContent(draft.yaml_or_json);
      setView("raw");
    }
  }, [draft]);

  useEffect(() => {
    if (data) {
      setLocalData(data);
      if (!draft || draft.kind !== "skill") {
        setRawContent(data.raw_content);
      }
    }
  }, [data, draft]);

  const handleFormChange = useCallback((patch: Partial<SkillDetail>) => {
    setLocalData((prev) => (prev ? { ...prev, ...patch } : prev));
  }, []);

  const handleSave = useCallback(async () => {
    setSaveError(null);
    if (!localData) return;
    try {
      if (view === "raw") {
        await saveSkill({
          name,
          body: { raw_content: rawContent, draft_id: draftId },
        }).unwrap();
      } else {
        await saveSkill({
          name,
          body: {
            description: localData.description,
            user_invocable: localData.user_invocable,
            disable_model_invocation: localData.disable_model_invocation,
            argument_hint: localData.argument_hint,
            allowed_tools: localData.allowed_tools,
            model: localData.model,
            context: localData.context,
            agent: localData.agent,
            body: localData.body,
            draft_id: draftId,
          },
        }).unwrap();
      }
    } catch (e) {
      setSaveError(e instanceof Error ? e.message : String(e));
    }
  }, [name, view, localData, rawContent, saveSkill, draftId]);

  if (isLoading || draftLoading) return <Spinner spinning />;
  if (!localData) {
    return (
      <div
        className={`${featureStyles.callout} ${featureStyles.calloutDanger}`}
      >
        <Icon icon={Info} size="sm" tone="danger" />
        {error !== undefined ? "Failed to load skill" : "Loading..."}
      </div>
    );
  }

  if (draft && draft.kind !== "skill") {
    return (
      <div
        className={`${featureStyles.callout} ${featureStyles.calloutDanger}`}
      >
        <Icon icon={Info} size="sm" tone="danger" />
        Draft kind mismatch: expected skill draft
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
        <SkillForm
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
