import React, { useState, useCallback, useMemo } from "react";
import { SlidersHorizontal, Wrench, Brain, Settings } from "lucide-react";
import { StringListEditor } from "./StringListEditor";
import { RulesTableEditor } from "./RulesTableEditor";
import { ToolToggleList } from "./ToolToggleList";
import {
  ConfigPatch,
  safeArray,
  safeString,
  safeBoolean,
  safeObject,
  isString,
  safeToolConfirmRules,
  parseIntSafe,
} from "./configUtils";
import { useGetCapsQuery } from "../../../services/refact/caps";
import { useCapsForToolUse } from "../../../hooks";
import { enrichAndGroupModels } from "../../../utils/enrichModels";
import { CapabilityIcons } from "../../Providers/ProviderForm/ProviderModelsList/components";
import {
  formatContextWindow,
  formatPricing,
} from "../../Providers/ProviderForm/ProviderModelsList/utils/groupModelsWithPricing";
import {
  ModelSamplingParams,
  type SamplingValues,
} from "../../../components/ModelSamplingParams";
import {
  Field,
  FieldSwitch,
  FieldText,
  FieldTextarea,
  ModelSelector,
  SettingsShell,
  type ModelOption,
  type ModelSelectorBadge,
  type ModelSelectorGroup,
} from "../../../components/ui";
import styles from "./editors.module.css";

type ModeFormProps = {
  config: Record<string, unknown>;
  onPatch: (patch: ConfigPatch) => void;
};

type EnrichedModelGroup = ReturnType<typeof enrichAndGroupModels>;
type EnrichedModel = EnrichedModelGroup[number]["models"][number];

function modelBadges(model: EnrichedModel) {
  const badges: ModelSelectorBadge[] = [];
  if (model.isDefault) badges.push("default");
  if (model.isThinking) badges.push("reasoning");
  if (model.isLight) badges.push("light");
  if (model.isBuddy) badges.push("buddy");
  if (model.isTaskPlannerAgent) badges.push("task-agent");
  if (model.isChat2) badges.push("chat2");
  return badges;
}

function pricingOption(model: EnrichedModel) {
  if (!model.pricing) return undefined;
  const [prompt, output] = formatPricing(model.pricing, true).split("/");
  return { prompt, output };
}

function modelGroups(groupedModels: EnrichedModelGroup): ModelSelectorGroup[] {
  return groupedModels.map((group) => ({
    id: group.provider,
    label: group.displayName,
  }));
}

function modelOptions(groupedModels: EnrichedModelGroup): ModelOption[] {
  return groupedModels.flatMap((group) =>
    group.models.map((model) => ({
      value: model.value,
      displayName: model.value,
      group: group.provider,
      disabled: model.disabled,
      pricing: pricingOption(model),
      contextWindow: model.nCtx ? formatContextWindow(model.nCtx) : undefined,
      badges: modelBadges(model),
      capabilities: model.capabilities ? (
        <CapabilityIcons capabilities={model.capabilities} />
      ) : undefined,
    })),
  );
}

type ModelTypeSectionProps = {
  title: string;
  typeKey: "default" | "light" | "thinking";
  config: Record<string, unknown>;
  groupedModels: EnrichedModelGroup;
  onPatch: (path: (string | number)[], value: unknown) => void;
};

