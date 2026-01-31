import React, { useState, useCallback } from "react";
import {
  Flex,
  TextField,
  Text,
  Switch,
  TextArea,
  Tabs,
} from "@radix-ui/themes";
import { StringListEditor } from "./StringListEditor";
import { RulesTableEditor } from "./RulesTableEditor";
import {
  ConfigPatch,
  safeArray,
  safeString,
  safeBoolean,
  safeObject,
  isString,
  safeToolConfirmRules,
} from "./configUtils";
import styles from "./editors.module.css";

type ModeFormProps = {
  config: Record<string, unknown>;
  onPatch: (patch: ConfigPatch) => void;
  availableTools?: string[];
};

export const ModeForm: React.FC<ModeFormProps> = ({
  config,
  onPatch,
  availableTools = [],
}) => {
  const [activeTab, setActiveTab] = useState("basic");

  const title = safeString(config.title);
  const description = safeString(config.description);
  const specific = safeBoolean(config.specific);
  const prompt = safeString(config.prompt);
  const tools = safeArray(config.tools, isString);
  const llmDefaults = safeObject(config.llm_defaults);
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

  return (
    <Tabs.Root
      value={activeTab}
      onValueChange={setActiveTab}
      style={{
        display: "flex",
        flexDirection: "column",
        flex: 1,
        minHeight: 0,
      }}
    >
      <Tabs.List>
        <Tabs.Trigger value="basic">Basic</Tabs.Trigger>
        <Tabs.Trigger value="tools">Tools</Tabs.Trigger>
        <Tabs.Trigger value="llm">LLM Settings</Tabs.Trigger>
        <Tabs.Trigger value="advanced">Advanced</Tabs.Trigger>
      </Tabs.List>

      {activeTab === "basic" && (
        <div className={styles.formTabContentExpanding}>
          <Flex direction="column" gap="3" style={{ flexShrink: 0 }}>
            <Flex direction="column" gap="1">
              <Text size="1" weight="medium">
                Title
              </Text>
              <TextField.Root
                size="1"
                value={title}
                onChange={(e) => patch(["title"], e.target.value)}
                placeholder="Display name"
              />
            </Flex>

            <Flex direction="column" gap="1">
              <Text size="1" weight="medium">
                Description
              </Text>
              <TextField.Root
                size="1"
                value={description}
                onChange={(e) => patch(["description"], e.target.value)}
                placeholder="Brief description"
              />
            </Flex>

            <Flex align="center" gap="2">
              <Switch
                size="1"
                checked={specific}
                onCheckedChange={(checked) => patch(["specific"], checked)}
              />
              <Text size="1">Internal Only</Text>
              <Text size="1" color="gray">
                (hide from mode selector)
              </Text>
            </Flex>
          </Flex>

          <div className={styles.expandingField}>
            <Text size="1" weight="medium">
              System Prompt
            </Text>
            <TextArea
              value={prompt}
              onChange={(e) => patch(["prompt"], e.target.value)}
              placeholder="System prompt for this mode..."
              className={styles.promptTextareaExpand}
            />
            <Text size="1" color="gray">
              Supports: %PROJECT_TREE%, %WORKSPACE_INFO%, %ARGS%, etc.
            </Text>
          </div>
        </div>
      )}

      {activeTab === "tools" && (
        <div className={styles.formTabContent}>
          <StringListEditor
            value={tools}
            onChange={(t) => patch(["tools"], t)}
            label="Available Tools"
            placeholder="Add tool..."
            suggestions={availableTools}
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
          <Flex gap="2" wrap="wrap">
            <Flex direction="column" gap="1" style={{ flex: 1, minWidth: 80 }}>
              <Text size="1">Max Tokens</Text>
              <TextField.Root
                size="1"
                type="number"
                value={
                  typeof llmDefaults.max_new_tokens === "number"
                    ? llmDefaults.max_new_tokens.toString()
                    : ""
                }
                onChange={(e) =>
                  patch(
                    ["llm_defaults", "max_new_tokens"],
                    e.target.value ? parseInt(e.target.value, 10) : undefined,
                  )
                }
                placeholder="Default"
              />
            </Flex>
            <Flex direction="column" gap="1" style={{ flex: 1, minWidth: 70 }}>
              <Text size="1">Temp</Text>
              <TextField.Root
                size="1"
                type="number"
                step="0.1"
                value={
                  typeof llmDefaults.temperature === "number"
                    ? llmDefaults.temperature.toString()
                    : ""
                }
                onChange={(e) =>
                  patch(
                    ["llm_defaults", "temperature"],
                    e.target.value ? parseFloat(e.target.value) : undefined,
                  )
                }
                placeholder="Default"
              />
            </Flex>
            <Flex direction="column" gap="1" style={{ flex: 1, minWidth: 70 }}>
              <Text size="1">Top P</Text>
              <TextField.Root
                size="1"
                type="number"
                step="0.1"
                value={
                  typeof llmDefaults.top_p === "number"
                    ? llmDefaults.top_p.toString()
                    : ""
                }
                onChange={(e) =>
                  patch(
                    ["llm_defaults", "top_p"],
                    e.target.value ? parseFloat(e.target.value) : undefined,
                  )
                }
                placeholder="Default"
              />
            </Flex>
          </Flex>

          <Flex gap="3" wrap="wrap">
            <Flex align="center" gap="1">
              <Switch
                size="1"
                checked={
                  typeof llmDefaults.boost_reasoning === "boolean"
                    ? llmDefaults.boost_reasoning
                    : false
                }
                onCheckedChange={(checked) =>
                  patch(
                    ["llm_defaults", "boost_reasoning"],
                    checked || undefined,
                  )
                }
              />
              <Text size="1">Boost Reasoning</Text>
            </Flex>
            <Flex align="center" gap="1">
              <Switch
                size="1"
                checked={
                  typeof llmDefaults.parallel_tool_calls === "boolean"
                    ? llmDefaults.parallel_tool_calls
                    : false
                }
                onCheckedChange={(checked) =>
                  patch(
                    ["llm_defaults", "parallel_tool_calls"],
                    checked || undefined,
                  )
                }
              />
              <Text size="1">Parallel Tools</Text>
            </Flex>
          </Flex>

          <Flex gap="2" wrap="wrap">
            <Flex direction="column" gap="1" style={{ flex: 1, minWidth: 100 }}>
              <Text size="1">Reasoning Effort</Text>
              <TextField.Root
                size="1"
                value={
                  typeof llmDefaults.reasoning_effort === "string"
                    ? llmDefaults.reasoning_effort
                    : ""
                }
                onChange={(e) =>
                  patch(
                    ["llm_defaults", "reasoning_effort"],
                    e.target.value || undefined,
                  )
                }
                placeholder="low/medium/high"
              />
            </Flex>
            <Flex direction="column" gap="1" style={{ flex: 1, minWidth: 100 }}>
              <Text size="1">Tool Choice</Text>
              <TextField.Root
                size="1"
                value={
                  typeof llmDefaults.tool_choice === "string"
                    ? llmDefaults.tool_choice
                    : ""
                }
                onChange={(e) =>
                  patch(
                    ["llm_defaults", "tool_choice"],
                    e.target.value || undefined,
                  )
                }
                placeholder="auto/none/required"
              />
            </Flex>
          </Flex>
        </div>
      )}

      {activeTab === "advanced" && (
        <div className={styles.formTabContent}>
          <Flex direction="column" gap="2">
            <Text size="1" weight="medium">
              Thread Defaults
            </Text>
            <Flex gap="3" wrap="wrap">
              <Flex align="center" gap="1">
                <Switch
                  size="1"
                  checked={
                    typeof threadDefaults.include_project_info === "boolean"
                      ? threadDefaults.include_project_info
                      : false
                  }
                  onCheckedChange={(checked) =>
                    patch(
                      ["thread_defaults", "include_project_info"],
                      checked || undefined,
                    )
                  }
                />
                <Text size="1">Project Info</Text>
              </Flex>
              <Flex align="center" gap="1">
                <Switch
                  size="1"
                  checked={
                    typeof threadDefaults.checkpoints_enabled === "boolean"
                      ? threadDefaults.checkpoints_enabled
                      : false
                  }
                  onCheckedChange={(checked) =>
                    patch(
                      ["thread_defaults", "checkpoints_enabled"],
                      checked || undefined,
                    )
                  }
                />
                <Text size="1">Checkpoints</Text>
              </Flex>
              <Flex align="center" gap="1">
                <Switch
                  size="1"
                  checked={
                    typeof threadDefaults.auto_approve_editing_tools === "boolean"
                      ? threadDefaults.auto_approve_editing_tools
                      : false
                  }
                  onCheckedChange={(checked) =>
                    patch(
                      ["thread_defaults", "auto_approve_editing_tools"],
                      checked || undefined,
                    )
                  }
                />
                <Text size="1">Auto Approve Editing</Text>
              </Flex>
            </Flex>
          </Flex>

          <Flex direction="column" gap="1">
            <Text size="1" weight="medium">
              Base Mode
            </Text>
            <TextField.Root
              size="1"
              value={base ?? ""}
              onChange={(e) => patch(["base"], e.target.value || undefined)}
              placeholder="Inherit from (e.g., agent)"
            />
          </Flex>

          <StringListEditor
            value={matchModels ?? []}
            onChange={(models) =>
              patch(["match_models"], models.length > 0 ? models : undefined)
            }
            label="Match Models"
            placeholder="Model pattern..."
          />

          <Flex gap="2" wrap="wrap">
            <Flex direction="column" gap="1" style={{ minWidth: 80 }}>
              <Text size="1">UI Order</Text>
              <TextField.Root
                size="1"
                type="number"
                value={typeof ui.order === "number" ? ui.order.toString() : ""}
                onChange={(e) =>
                  patch(
                    ["ui", "order"],
                    e.target.value ? parseInt(e.target.value, 10) : undefined,
                  )
                }
                placeholder="Order"
              />
            </Flex>
          </Flex>

          <StringListEditor
            value={safeArray(ui.tags, isString)}
            onChange={(tags) => patch(["ui", "tags"], tags)}
            label="UI Tags"
            placeholder="Add tag..."
          />
        </div>
      )}
    </Tabs.Root>
  );
};
