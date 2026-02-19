import React, { useCallback, useEffect, useMemo, useState } from "react";
import {
  Dialog,
  Flex,
  Text,
  Button,
  Switch,
  ScrollArea,
  Slider,
  Callout,
  Separator,
  Badge,
  IconButton,
  Code,
} from "@radix-ui/themes";
import {
  ExclamationTriangleIcon,
  CheckCircledIcon,
  EyeOpenIcon,
  Cross2Icon,
} from "@radix-ui/react-icons";
import {
  useGetProjectInformationQuery,
  useSaveProjectInformationMutation,
  useGetProjectInformationPreviewMutation,
  ProjectInformationConfig,
  ProjectInfoBlock,
  defaultProjectInformationConfig,
  SectionConfig,
} from "../../services/refact/projectInformation";
import { useAppDispatch, useAppSelector } from "../../hooks";
import { selectCurrentThreadId } from "../../features/Chat";
import { setIncludeProjectInfo } from "../../features/Chat/Thread/actions";

type Props = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
};

type SectionMeta = {
  label: string;
  field: "max_chars" | "max_chars_per_item" | "max_items";
  minTokens: number;
  maxTokens: number;
  stepTokens: number;
};

const SECTION_META: Record<string, SectionMeta> = {
  system_info: {
    label: "System Information",
    field: "max_chars",
    minTokens: 100,
    maxTokens: 2000,
    stepTokens: 100,
  },
  environment_instructions: {
    label: "Environment Instructions",
    field: "max_chars",
    minTokens: 250,
    maxTokens: 4000,
    stepTokens: 250,
  },
  detected_environments: {
    label: "Detected Environments",
    field: "max_items",
    minTokens: 5,
    maxTokens: 100,
    stepTokens: 5,
  },
  git_info: {
    label: "Git Information",
    field: "max_chars",
    minTokens: 250,
    maxTokens: 4000,
    stepTokens: 250,
  },
  project_tree: {
    label: "Project Tree",
    field: "max_chars",
    minTokens: 500,
    maxTokens: 16000,
    stepTokens: 500,
  },
  instruction_files: {
    label: "Instruction Files (AGENTS.md, etc.)",
    field: "max_chars_per_item",
    minTokens: 250,
    maxTokens: 16000,
    stepTokens: 500,
  },
  project_configs: {
    label: "Project Configs (.refact/)",
    field: "max_chars_per_item",
    minTokens: 250,
    maxTokens: 8000,
    stepTokens: 250,
  },
  memories: {
    label: "Memories",
    field: "max_chars_per_item",
    minTokens: 100,
    maxTokens: 8000,
    stepTokens: 250,
  },
};

const truncatePath = (path: string, maxLen = 50): string => {
  if (path.length <= maxLen) return path;
  const parts = path.split("/");
  if (parts.length <= 2) return "..." + path.slice(-maxLen + 3);
  const filename = parts[parts.length - 1];
  const parent = parts[parts.length - 2];
  const suffix = `${parent}/${filename}`;
  if (suffix.length >= maxLen - 3) return "..." + suffix.slice(-maxLen + 3);
  return ".../" + suffix;
};

const CHARS_PER_TOKEN = 4;
const charsToTokens = (chars: number): number =>
  Math.ceil(chars / CHARS_PER_TOKEN);
const tokensToChars = (tokens: number): number => tokens * CHARS_PER_TOKEN;

type ContentPreviewProps = {
  block: ProjectInfoBlock | null;
  onClose: () => void;
};

const ContentPreviewDialog: React.FC<ContentPreviewProps> = ({
  block,
  onClose,
}) => {
  if (!block) return null;

  const isTruncated = block.truncated && block.original_char_count;
  const originalTokens =
    isTruncated && block.original_char_count
      ? charsToTokens(block.original_char_count)
      : charsToTokens(block.char_count);
  const truncatedTokens = charsToTokens(block.char_count);

  return (
    <Dialog.Root open={!!block} onOpenChange={(open) => !open && onClose()}>
      <Dialog.Content
        maxWidth="800px"
        style={{ maxHeight: "80vh", overflow: "hidden" }}
      >
        <Flex justify="between" align="center" mb="3">
          <Dialog.Title style={{ margin: 0 }}>
            {block.path ?? block.title}
          </Dialog.Title>
          <IconButton variant="ghost" onClick={onClose}>
            <Cross2Icon />
          </IconButton>
        </Flex>

        <Flex gap="2" mb="3" wrap="wrap">
          <Badge color="blue">
            {isTruncated
              ? `${originalTokens.toLocaleString()} → ${truncatedTokens.toLocaleString()} tokens`
              : `~${truncatedTokens.toLocaleString()} tokens`}
          </Badge>
          {isTruncated && <Badge color="orange">Truncated</Badge>}
          <Badge color="gray">{block.section}</Badge>
        </Flex>

        <ScrollArea style={{ maxHeight: "calc(80vh - 150px)" }}>
          <Code
            size="1"
            style={{
              display: "block",
              whiteSpace: "pre-wrap",
              wordBreak: "break-word",
              padding: "var(--space-3)",
              backgroundColor: "var(--gray-2)",
              borderRadius: "var(--radius-2)",
            }}
          >
            {block.content || "(empty)"}
          </Code>
        </ScrollArea>

        <Flex justify="end" mt="3">
          <Button type="button" variant="soft" onClick={onClose}>
            Close
          </Button>
        </Flex>
      </Dialog.Content>
    </Dialog.Root>
  );
};