const ModelTypeSection: React.FC<ModelTypeSectionProps> = ({
  title,
  typeKey,
  config,
  groupedModels,
  onPatch,
}) => {
  const model = safeString(config.model);
  const toolChoice =
    typeof config.tool_choice === "string" ? config.tool_choice : "";
  const parallelToolCalls =
    typeof config.parallel_tool_calls === "boolean"
      ? config.parallel_tool_calls
      : false;

  const basePath = useMemo(
    () => ["model_defaults", typeKey] as const,
    [typeKey],
  );

  const samplingValues: SamplingValues = useMemo(
    () => ({
      max_new_tokens:
        typeof config.max_new_tokens === "number"
          ? config.max_new_tokens
          : undefined,
      top_p: typeof config.top_p === "number" ? config.top_p : undefined,
      boost_reasoning:
        typeof config.boost_reasoning === "boolean"
          ? config.boost_reasoning
          : undefined,
      reasoning_effort:
        typeof config.reasoning_effort === "string"
          ? config.reasoning_effort
          : undefined,
      thinking_budget:
        typeof config.thinking_budget === "number"
          ? config.thinking_budget
          : undefined,
    }),
    [config],
  );

  const handleSamplingChange = useCallback(
    <K extends keyof SamplingValues>(field: K, value: SamplingValues[K]) => {
      onPatch([...basePath, field], value);
    },
    [onPatch, basePath],
  );

  return (
    <section className={styles.modelDefaultsSection}>
      <h3 className={styles.sectionTitle}>{title}</h3>
      <Field label="Model">
        <ModelSelector
          allowUnset
          groups={modelGroups(groupedModels)}
          models={modelOptions(groupedModels)}
          unsetLabel="Inherit from global"
          value={model || null}
          onSelect={(v) =>
            onPatch([...basePath, "model"], v === "" ? undefined : v)
          }
        />
      </Field>

      <ModelSamplingParams
        model={model || undefined}
        values={samplingValues}
        onChange={handleSamplingChange}
      />

      <div className={styles.switchGrid}>
        <Field label="Parallel Tool Calls">
          <FieldSwitch
            checked={parallelToolCalls}
            onChange={(checked) =>
              onPatch(
                [...basePath, "parallel_tool_calls"],
                checked || undefined,
              )
            }
          />
        </Field>
        <Field label="Tool Choice">
          <FieldText
            value={toolChoice}
            placeholder="auto/none"
            onChange={(value) =>
              onPatch([...basePath, "tool_choice"], value || undefined)
            }
          />
        </Field>
      </div>
    </section>
  );
};

const MODE_SECTIONS = [
  { id: "basic", label: "Basic", icon: SlidersHorizontal },
  { id: "tools", label: "Tools", icon: Wrench },
  { id: "llm", label: "LLM Settings", icon: Brain },
  { id: "advanced", label: "Advanced", icon: Settings },
];

