import React, { useState, useCallback, useEffect } from "react";
import { Bot, Code2, MessageSquare, Settings, Workflow } from "lucide-react";
import { StringListEditor } from "./StringListEditor";
import { ToolParametersEditor, ToolParameter } from "./ToolParametersEditor";
import { toInputSchema, fromInputSchema } from "../../../utils/toolSchema";
import { MessageListEditor } from "./MessageListEditor";
import {
  Button,
  Field,
  FieldSwitch,
  FieldText,
  FieldTextarea,
  SettingsShell,
} from "../../../components/ui";
import {
  ConfigPatch,
  extractSubagentExtra,
  computeExtraPatches,
  safeArray,
  safeString,
  safeBoolean,
  safeObject,
  isString,
  isPlainObject,
  sanitizeObject,
  safeNumber,
  safeMessageArray,
  parseIntSafe,
} from "./configUtils";
import styles from "./editors.module.css";

type SubagentFormProps = {
  config: Record<string, unknown>;
  onPatch: (patch: ConfigPatch) => void;
  availableTools?: string[];
};

const SUBAGENT_SECTIONS = [
  { id: "basic", label: "Basic", icon: Bot },
  { id: "tool", label: "Tool Schema", icon: Code2 },
  { id: "subchat", label: "Subchat", icon: Workflow },
  { id: "messages", label: "Messages", icon: MessageSquare },
  { id: "advanced", label: "Advanced", icon: Settings },
];

export const SubagentForm: React.FC<SubagentFormProps> = ({
  config,
  onPatch,
  availableTools = [],
}) => {
  const [activeTab, setActiveTab] = useState("basic");
  const [extraJson, setExtraJson] = useState("");
  const [extraJsonDirty, setExtraJsonDirty] = useState(false);
  const [extraJsonError, setExtraJsonError] = useState<string | null>(null);

  const extra = extractSubagentExtra(config);
  const configId = safeString(config.id);

  useEffect(() => {
    if (!extraJsonDirty) {
      const newExtra = extractSubagentExtra(config);
      const newJson =
        Object.keys(newExtra).length === 0
          ? ""
          : JSON.stringify(newExtra, null, 2);
      setExtraJson(newJson);
      setExtraJsonError(null);
    }
  }, [configId, config, extraJsonDirty]);

  const title = safeString(config.title);
  const description = safeString(config.description);
  const specific = safeBoolean(config.specific);
  const exposeAsTool = safeBoolean(config.expose_as_tool);
  const hasCode = safeBoolean(config.has_code);
  const tools = safeArray(config.tools, isString);
  const tool = config.tool !== undefined ? safeObject(config.tool) : undefined;
  const subchat = safeObject(config.subchat);
  const messages = safeObject(config.messages);
  const prompts = safeObject(config.prompts);
  const gatherFiles = safeObject(config.gather_files);
  const base = typeof config.base === "string" ? config.base : undefined;
  const matchModels = Array.isArray(config.match_models)
    ? safeArray(config.match_models, isString)
    : undefined;

  const patch = useCallback(
    (path: (string | number)[], value: unknown) => {
      onPatch({ path, value });
    },
    [onPatch],
  );

  const handleExtraChange = useCallback((text: string) => {
    setExtraJson(text);
    setExtraJsonDirty(true);
    setExtraJsonError(null);
  }, []);

  const applyExtraChanges = useCallback(() => {
    try {
      const parsed: unknown = extraJson.trim() ? JSON.parse(extraJson) : {};
      if (!isPlainObject(parsed)) {
        setExtraJsonError("Extra fields must be a JSON object");
        return;
      }
      const newExtra = sanitizeObject(parsed) as Record<string, unknown>;
      const patches = computeExtraPatches(extra, newExtra);
      for (const p of patches) {
        onPatch(p);
      }
      setExtraJsonDirty(false);
      setExtraJsonError(null);
    } catch (e) {
      setExtraJsonError(e instanceof Error ? e.message : "Invalid JSON");
    }
  }, [extraJson, extra, onPatch]);

  return (
    <SettingsShell
      active={activeTab}
      sections={SUBAGENT_SECTIONS}
      title="Subagent"
      description="Configure subagent metadata, tool schema, prompts, and execution defaults."
      onSectionChange={setActiveTab}
    >
      <div className={styles.formTabContent}>
        {activeTab === "basic" && (
          <BasicTab
            title={title}
            description={description}
            specific={specific}
            exposeAsTool={exposeAsTool}
            hasCode={hasCode}
            tools={tools}
            patch={patch}
            availableTools={availableTools}
          />
        )}
        {activeTab === "tool" && <ToolTab tool={tool} patch={patch} />}
        {activeTab === "subchat" && (
          <SubchatTab subchat={subchat} patch={patch} />
        )}
        {activeTab === "messages" && (
          <MessagesTab messages={messages} prompts={prompts} patch={patch} />
        )}
        {activeTab === "advanced" && (
          <AdvancedTab
            base={base}
            matchModels={matchModels}
            gatherFiles={gatherFiles}
            extraJson={extraJson}
            extraJsonDirty={extraJsonDirty}
            extraJsonError={extraJsonError}
            onExtraChange={handleExtraChange}
            onExtraApply={applyExtraChanges}
            patch={patch}
          />
        )}
      </div>
    </SettingsShell>
  );
};