type SectionRowProps = {
  sectionKey: string;
  config: SectionConfig;
  blocks: ProjectInfoBlock[];
  onToggle: (enabled: boolean) => void;
  onFieldChange: (field: string, value: number) => void;
  onFileToggle?: (path: string, enabled: boolean) => void;
  onPreviewBlock?: (block: ProjectInfoBlock) => void;
};

const SECTIONS_WITH_FILE_TOGGLES = ["instruction_files", "memories"];

const SectionRow: React.FC<SectionRowProps> = ({
  sectionKey,
  config,
  blocks,
  onToggle,
  onFieldChange,
  onFileToggle,
  onPreviewBlock,
}) => {
  const meta = SECTION_META[sectionKey];
  const allSectionBlocks = blocks.filter((b) => b.section === sectionKey);
  const enabledBlocks = allSectionBlocks.filter((b) => b.enabled);
  const totalChars = enabledBlocks.reduce((sum, b) => sum + b.char_count, 0);
  const tokens = charsToTokens(totalChars);

  const isItemsField = meta.field === "max_items";
  const currentChars = config[meta.field] ?? tokensToChars(meta.maxTokens / 2);
  const currentTokens = isItemsField
    ? currentChars
    : charsToTokens(currentChars);
  const fieldLabel = isItemsField ? "Max items" : "Max tokens";
  const showFileToggles =
    SECTIONS_WITH_FILE_TOGGLES.includes(sectionKey) &&
    allSectionBlocks.length > 0 &&
    allSectionBlocks[0].path;

  const handleSliderChange = (tokenValue: number) => {
    const charValue = isItemsField ? tokenValue : tokensToChars(tokenValue);
    onFieldChange(meta.field, charValue);
  };

  return (
    <Flex direction="column" gap="2" py="2">
      <Flex align="center" justify="between">
        <Flex align="center" gap="2">
          <Switch
            size="1"
            checked={config.enabled}
            onCheckedChange={onToggle}
          />
          <Text size="2" weight="medium">
            {meta.label}
          </Text>
        </Flex>
        <Badge color={config.enabled ? "blue" : "gray"} size="1">
          ~{tokens.toLocaleString()} tokens
        </Badge>
      </Flex>
      {config.enabled && (
        <Flex direction="column" gap="1" pl="6">
          <Flex align="center" gap="2">
            <Text size="1" color="gray">
              {fieldLabel}:
            </Text>
            <Slider
              size="1"
              value={[currentTokens]}
              min={meta.minTokens}
              max={meta.maxTokens}
              step={meta.stepTokens}
              onValueChange={([v]) => handleSliderChange(v)}
              style={{ width: 120 }}
            />
            <Text size="1" color="gray">
              {currentTokens.toLocaleString()}
            </Text>
          </Flex>
          {allSectionBlocks.length > 0 && (
            <Flex align="center" gap="2">
              <Text size="1" color="gray">
                {enabledBlocks.length}/{allSectionBlocks.length} item(s), ~
                {tokens.toLocaleString()} tokens
              </Text>
              {!showFileToggles &&
                allSectionBlocks.length === 1 &&
                onPreviewBlock && (
                  <IconButton
                    size="1"
                    variant="ghost"
                    onClick={() => onPreviewBlock(allSectionBlocks[0])}
                    title="View content"
                  >
                    <EyeOpenIcon />
                  </IconButton>
                )}
            </Flex>
          )}
          {showFileToggles && onFileToggle && (
            <Flex
              direction="column"
              gap="1"
              mt="2"
              style={{ maxWidth: "100%", overflow: "hidden" }}
            >
              {allSectionBlocks.map((block) => (
                <Flex
                  key={block.id}
                  align="center"
                  gap="2"
                  style={{
                    opacity: block.enabled ? 1 : 0.6,
                    minWidth: 0,
                  }}
                >
                  <Switch
                    size="1"
                    checked={block.enabled}
                    onCheckedChange={(checked) => {
                      if (block.path) {
                        onFileToggle(block.path, checked);
                      }
                    }}
                    style={{ flexShrink: 0 }}
                  />
                  <Text
                    size="1"
                    style={{
                      flex: 1,
                      minWidth: 0,
                      overflow: "hidden",
                      textOverflow: "ellipsis",
                      whiteSpace: "nowrap",
                    }}
                    title={block.path ?? block.title}
                  >
                    {truncatePath(block.path ?? block.title, 45)}
                  </Text>
                  <Text
                    size="1"
                    color="gray"
                    style={{ flexShrink: 0, whiteSpace: "nowrap" }}
                  >
                    {block.original_char_count
                      ? `${charsToTokens(
                          block.original_char_count,
                        ).toLocaleString()}→${charsToTokens(
                          block.char_count,
                        ).toLocaleString()}`
                      : `~${charsToTokens(
                          block.char_count,
                        ).toLocaleString()}`}{" "}
                    tok
                  </Text>
                  {onPreviewBlock && (
                    <IconButton
                      size="1"
                      variant="ghost"
                      onClick={() => onPreviewBlock(block)}
                      title="View content"
                      style={{ flexShrink: 0 }}
                    >
                      <EyeOpenIcon />
                    </IconButton>
                  )}
                </Flex>
              ))}
            </Flex>
          )}
        </Flex>
      )}
    </Flex>
  );
};

