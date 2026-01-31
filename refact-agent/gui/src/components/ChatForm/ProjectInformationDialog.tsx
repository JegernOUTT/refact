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
} from "@radix-ui/themes";
import { ExclamationTriangleIcon, CheckCircledIcon } from "@radix-ui/react-icons";
import {
  useGetProjectInformationQuery,
  useSaveProjectInformationMutation,
  useGetProjectInformationPreviewMutation,
  ProjectInformationConfig,
  ProjectInfoBlock,
  defaultProjectInformationConfig,
  SectionConfig,
} from "../../services/refact/projectInformation";

type Props = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
};

type SectionMeta = {
  label: string;
  field: "max_chars" | "max_chars_per_item" | "max_items";
  min: number;
  max: number;
  step: number;
};

const SECTION_META: Record<string, SectionMeta> = {
  system_info: { label: "System Information", field: "max_chars", min: 500, max: 8000, step: 500 },
  environment_instructions: { label: "Environment Instructions", field: "max_chars", min: 1000, max: 16000, step: 1000 },
  detected_environments: { label: "Detected Environments", field: "max_items", min: 5, max: 100, step: 5 },
  git_info: { label: "Git Information", field: "max_chars", min: 1000, max: 16000, step: 1000 },
  project_tree: { label: "Project Tree", field: "max_chars", min: 2000, max: 32000, step: 2000 },
  instruction_files: { label: "Instruction Files (AGENTS.md, etc.)", field: "max_chars_per_item", min: 1000, max: 16000, step: 1000 },
  project_configs: { label: "Project Configs (.refact/)", field: "max_chars_per_item", min: 1000, max: 8000, step: 500 },
  memories: { label: "Memories", field: "max_chars_per_item", min: 500, max: 8000, step: 500 },
};

const estimateTokens = (chars: number): number => Math.ceil(chars / 4);

type SectionRowProps = {
  sectionKey: string;
  config: SectionConfig;
  blocks: ProjectInfoBlock[];
  onToggle: (enabled: boolean) => void;
  onFieldChange: (field: string, value: number) => void;
};

const SectionRow: React.FC<SectionRowProps> = ({
  sectionKey,
  config,
  blocks,
  onToggle,
  onFieldChange,
}) => {
  const meta = SECTION_META[sectionKey];
  const sectionBlocks = blocks.filter((b) => b.section === sectionKey && b.enabled);
  const totalChars = sectionBlocks.reduce((sum, b) => sum + b.char_count, 0);
  const tokens = estimateTokens(totalChars);

  const currentValue = config[meta.field] ?? meta.max / 2;
  const fieldLabel = meta.field === "max_items" ? "Max items" : "Max chars";

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
            <Text size="1" color="gray">{fieldLabel}:</Text>
            <Slider
              size="1"
              value={[currentValue]}
              min={meta.min}
              max={meta.max}
              step={meta.step}
              onValueChange={([v]) => onFieldChange(meta.field, v)}
              style={{ width: 120 }}
            />
            <Text size="1" color="gray">{currentValue.toLocaleString()}</Text>
          </Flex>
          {sectionBlocks.length > 0 && (
            <Text size="1" color="gray">
              {sectionBlocks.length} item(s), {totalChars.toLocaleString()} chars
            </Text>
          )}
        </Flex>
      )}
    </Flex>
  );
};

export const ProjectInformationDialog: React.FC<Props> = ({ open, onOpenChange }) => {
  const { data: savedConfig, isLoading } = useGetProjectInformationQuery(undefined, {
    skip: !open,
  });
  const [saveConfig, { isLoading: isSaving }] = useSaveProjectInformationMutation();
  const [triggerPreview, { data: previewData, isLoading: isPreviewing }] =
    useGetProjectInformationPreviewMutation();

  const [localConfig, setLocalConfig] = useState<ProjectInformationConfig>(
    defaultProjectInformationConfig,
  );
  const [saveError, setSaveError] = useState<string | null>(null);
  const [saveSuccess, setSaveSuccess] = useState(false);

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
    if (open && localConfig) {
      const timeoutId = setTimeout(() => {
        triggerPreview(localConfig);
      }, 200);
      return () => clearTimeout(timeoutId);
    }
  }, [open, localConfig, triggerPreview]);

  const blocks = previewData?.blocks ?? [];

  const totalTokens = useMemo(() => {
    const enabledBlocks = blocks.filter((b) => b.enabled);
    const totalChars = enabledBlocks.reduce((sum, b) => sum + b.char_count, 0);
    return estimateTokens(totalChars);
  }, [blocks]);

  const updateSection = useCallback(
    (sectionKey: keyof ProjectInformationConfig["sections"], updates: Partial<SectionConfig>) => {
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

  const handleSave = useCallback(async () => {
    setSaveError(null);
    setSaveSuccess(false);
    try {
      await saveConfig(localConfig).unwrap();
      setSaveSuccess(true);
      setTimeout(() => onOpenChange(false), 500);
    } catch (err) {
      setSaveError(err instanceof Error ? err.message : "Failed to save configuration");
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
      <Dialog.Content maxWidth="600px">
        <Dialog.Title>Project Information</Dialog.Title>
        <Dialog.Description size="2" color="gray" mb="4">
          Configure what project information is included in chat context.
          Token counts are approximate (~4 chars/token).
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
              onCheckedChange={(enabled) =>
                setLocalConfig((prev) => ({ ...prev, enabled }))
              }
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
            {(Object.keys(SECTION_META) as Array<keyof typeof SECTION_META>).map(
              (sectionKey) => (
                <React.Fragment key={sectionKey}>
                  <SectionRow
                    sectionKey={sectionKey}
                    config={localConfig.sections[sectionKey as keyof ProjectInformationConfig["sections"]]}
                    blocks={blocks}
                    onToggle={(enabled) =>
                      updateSection(sectionKey as keyof ProjectInformationConfig["sections"], { enabled })
                    }
                    onFieldChange={(field, value) =>
                      updateSection(sectionKey as keyof ProjectInformationConfig["sections"], { [field]: value })
                    }
                  />
                  <Separator size="4" />
                </React.Fragment>
              ),
            )}
          </Flex>
        </ScrollArea>

        {previewData?.warnings && previewData.warnings.length > 0 && (
          <Callout.Root color="orange" mt="3">
            <Callout.Icon>
              <ExclamationTriangleIcon />
            </Callout.Icon>
            <Callout.Text>
              {previewData.warnings.length} warning(s): {previewData.warnings[0]}
              {previewData.warnings.length > 1 && ` (+${previewData.warnings.length - 1} more)`}
            </Callout.Text>
          </Callout.Root>
        )}

        <Flex gap="3" mt="4" justify="end">
          <Button variant="soft" color="gray" onClick={handleReset}>
            Reset to Defaults
          </Button>
          <Dialog.Close>
            <Button variant="soft" color="gray">
              Cancel
            </Button>
          </Dialog.Close>
          <Button onClick={handleSave} disabled={isSaving}>
            {isSaving ? "Saving..." : "Save"}
          </Button>
        </Flex>
      </Dialog.Content>
    </Dialog.Root>
  );
};