type PatchFn = (path: (string | number)[], value: unknown) => void;

const BasicTab: React.FC<{
  title: string;
  description: string;
  specific: boolean;
  exposeAsTool: boolean;
  hasCode: boolean;
  tools: string[];
  patch: PatchFn;
  availableTools: string[];
}> = ({
  title,
  description,
  specific,
  exposeAsTool,
  hasCode,
  tools,
  patch,
  availableTools,
}) => (
  <>
    <Field label="Title">
      <FieldText
        value={title}
        onChange={(value) => patch(["title"], value)}
        placeholder="Display name"
      />
    </Field>

    <Field label="Description">
      <FieldTextarea
        value={description}
        onChange={(value) => patch(["description"], value)}
        placeholder="What this subagent does..."
        rows={2}
      />
    </Field>

    <div className={styles.switchGrid}>
      <Field label="Internal Only">
        <FieldSwitch
          checked={specific}
          onChange={(checked) => patch(["specific"], checked)}
        />
      </Field>
      <Field label="Expose as Tool">
        <FieldSwitch
          checked={exposeAsTool}
          onChange={(checked) => patch(["expose_as_tool"], checked)}
        />
      </Field>
      <Field label="Has Code">
        <FieldSwitch
          checked={hasCode}
          onChange={(checked) => patch(["has_code"], checked)}
        />
      </Field>
    </div>

    <StringListEditor
      value={tools}
      onChange={(t) => patch(["tools"], t)}
      label="Available Tools"
      placeholder="Add tool..."
      suggestions={availableTools}
    />
  </>
);

const ToolTab: React.FC<{
  tool: Record<string, unknown> | undefined;
  patch: PatchFn;
}> = ({ tool, patch }) => {
  const hasTool = tool !== undefined;
  const toolDesc =
    typeof tool?.description === "string" ? tool.description : "";
  const agentic = typeof tool?.agentic === "boolean" ? tool.agentic : false;
  const allowParallel =
    typeof tool?.allow_parallel === "boolean" ? tool.allow_parallel : false;

  const inputSchema =
    tool?.input_schema &&
    typeof tool.input_schema === "object" &&
    !Array.isArray(tool.input_schema)
      ? (tool.input_schema as Record<string, unknown>)
      : {};
  const { params: parameters, required } = fromInputSchema(inputSchema);

  const handleParametersChange = (newParams: ToolParameter[]) => {
    patch(["tool", "input_schema"], toInputSchema(newParams, required));
  };

  const handleRequiredChange = (newRequired: string[]) => {
    patch(["tool", "input_schema"], toInputSchema(parameters, newRequired));
  };

  return (
    <>
      <Field label="Define Custom Tool Schema">
        <FieldSwitch
          checked={hasTool}
          onChange={(checked) => {
            if (checked) {
              patch(["tool"], {
                description: "",
                agentic: false,
                allow_parallel: false,
                input_schema: { type: "object", properties: {}, required: [] },
              });
            } else {
              patch(["tool"], undefined);
            }
          }}
        />
      </Field>

      {hasTool && (
        <>
          <Field label="Tool Description">
            <FieldTextarea
              value={toolDesc}
              onChange={(value) => patch(["tool", "description"], value)}
              placeholder="Description shown to the LLM..."
              rows={2}
            />
          </Field>

          <div className={styles.switchGrid}>
            <Field label="Agentic" helper="Tool can make multiple calls.">
              <FieldSwitch
                checked={agentic}
                onChange={(checked) => patch(["tool", "agentic"], checked)}
              />
            </Field>

            <Field
              label="Allow Parallel"
              helper="Tool can run concurrently with other parallel tools."
            >
              <FieldSwitch
                checked={allowParallel}
                onChange={(checked) =>
                  patch(["tool", "allow_parallel"], checked)
                }
              />
            </Field>
          </div>

          <ToolParametersEditor
            parameters={parameters}
            required={required}
            onParametersChange={handleParametersChange}
            onRequiredChange={handleRequiredChange}
          />
        </>
      )}
    </>
  );
};