export const ProjectInformationDialog: React.FC<Props> = ({
  open,
  onOpenChange,
}) => {
  const dispatch = useAppDispatch();
  const chatId = useAppSelector(selectCurrentThreadId);
  const { data: savedConfig, isLoading } = useGetProjectInformationQuery(
    undefined,
    {
      skip: !open,
    },
  );
  const [saveConfig, { isLoading: isSaving }] =
    useSaveProjectInformationMutation();
  const [triggerPreview, { data: previewData, isLoading: isPreviewing }] =
    useGetProjectInformationPreviewMutation();

  const [localConfig, setLocalConfig] = useState<ProjectInformationConfig>(
    defaultProjectInformationConfig,
  );
  const [saveError, setSaveError] = useState<string | null>(null);
  const [saveSuccess, setSaveSuccess] = useState(false);
  const [previewBlock, setPreviewBlock] = useState<ProjectInfoBlock | null>(
    null,
  );

  useEffect(() => {
    if (savedConfig) {
      setLocalConfig(savedConfig);
    }
  }, [savedConfig]);

  useEffect(() => {
    if (!open) {
      setSaveError(null);
      setSaveSuccess(false);
    }
  }, [open]);

  useEffect(() => {
    if (open && localConfig.enabled) {
      const timeoutId = setTimeout(() => {
        void triggerPreview(localConfig);
      }, 300);
      return () => clearTimeout(timeoutId);
    }
  }, [open, localConfig, triggerPreview]);

  const blocks = useMemo(
    () => previewData?.blocks ?? [],
    [previewData?.blocks],
  );

  const totalTokens = useMemo(() => {
    if (!localConfig.enabled) return 0;
    const enabledBlocks = blocks.filter((b) => b.enabled);
    const totalChars = enabledBlocks.reduce((sum, b) => sum + b.char_count, 0);
    return charsToTokens(totalChars);
  }, [blocks, localConfig.enabled]);

  const updateSection = useCallback(
    (
      sectionKey: keyof ProjectInformationConfig["sections"],
      updates: Partial<SectionConfig>,
    ) => {
      setLocalConfig((prev) => ({
        ...prev,
        sections: {
          ...prev.sections,
          [sectionKey]: {
            ...prev.sections[sectionKey],
            ...updates,
          },
        },
      }));
    },
    [],
  );

  const updateFileOverride = useCallback(
    (
      sectionKey: keyof ProjectInformationConfig["sections"],
      path: string,
      enabled: boolean,
    ) => {
      setLocalConfig((prev) => {
        const section = prev.sections[sectionKey];
        const currentOverrides = section.overrides ?? {};
        const currentOverride =
          (currentOverrides[path] as Record<string, unknown> | undefined) ?? {};
        return {
          ...prev,
          sections: {
            ...prev.sections,
            [sectionKey]: {
              ...section,
              overrides: {
                ...currentOverrides,
                [path]: {
                  ...currentOverride,
                  enabled,
                },
              },
            },
          },
        };
      });
    },
    [],
  );

  const handleSave = useCallback(async () => {
    setSaveError(null);
    setSaveSuccess(false);
    try {
      await saveConfig(localConfig).unwrap();
      setSaveSuccess(true);
      setTimeout(() => onOpenChange(false), 500);
    } catch (err) {
      setSaveError(
        err instanceof Error ? err.message : "Failed to save configuration",
      );
    }
  }, [saveConfig, localConfig, onOpenChange]);

  const handleReset = useCallback(() => {
    setLocalConfig(defaultProjectInformationConfig);
  }, []);

  if (isLoading) {
    return (
      <Dialog.Root open={open} onOpenChange={onOpenChange}>
        <Dialog.Content maxWidth="600px">
          <Dialog.Title>Project Information</Dialog.Title>
          <Flex align="center" justify="center" py="6">
            <Text color="gray">Loading...</Text>
          </Flex>
        </Dialog.Content>
      </Dialog.Root>
    );
  }

  return (
    <Dialog.Root open={open} onOpenChange={onOpenChange}>
      <Dialog.Content maxWidth="600px" style={{ overflow: "hidden" }}>
        <Dialog.Title>Project Information</Dialog.Title>
        <Dialog.Description size="2" color="gray" mb="4">
          Configure what project information is included in chat context. Token
          counts are approximate (~4 chars/token).
        </Dialog.Description>

        {saveError && (
          <Callout.Root color="red" mb="3">
            <Callout.Icon>
              <ExclamationTriangleIcon />
            </Callout.Icon>
            <Callout.Text>{saveError}</Callout.Text>
          </Callout.Root>
        )}

        {saveSuccess && (
          <Callout.Root color="green" mb="3">
            <Callout.Icon>
              <CheckCircledIcon />
            </Callout.Icon>
            <Callout.Text>Configuration saved!</Callout.Text>
          </Callout.Root>
        )}

        <Flex align="center" justify="between" mb="3">
          <Flex align="center" gap="2">
            <Switch
              checked={localConfig.enabled}
              onCheckedChange={(enabled) => {
                setLocalConfig((prev) => ({ ...prev, enabled }));
                if (chatId) {
                  dispatch(setIncludeProjectInfo({ chatId, value: enabled }));
                }
              }}
            />
            <Text weight="medium">Include project information</Text>
          </Flex>
          <Badge color="blue" size="2">
            Total: ~{totalTokens.toLocaleString()} tokens
            {isPreviewing && " (updating...)"}
          </Badge>
        </Flex>

        <Separator size="4" mb="3" />

        <ScrollArea style={{ maxHeight: 400 }}>
          <Flex direction="column" gap="1">
            {Object.keys(SECTION_META).map((sectionKey) => {
              const key =
                sectionKey as keyof ProjectInformationConfig["sections"];
              return (
                <React.Fragment key={sectionKey}>
                  <SectionRow
                    sectionKey={sectionKey}
                    config={localConfig.sections[key]}
                    blocks={blocks}
                    onToggle={(enabled) => updateSection(key, { enabled })}
                    onFieldChange={(field, value) =>
                      updateSection(key, { [field]: value })
                    }
                    onFileToggle={(path, enabled) =>
                      updateFileOverride(key, path, enabled)
                    }
                    onPreviewBlock={setPreviewBlock}
                  />
                  <Separator size="4" />
                </React.Fragment>
              );
            })}
          </Flex>
        </ScrollArea>

        {previewData?.warnings && previewData.warnings.length > 0 && (
          <Callout.Root color="orange" mt="3">
            <Callout.Icon>
              <ExclamationTriangleIcon />
            </Callout.Icon>
            <Callout.Text>
              {previewData.warnings.length} warning(s):{" "}
              {previewData.warnings[0]}
              {previewData.warnings.length > 1 &&
                ` (+${previewData.warnings.length - 1} more)`}
            </Callout.Text>
          </Callout.Root>
        )}

        <Flex gap="3" mt="4" justify="end">
          <Button
            type="button"
            variant="soft"
            color="gray"
            onClick={handleReset}
          >
            Reset to Defaults
          </Button>
          <Dialog.Close>
            <Button type="button" variant="soft" color="gray">
              Cancel
            </Button>
          </Dialog.Close>
          <Button
            type="button"
            onClick={() => void handleSave()}
            disabled={isSaving}
          >
            {isSaving ? "Saving..." : "Save"}
          </Button>
        </Flex>

        <ContentPreviewDialog
          block={previewBlock}
          onClose={() => setPreviewBlock(null)}
        />
      </Dialog.Content>
    </Dialog.Root>
  );
};
