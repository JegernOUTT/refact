import React, { useState, useCallback, useEffect } from "react";
import {
  Flex,
  TextField,
  Text,
  Switch,
  TextArea,
  Tabs,
  Button,
} from "@radix-ui/themes";
import { StringListEditor } from "./StringListEditor";
import { ToolParametersEditor, ToolParameter } from "./ToolParametersEditor";
import { toInputSchema, fromInputSchema } from "../../../utils/toolSchema";
import { MessageListEditor } from "./MessageListEditor";
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
    <Tabs.Root value={activeTab} onValueChange={setActiveTab}>
      <Tabs.List>
        <Tabs.Trigger value="basic">Basic</Tabs.Trigger>
        <Tabs.Trigger value="tool">Tool Schema</Tabs.Trigger>
        <Tabs.Trigger value="subchat">Subchat</Tabs.Trigger>
        <Tabs.Trigger value="messages">Messages</Tabs.Trigger>
        <Tabs.Trigger value="advanced">Advanced</Tabs.Trigger>
      </Tabs.List>

      <Flex direction="column" gap="4" pt="4">
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
      </Flex>
    </Tabs.Root>
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
    <Flex direction="column" gap="2">
      <Text size="2" weight="medium">
        Title
      </Text>
      <TextField.Root
        value={title}
        onChange={(e) => patch(["title"], e.target.value)}
        placeholder="Display name"
      />
    </Flex>

    <Flex direction="column" gap="2">
      <Text size="2" weight="medium">
        Description
      </Text>
      <TextArea
        value={description}
        onChange={(e) => patch(["description"], e.target.value)}
        placeholder="What this subagent does..."
        rows={2}
      />
    </Flex>

    <Flex gap="4" wrap="wrap">
      <Flex align="center" gap="2">
        <Switch
          checked={specific}
          onCheckedChange={(c) => patch(["specific"], c)}
        />
        <Text size="2">Internal Only</Text>
      </Flex>
      <Flex align="center" gap="2">
        <Switch
          checked={exposeAsTool}
          onCheckedChange={(c) => patch(["expose_as_tool"], c)}
        />
        <Text size="2">Expose as Tool</Text>
      </Flex>
      <Flex align="center" gap="2">
        <Switch
          checked={hasCode}
          onCheckedChange={(c) => patch(["has_code"], c)}
        />
        <Text size="2">Has Code</Text>
      </Flex>
    </Flex>

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
      <Flex align="center" gap="2">
        <Switch
          checked={hasTool}
          onCheckedChange={(checked) => {
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
        <Text size="2">Define Custom Tool Schema</Text>
      </Flex>

      {hasTool && (
        <>
          <Flex direction="column" gap="2">
            <Text size="2" weight="medium">
              Tool Description
            </Text>
            <TextArea
              value={toolDesc}
              onChange={(e) => patch(["tool", "description"], e.target.value)}
              placeholder="Description shown to the LLM..."
              rows={2}
            />
          </Flex>

          <Flex align="center" gap="2">
            <Switch
              checked={agentic}
              onCheckedChange={(c) => patch(["tool", "agentic"], c)}
            />
            <Text size="2">Agentic</Text>
            <Text size="1" color="gray">
              (tool can make multiple calls)
            </Text>
          </Flex>

          <Flex align="center" gap="2">
            <Switch
              checked={allowParallel}
              onCheckedChange={(c) => patch(["tool", "allow_parallel"], c)}
            />
            <Text size="2">Allow Parallel</Text>
            <Text size="1" color="gray">
              (tool can run concurrently with other parallel tools)
            </Text>
          </Flex>

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
      <Flex gap="4">
        <Flex direction="column" gap="2" style={{ flex: 1 }}>
          <Text size="2" weight="medium">
            Context Mode
          </Text>
          <TextField.Root
            value={safeString(subchat.context_mode) || "bare"}
            onChange={(e) => patch(["subchat", "context_mode"], e.target.value)}
            placeholder="bare / full / ..."
          />
        </Flex>
        <Flex direction="column" gap="2" style={{ flex: 1 }}>
          <Text size="2" weight="medium">
            Model
          </Text>
          <TextField.Root
            value={safeString(subchat.model)}
            onChange={(e) =>
              patch(["subchat", "model"], e.target.value || undefined)
            }
            placeholder="Default"
          />
        </Flex>
        <Flex direction="column" gap="2" style={{ flex: 1 }}>
          <Text size="2" weight="medium">
            Model Type
          </Text>
          <TextField.Root
            value={safeString(subchat.model_type)}
            onChange={(e) =>
              patch(["subchat", "model_type"], e.target.value || undefined)
            }
            placeholder="Default"
          />
        </Flex>
      </Flex>

      <Flex align="center" gap="2">
        <Switch
          checked={safeBoolean(subchat.stateful)}
          onCheckedChange={(c) => patch(["subchat", "stateful"], c)}
        />
        <Text size="2">Stateful</Text>
      </Flex>

      <Flex gap="4">
        <Flex direction="column" gap="2" style={{ flex: 1 }}>
          <Text size="2" weight="medium">
            Max Steps
          </Text>
          <TextField.Root
            type="number"
            value={safeNumber(subchat.max_steps)?.toString() ?? ""}
            onChange={(e) =>
              patch(["subchat", "max_steps"], parseIntSafe(e.target.value))
            }
            placeholder="Default"
          />
        </Flex>
        <Flex direction="column" gap="2" style={{ flex: 1 }}>
          <Text size="2" weight="medium">
            N Context
          </Text>
          <TextField.Root
            type="number"
            value={safeNumber(subchat.n_ctx)?.toString() ?? ""}
            onChange={(e) =>
              patch(["subchat", "n_ctx"], parseIntSafe(e.target.value))
            }
            placeholder="Default"
          />
        </Flex>
        <Flex direction="column" gap="2" style={{ flex: 1 }}>
          <Text size="2" weight="medium">
            Max New Tokens
          </Text>
          <TextField.Root
            type="number"
            value={safeNumber(subchat.max_new_tokens)?.toString() ?? ""}
            onChange={(e) =>
              patch(["subchat", "max_new_tokens"], parseIntSafe(e.target.value))
            }
            placeholder="Default"
          />
        </Flex>
      </Flex>

      <Flex gap="4">
        <Flex direction="column" gap="2" style={{ flex: 1 }}>
          <Text size="2" weight="medium">
            Reasoning Effort
          </Text>
          <TextField.Root
            value={safeString(subchat.reasoning_effort)}
            onChange={(e) =>
              patch(
                ["subchat", "reasoning_effort"],
                e.target.value || undefined,
              )
            }
            placeholder="low / medium / high / xhigh / max"
          />
        </Flex>
        <Flex direction="column" gap="2" style={{ flex: 1 }}>
          <Text size="2" weight="medium">
            Tokens for RAG
          </Text>
          <TextField.Root
            type="number"
            value={safeNumber(subchat.tokens_for_rag)?.toString() ?? ""}
            onChange={(e) =>
              patch(["subchat", "tokens_for_rag"], parseIntSafe(e.target.value))
            }
            placeholder="Default"
          />
        </Flex>
      </Flex>
    </>
  );
};

const MessagesTab: React.FC<{
  messages: Record<string, unknown>;
  prompts: Record<string, unknown>;
  patch: PatchFn;
}> = ({ messages, prompts, patch }) => (
  <>
    <Flex direction="column" gap="2">
      <Text size="2" weight="medium">
        System Prompt
      </Text>
      <TextArea
        value={safeString(messages.system_prompt)}
        onChange={(e) =>
          patch(["messages", "system_prompt"], e.target.value || undefined)
        }
        placeholder="System prompt..."
        className={styles.promptTextarea}
      />
    </Flex>

    <Flex direction="column" gap="2">
      <Text size="2" weight="medium">
        User Template
      </Text>
      <TextArea
        value={safeString(messages.user_template)}
        onChange={(e) =>
          patch(["messages", "user_template"], e.target.value || undefined)
        }
        placeholder="User message template..."
        rows={3}
      />
    </Flex>

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

    <Text size="2" weight="medium" mt="2">
      Prompts
    </Text>
    {(
      [
        "solver",
        "reviewer",
        "guardrails",
        "gather_system",
        "gather_retry",
      ] as const
    ).map((key) => (
      <Flex key={key} direction="column" gap="1">
        <Text size="1" color="gray">
          {key.replace("_", " ")}
        </Text>
        <TextArea
          value={safeString(prompts[key])}
          onChange={(e) => patch(["prompts", key], e.target.value || undefined)}
          placeholder={`${key} prompt...`}
          rows={2}
        />
      </Flex>
    ))}
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
      <Flex direction="column" gap="2">
        <Text size="2" weight="medium">
          Base Subagent
        </Text>
        <TextField.Root
          value={base ?? ""}
          onChange={(e) => patch(["base"], e.target.value || undefined)}
          placeholder="Inherit from another subagent"
        />
      </Flex>

      <StringListEditor
        value={matchModels ?? []}
        onChange={(m) => patch(["match_models"], m.length > 0 ? m : undefined)}
        label="Match Models"
        placeholder="Model pattern..."
      />

      <Text size="2" weight="medium">
        Gather Files
      </Text>
      <Flex gap="4">
        <Flex direction="column" gap="2" style={{ flex: 1 }}>
          <Text size="1" color="gray">
            Subagent
          </Text>
          <TextField.Root
            value={safeString(gatherFiles.subagent)}
            onChange={(e) =>
              patch(["gather_files", "subagent"], e.target.value || undefined)
            }
            placeholder="Subagent name"
          />
        </Flex>
        <Flex direction="column" gap="2" style={{ flex: 1 }}>
          <Text size="1" color="gray">
            Max Files
          </Text>
          <TextField.Root
            type="number"
            value={safeNumber(gatherFiles.max_files)?.toString() ?? ""}
            onChange={(e) =>
              patch(["gather_files", "max_files"], parseIntSafe(e.target.value))
            }
            placeholder="Default"
          />
        </Flex>
        <Flex direction="column" gap="2" style={{ flex: 1 }}>
          <Text size="1" color="gray">
            Max Steps
          </Text>
          <TextField.Root
            type="number"
            value={safeNumber(gatherFiles.max_steps)?.toString() ?? ""}
            onChange={(e) =>
              patch(["gather_files", "max_steps"], parseIntSafe(e.target.value))
            }
            placeholder="Default"
          />
        </Flex>
      </Flex>

      <Flex direction="column" gap="2">
        <Flex justify="between" align="center">
          <Text size="2" weight="medium">
            Extra Fields (JSON)
          </Text>
          {extraJsonDirty && (
            <Button size="1" variant="soft" onClick={onExtraApply}>
              Apply
            </Button>
          )}
        </Flex>
        <Text size="1" color="gray">
          Unknown/custom fields at top level
        </Text>
        <TextArea
          value={extraJson}
          onChange={(e) => onExtraChange(e.target.value)}
          placeholder="{}"
          className={styles.extraFieldsEditor}
        />
        {extraJsonError && (
          <Text size="1" color="red">
            {extraJsonError}
          </Text>
        )}
      </Flex>
    </>
  );
};