export const ModeForm: React.FC<ModeFormProps> = ({ config, onPatch }) => {
  const [activeTab, setActiveTab] = useState("basic");

  const title = safeString(config.title);
  const description = safeString(config.description);
  const specific = safeBoolean(config.specific);
  const prompt = safeString(config.prompt);
  const tools = safeArray(config.tools, isString);
  const allowIntegrations = safeBoolean(config.allow_integrations);
  const allowMcp = safeBoolean(config.allow_mcp);
  const allowSubagents = safeBoolean(config.allow_subagents);
  const modelDefaults = safeObject(config.model_defaults);
  const modelDefaultsDefault = safeObject(modelDefaults.default);
  const modelDefaultsLight = safeObject(modelDefaults.light);
  const modelDefaultsThinking = safeObject(modelDefaults.thinking);
  const toolConfirmObj = safeObject(config.tool_confirm);
  const toolConfirmRules = safeToolConfirmRules(toolConfirmObj.rules);
  const threadDefaults = safeObject(config.thread_defaults);
  const ui = safeObject(config.ui);
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

  const { data: capsData } = useGetCapsQuery(undefined);
  const capsForToolUse = useCapsForToolUse();

  const groupedModels = useMemo(() => {
    return enrichAndGroupModels(capsForToolUse.usableModelsForPlan, capsData);
  }, [capsForToolUse.usableModelsForPlan, capsData]);

  return (
    <SettingsShell
      active={activeTab}
      sections={MODE_SECTIONS}
      title="Mode"
      description="Edit mode identity, tools, model defaults, and advanced matching."
      onSectionChange={setActiveTab}
    >
      {activeTab === "basic" && (
        <div className={styles.formTabContentExpanding}>
          <div className={styles.formStackShrink}>
            <Field label="Title">
              <FieldText
                value={title}
                onChange={(value) => patch(["title"], value)}
                placeholder="Display name"
              />
            </Field>

            <Field label="Description">
              <FieldText
                value={description}
                onChange={(value) => patch(["description"], value)}
                placeholder="Brief description"
              />
            </Field>

            <Field label="Internal Only" helper="Hide from mode selector.">
              <FieldSwitch
                checked={specific}
                onChange={(checked) => patch(["specific"], checked)}
              />
            </Field>
          </div>

          <Field
            label="System Prompt"
            helper="Supports: %PROJECT_TREE%, %WORKSPACE_INFO%, %ARGS%, etc."
            className={styles.expandingField}
          >
            <FieldTextarea
              value={prompt}
              onChange={(value) => patch(["prompt"], value)}
              placeholder="System prompt for this mode..."
              className={styles.promptTextareaExpand}
            />
          </Field>
        </div>
      )}

      {activeTab === "tools" && (
        <div className={styles.formTabContent}>
          <div className={styles.switchGrid}>
            <Field
              label="Integrations"
              helper="Automatically include integrations."
            >
              <FieldSwitch
                checked={allowIntegrations}
                onChange={(checked) =>
                  patch(["allow_integrations"], checked || undefined)
                }
              />
            </Field>
            <Field label="MCP" helper="Automatically include MCP tools.">
              <FieldSwitch
                checked={allowMcp}
                onChange={(checked) =>
                  patch(["allow_mcp"], checked || undefined)
                }
              />
            </Field>
            <Field label="Subagents" helper="Automatically include subagents.">
              <FieldSwitch
                checked={allowSubagents}
                onChange={(checked) =>
                  patch(["allow_subagents"], checked || undefined)
                }
              />
            </Field>
          </div>

          <ToolToggleList
            enabledTools={tools}
            onChange={(t) => patch(["tools"], t)}
          />

          <RulesTableEditor
            value={toolConfirmRules}
            onChange={(rules) => patch(["tool_confirm", "rules"], rules)}
            label="Tool Confirmation Rules"
          />
        </div>
      )}

      {activeTab === "llm" && (
        <div className={styles.formTabContent}>
          <ModelTypeSection
            title="Default Model"
            typeKey="default"
            config={modelDefaultsDefault}
            groupedModels={groupedModels}
            onPatch={patch}
          />
          <ModelTypeSection
            title="Light Model"
            typeKey="light"
            config={modelDefaultsLight}
            groupedModels={groupedModels}
            onPatch={patch}
          />
          <ModelTypeSection
            title="Thinking Model"
            typeKey="thinking"
            config={modelDefaultsThinking}
            groupedModels={groupedModels}
            onPatch={patch}
          />
        </div>
      )}

      {activeTab === "advanced" && (
        <div className={styles.formTabContent}>
          <div className={styles.switchGrid}>
            <Field label="Project Info">
              <FieldSwitch
                checked={
                  typeof threadDefaults.include_project_info === "boolean"
                    ? threadDefaults.include_project_info
                    : false
                }
                onChange={(checked) =>
                  patch(
                    ["thread_defaults", "include_project_info"],
                    checked || undefined,
                  )
                }
              />
            </Field>
            <Field label="Checkpoints">
              <FieldSwitch
                checked={
                  typeof threadDefaults.checkpoints_enabled === "boolean"
                    ? threadDefaults.checkpoints_enabled
                    : false
                }
                onChange={(checked) =>
                  patch(
                    ["thread_defaults", "checkpoints_enabled"],
                    checked || undefined,
                  )
                }
              />
            </Field>
            <Field label="Auto Approve Editing">
              <FieldSwitch
                checked={
                  typeof threadDefaults.auto_approve_editing_tools === "boolean"
                    ? threadDefaults.auto_approve_editing_tools
                    : false
                }
                onChange={(checked) =>
                  patch(
                    ["thread_defaults", "auto_approve_editing_tools"],
                    checked || undefined,
                  )
                }
              />
            </Field>
            <Field label="Auto Approve Dangerous">
              <FieldSwitch
                checked={
                  typeof threadDefaults.auto_approve_dangerous_commands ===
                  "boolean"
                    ? threadDefaults.auto_approve_dangerous_commands
                    : false
                }
                onChange={(checked) =>
                  patch(
                    ["thread_defaults", "auto_approve_dangerous_commands"],
                    checked || undefined,
                  )
                }
              />
            </Field>
          </div>

          <Field label="Base Mode">
            <FieldText
              value={base ?? ""}
              onChange={(value) => patch(["base"], value || undefined)}
              placeholder="Inherit from (e.g., agent)"
            />
          </Field>

          <StringListEditor
            value={matchModels ?? []}
            onChange={(models) =>
              patch(["match_models"], models.length > 0 ? models : undefined)
            }
            label="Match Models"
            placeholder="Model pattern..."
          />

          <Field label="UI Order">
            <FieldText
              type="number"
              value={typeof ui.order === "number" ? ui.order.toString() : ""}
              onChange={(value) => patch(["ui", "order"], parseIntSafe(value))}
              placeholder="Order"
            />
          </Field>

          <StringListEditor
            value={safeArray(ui.tags, isString)}
            onChange={(tags) => patch(["ui", "tags"], tags)}
            label="UI Tags"
            placeholder="Add tag..."
          />
        </div>
      )}
    </SettingsShell>
  );
};