const SubchatTab: React.FC<{
  subchat: Record<string, unknown>;
  patch: PatchFn;
}> = ({ subchat, patch }) => {
  return (
    <>
      <div className={styles.fieldGrid}>
        <Field label="Context Mode">
          <FieldText
            value={safeString(subchat.context_mode) || "bare"}
            onChange={(value) => patch(["subchat", "context_mode"], value)}
            placeholder="bare / full / ..."
          />
        </Field>
        <Field label="Model">
          <FieldText
            value={safeString(subchat.model)}
            onChange={(value) => patch(["subchat", "model"], value || undefined)}
            placeholder="Default"
          />
        </Field>
        <Field label="Model Type">
          <FieldText
            value={safeString(subchat.model_type)}
            onChange={(value) =>
              patch(["subchat", "model_type"], value || undefined)
            }
            placeholder="Default"
          />
        </Field>
      </div>

      <Field label="Stateful">
        <FieldSwitch
          checked={safeBoolean(subchat.stateful)}
          onChange={(checked) => patch(["subchat", "stateful"], checked)}
        />
      </Field>

      <div className={styles.fieldGrid}>
        <Field label="Max Steps">
          <FieldText
            type="number"
            value={safeNumber(subchat.max_steps)?.toString() ?? ""}
            onChange={(value) =>
              patch(["subchat", "max_steps"], parseIntSafe(value))
            }
            placeholder="Default"
          />
        </Field>
        <Field label="N Context">
          <FieldText
            type="number"
            value={safeNumber(subchat.n_ctx)?.toString() ?? ""}
            onChange={(value) => patch(["subchat", "n_ctx"], parseIntSafe(value))}
            placeholder="Default"
          />
        </Field>
        <Field label="Max New Tokens">
          <FieldText
            type="number"
            value={safeNumber(subchat.max_new_tokens)?.toString() ?? ""}
            onChange={(value) =>
              patch(["subchat", "max_new_tokens"], parseIntSafe(value))
            }
            placeholder="Default"
          />
        </Field>
      </div>

      <div className={styles.fieldGrid}>
        <Field label="Reasoning Effort">
          <FieldText
            value={safeString(subchat.reasoning_effort)}
            onChange={(value) =>
              patch(["subchat", "reasoning_effort"], value || undefined)
            }
            placeholder="low / medium / high / xhigh / max"
          />
        </Field>
        <Field label="Tokens for RAG">
          <FieldText
            type="number"
            value={safeNumber(subchat.tokens_for_rag)?.toString() ?? ""}
            onChange={(value) =>
              patch(["subchat", "tokens_for_rag"], parseIntSafe(value))
            }
            placeholder="Default"
          />
        </Field>
      </div>
    </>
  );
};

const MessagesTab: React.FC<{
  messages: Record<string, unknown>;
  prompts: Record<string, unknown>;
  patch: PatchFn;
}> = ({ messages, prompts, patch }) => (
  <>
    <Field label="System Prompt">
      <FieldTextarea
        value={safeString(messages.system_prompt)}
        onChange={(value) =>
          patch(["messages", "system_prompt"], value || undefined)
        }
        placeholder="System prompt..."
        className={styles.promptTextarea}
      />
    </Field>

    <Field label="User Template">
      <FieldTextarea
        value={safeString(messages.user_template)}
        onChange={(value) =>
          patch(["messages", "user_template"], value || undefined)
        }
        placeholder="User message template..."
        rows={3}
        className={styles.promptTextarea}
      />
    </Field>

    <MessageListEditor
      value={safeMessageArray(messages.pre_messages)}
      onChange={(m) => patch(["messages", "pre_messages"], m)}
      label="Pre-Messages"
    />

    <MessageListEditor
      value={safeMessageArray(messages.post_messages)}
      onChange={(m) => patch(["messages", "post_messages"], m)}
      label="Post-Messages"
    />

    <section className={styles.modelDefaultsSection}>
      <h3 className={styles.sectionTitle}>Prompts</h3>
      {(
        [
          "solver",
          "reviewer",
          "guardrails",
          "gather_system",
          "gather_retry",
        ] as const
      ).map((key) => (
        <Field key={key} label={key.replace("_", " ")}>
          <FieldTextarea
            value={safeString(prompts[key])}
            onChange={(value) => patch(["prompts", key], value || undefined)}
            placeholder={`${key} prompt...`}
            rows={2}
            className={styles.promptTextarea}
          />
        </Field>
      ))}
    </section>
  </>
);

const AdvancedTab: React.FC<{
  base: string | undefined;
  matchModels: string[] | undefined;
  gatherFiles: Record<string, unknown>;
  extraJson: string;
  extraJsonDirty: boolean;
  extraJsonError: string | null;
  onExtraChange: (text: string) => void;
  onExtraApply: () => void;
  patch: PatchFn;
}> = ({
  base,
  matchModels,
  gatherFiles,
  extraJson,
  extraJsonDirty,
  extraJsonError,
  onExtraChange,
  onExtraApply,
  patch,
}) => {
  return (
    <>
      <Field label="Base Subagent">
        <FieldText
          value={base ?? ""}
          onChange={(value) => patch(["base"], value || undefined)}
          placeholder="Inherit from another subagent"
        />
      </Field>

      <StringListEditor
        value={matchModels ?? []}
        onChange={(m) => patch(["match_models"], m.length > 0 ? m : undefined)}
        label="Match Models"
        placeholder="Model pattern..."
      />

      <section className={styles.modelDefaultsSection}>
        <h3 className={styles.sectionTitle}>Gather Files</h3>
        <div className={styles.fieldGrid}>
          <Field label="Subagent">
            <FieldText
              value={safeString(gatherFiles.subagent)}
              onChange={(value) =>
                patch(["gather_files", "subagent"], value || undefined)
              }
              placeholder="Subagent name"
            />
          </Field>
          <Field label="Max Files">
            <FieldText
              type="number"
              value={safeNumber(gatherFiles.max_files)?.toString() ?? ""}
              onChange={(value) =>
                patch(["gather_files", "max_files"], parseIntSafe(value))
              }
              placeholder="Default"
            />
          </Field>
          <Field label="Max Steps">
            <FieldText
              type="number"
              value={safeNumber(gatherFiles.max_steps)?.toString() ?? ""}
              onChange={(value) =>
                patch(["gather_files", "max_steps"], parseIntSafe(value))
              }
              placeholder="Default"
            />
          </Field>
        </div>
      </section>

      <Field
        label="Extra Fields (JSON)"
        helper="Unknown/custom fields at top level."
        error={extraJsonError}
      >
        <div className={styles.extraFieldStack}>
          <FieldTextarea
            value={extraJson}
            onChange={onExtraChange}
            placeholder="{}"
            className={styles.extraFieldsEditor}
          />
          {extraJsonDirty && (
            <Button size="sm" variant="soft" onClick={onExtraApply}>
              Apply
            </Button>
          )}
        </div>
      </Field>
    </>
  );
};
