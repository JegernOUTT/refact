import React, {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { AlertTriangle, CheckCircle2, Eye } from "lucide-react";
import {
  useGetProjectInformationQuery,
  useSaveProjectInformationMutation,
  useGetProjectInformationPreviewMutation,
  ProjectInformationConfig,
  ProjectInfoBlock,
  defaultProjectInformationConfig,
  SectionConfig,
} from "../../services/refact/projectInformation";
import { useAppDispatch } from "../../hooks/useAppDispatch";
import { dialogNonInteractiveCloseHandlers } from "../../utils/dialogPointerClose";
import { setIncludeProjectInfo } from "../../features/Chat/Thread/actions";
import {
  KitBadge,
  KitButton,
  KitCheckRow,
  KitDialog,
  KitDialogCancel,
  KitDivider,
  KitIconButton,
  KitSection,
  KitSliderRow,
  KitStaticRow,
  KitToggleRow,
} from "../ComposerKit";
import styles from "./ProjectInformationDialog.module.css";

type Props = {
  chatId: string;
  open: boolean;
  onOpenChange: (open: boolean) => void;
};

type SectionKey = keyof ProjectInformationConfig["sections"];

type SectionMeta = {
  label: string;
  field: "max_chars" | "max_chars_per_item" | "max_items";
  minTokens: number;
  maxTokens: number;
  stepTokens: number;
};

const SECTION_META: Record<SectionKey, SectionMeta> = {
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

const SECTION_KEYS = Object.keys(SECTION_META) as SectionKey[];

const SECTIONS_WITH_FILE_TOGGLES: SectionKey[] = [
  "instruction_files",
  "memories",
];

const PREVIEW_DEBOUNCE_MS = 400;

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

const countTokens = (blocks: ProjectInfoBlock[]): number =>
  charsToTokens(
    blocks.reduce((sum, b) => (b.enabled ? sum + b.char_count : sum), 0),
  );

const sameBlockList = (
  a: ProjectInfoBlock[] | undefined,
  b: ProjectInfoBlock[],
): boolean => {
  if (!a || a.length !== b.length) return false;
  return a.every((block, index) => block === b[index]);
};

const blockTokensLabel = (block: ProjectInfoBlock): string =>
  block.original_char_count
    ? `${charsToTokens(
        block.original_char_count,
      ).toLocaleString()} → ${charsToTokens(
        block.char_count,
      ).toLocaleString()} tok`
    : `~${charsToTokens(block.char_count).toLocaleString()} tok`;

type ContentPreviewProps = {
  block: ProjectInfoBlock | null;
  onClose: () => void;
};

const ContentPreviewDialog: React.FC<ContentPreviewProps> = ({
  block,
  onClose,
}) => {
  if (!block) return null;

  const isTruncated = Boolean(block.truncated && block.original_char_count);

  return (
    <KitDialog
      open
      onOpenChange={(open) => !open && onClose()}
      title={block.path ?? block.title}
      meta={
        <span className={styles.previewMeta}>
          <KitBadge>{block.section}</KitBadge>
          {isTruncated ? <KitBadge tone="warning">Truncated</KitBadge> : null}
          {blockTokensLabel(block)}
        </span>
      }
      closeLabel="Close preview"
      width="800px"
      maxHeight="80vh"
      bodyProps={dialogNonInteractiveCloseHandlers(onClose)}
      footer={<KitDialogCancel>Close</KitDialogCancel>}
    >
      <pre className={styles.previewCode}>{block.content || "(empty)"}</pre>
    </KitDialog>
  );
};

type SectionRowProps = {
  sectionKey: SectionKey;
  config: SectionConfig;
  blocks: ProjectInfoBlock[];
  onToggle: (sectionKey: SectionKey, enabled: boolean) => void;
  onLimitChange: (sectionKey: SectionKey, field: string, value: number) => void;
  onFileToggle: (
    sectionKey: SectionKey,
    block: ProjectInfoBlock,
    enabled: boolean,
  ) => void;
  onPreviewBlock: (block: ProjectInfoBlock) => void;
};

const PreviewButton: React.FC<{
  block: ProjectInfoBlock;
  onPreview: (block: ProjectInfoBlock) => void;
}> = ({ block, onPreview }) => (
  <KitIconButton
    icon={Eye}
    aria-label="View content"
    title="View content"
    onClick={(event) => {
      event.preventDefault();
      onPreview(block);
    }}
  />
);

const SectionRowComponent: React.FC<SectionRowProps> = ({
  sectionKey,
  config,
  blocks,
  onToggle,
  onLimitChange,
  onFileToggle,
  onPreviewBlock,
}) => {
  const meta = SECTION_META[sectionKey];
  const enabledCount = blocks.filter((b) => b.enabled).length;
  const tokens = countTokens(blocks);

  const isItemsField = meta.field === "max_items";
  const currentChars = config[meta.field] ?? tokensToChars(meta.maxTokens / 2);
  const currentTokens = isItemsField
    ? currentChars
    : charsToTokens(currentChars);
  const fieldLabel = isItemsField ? "Max items" : "Max tokens";
  const showFileToggles =
    SECTIONS_WITH_FILE_TOGGLES.includes(sectionKey) &&
    blocks.length > 0 &&
    Boolean(blocks[0].path);

  const handleSliderChange = (tokenValue: number) => {
    const charValue = isItemsField ? tokenValue : tokensToChars(tokenValue);
    onLimitChange(sectionKey, meta.field, charValue);
  };

  return (
    <div className={styles.section}>
      <KitToggleRow
        label={meta.label}
        ariaLabel={meta.label}
        meta={config.enabled ? `~${tokens.toLocaleString()} tok` : undefined}
        checked={config.enabled}
        onCheckedChange={(enabled) => onToggle(sectionKey, enabled)}
      />
      {config.enabled ? (
        <div className={styles.detail}>
          <KitSliderRow
            label={fieldLabel}
            value={currentTokens}
            min={meta.minTokens}
            max={meta.maxTokens}
            step={meta.stepTokens}
            ariaLabel={`${meta.label} ${fieldLabel}`}
            onValueChange={handleSliderChange}
          />
          {showFileToggles
            ? blocks.map((block) => (
                <KitCheckRow
                  key={block.id}
                  label={truncatePath(block.path ?? block.title, 45)}
                  title={block.path ?? block.title}
                  ariaLabel={block.path ?? block.title}
                  meta={blockTokensLabel(block)}
                  trailing={
                    <PreviewButton block={block} onPreview={onPreviewBlock} />
                  }
                  checked={block.enabled}
                  onCheckedChange={(checked) =>
                    onFileToggle(sectionKey, block, checked)
                  }
                />
              ))
            : null}
          {!showFileToggles && blocks.length > 0 ? (
            <KitStaticRow
              tone="muted"
              label={`${enabledCount}/${blocks.length} item(s)`}
              meta={`~${tokens.toLocaleString()} tok`}
              trailing={
                blocks.length === 1 ? (
                  <PreviewButton block={blocks[0]} onPreview={onPreviewBlock} />
                ) : undefined
              }
            />
          ) : null}
        </div>
      ) : null}
    </div>
  );
};

const SectionRow = React.memo(SectionRowComponent);
SectionRow.displayName = "SectionRow";

export const ProjectInformationDialog: React.FC<Props> = ({
  chatId,
  open,
  onOpenChange,
}) => {
  const dispatch = useAppDispatch();
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
  // Optimistic, id-keyed overlay for per-file switches. Never sent to the
  // preview endpoint: file toggles do not change server-side truncation.
  const [fileOverlay, setFileOverlay] = useState<
    Partial<Record<string, boolean>>
  >({});
  const [saveError, setSaveError] = useState<string | null>(null);
  const [saveSuccess, setSaveSuccess] = useState(false);
  const [previewBlock, setPreviewBlock] = useState<ProjectInfoBlock | null>(
    null,
  );

  const configRef = useRef(localConfig);
  configRef.current = localConfig;

  // Sync from the server only when the dialog (re)opens, so an in-dialog save
  // never replaces the config the user is editing.
  const syncedRef = useRef(false);
  useEffect(() => {
    if (!open) {
      syncedRef.current = false;
      setSaveError(null);
      setSaveSuccess(false);
      return;
    }
    if (savedConfig && !syncedRef.current) {
      syncedRef.current = true;
      setLocalConfig(savedConfig);
      setFileOverlay({});
    }
  }, [open, savedConfig]);

  // Only limit values change what the backend renders, so only those retrigger
  // a preview (debounced).
  const limitsSignature = useMemo(
    () =>
      JSON.stringify(
        SECTION_KEYS.map((key) => {
          const section = localConfig.sections[key];
          return [
            section.max_chars,
            section.max_items,
            section.max_chars_per_item,
            section.max_depth,
          ];
        }),
      ),
    [localConfig.sections],
  );

  useEffect(() => {
    if (!open) return;
    const timeoutId = setTimeout(() => {
      if (!configRef.current.enabled) return;
      void triggerPreview(configRef.current);
    }, PREVIEW_DEBOUNCE_MS);
    return () => clearTimeout(timeoutId);
  }, [open, limitsSignature, triggerPreview]);

  const baseBlocks = useMemo(
    () => previewData?.blocks ?? [],
    [previewData?.blocks],
  );

  // Per-block identity cache: a resolved block keeps its reference as long as
  // neither its source nor its effective enabled flag changed.
  const blockCacheRef = useRef(
    new Map<string, { source: ProjectInfoBlock; result: ProjectInfoBlock }>(),
  );
  // Per-section slice cache: unchanged sections keep the same array reference,
  // so their memoized rows never re-render.
  const sliceCacheRef = useRef<Partial<Record<SectionKey, ProjectInfoBlock[]>>>(
    {},
  );

  const blocksBySection = useMemo(() => {
    const grouped = {} as Record<SectionKey, ProjectInfoBlock[]>;
    for (const key of SECTION_KEYS) {
      grouped[key] = [];
    }

    const cache = blockCacheRef.current;
    baseBlocks.forEach((block) => {
      const sectionKey = block.section as SectionKey;
      if (!(sectionKey in grouped)) return;
      const sectionEnabled = localConfig.sections[sectionKey].enabled;
      const overlay = fileOverlay[block.id];
      const enabled = sectionEnabled && (overlay ?? block.enabled);
      let resolved: ProjectInfoBlock;
      const cached = cache.get(block.id);
      if (
        cached &&
        cached.source === block &&
        cached.result.enabled === enabled
      ) {
        resolved = cached.result;
      } else {
        resolved = block.enabled === enabled ? block : { ...block, enabled };
        cache.set(block.id, { source: block, result: resolved });
      }
      grouped[sectionKey].push(resolved);
    });

    const previous = sliceCacheRef.current;
    const next: Record<SectionKey, ProjectInfoBlock[]> = grouped;
    SECTION_KEYS.forEach((key) => {
      const cachedSlice = previous[key];
      if (sameBlockList(cachedSlice, grouped[key]) && cachedSlice) {
        next[key] = cachedSlice;
      }
    });
    sliceCacheRef.current = next;
    return next;
  }, [baseBlocks, fileOverlay, localConfig.sections]);

  const totalTokens = useMemo(() => {
    if (!localConfig.enabled) return 0;
    return SECTION_KEYS.reduce(
      (sum, key) => sum + countTokens(blocksBySection[key]),
      0,
    );
  }, [blocksBySection, localConfig.enabled]);

  // Optimistic: section switch only touches local state.
  const handleSectionToggle = useCallback(
    (sectionKey: SectionKey, enabled: boolean) => {
      setLocalConfig((prev) => ({
        ...prev,
        sections: {
          ...prev.sections,
          [sectionKey]: { ...prev.sections[sectionKey], enabled },
        },
      }));
    },
    [],
  );

  // The only change that needs the server: truncation depends on limits.
  const handleLimitChange = useCallback(
    (sectionKey: SectionKey, field: string, value: number) => {
      setLocalConfig((prev) => ({
        ...prev,
        sections: {
          ...prev.sections,
          [sectionKey]: { ...prev.sections[sectionKey], [field]: value },
        },
      }));
    },
    [],
  );

  // Optimistic: per-file switch updates the config override and the local
  // block overlay so token totals move instantly, with no round-trip.
  const handleFileToggle = useCallback(
    (sectionKey: SectionKey, block: ProjectInfoBlock, enabled: boolean) => {
      setFileOverlay((prev) => ({ ...prev, [block.id]: enabled }));
      const path = block.path;
      if (!path) return;
      setLocalConfig((prev) => {
        const section = prev.sections[sectionKey];
        const currentOverrides: Partial<
          NonNullable<SectionConfig["overrides"]>
        > = section.overrides ?? {};
        return {
          ...prev,
          sections: {
            ...prev.sections,
            [sectionKey]: {
              ...section,
              overrides: {
                ...currentOverrides,
                [path]: { ...(currentOverrides[path] ?? {}), enabled },
              },
            },
          },
        };
      });
    },
    [],
  );

  const handleMasterToggle = useCallback(
    (enabled: boolean) => {
      setLocalConfig((prev) => ({ ...prev, enabled }));
      if (chatId) {
        dispatch(setIncludeProjectInfo({ chatId, value: enabled }));
      }
    },
    [chatId, dispatch],
  );

  const handleSave = useCallback(async () => {
    setSaveError(null);
    setSaveSuccess(false);
    try {
      await saveConfig(configRef.current).unwrap();
      setSaveSuccess(true);
      setTimeout(() => onOpenChange(false), 500);
    } catch (err) {
      setSaveError(
        err instanceof Error ? err.message : "Failed to save configuration",
      );
    }
  }, [saveConfig, onOpenChange]);

  const handleReset = useCallback(() => {
    setLocalConfig(defaultProjectInformationConfig);
    setFileOverlay({});
  }, []);

  const warnings = previewData?.warnings ?? [];
  const closeOnBackground = dialogNonInteractiveCloseHandlers(() =>
    onOpenChange(false),
  );

  return (
    <KitDialog
      open={open}
      onOpenChange={onOpenChange}
      title="Project information"
      description="Choose what project context goes into this chat. Token counts are approximate (~4 chars/token)."
      meta={
        isLoading ? undefined : (
          <span
            className={styles.total}
            data-busy={isPreviewing ? "true" : undefined}
          >
            Total ~{totalTokens.toLocaleString()} tokens
          </span>
        )
      }
      width="560px"
      bodyProps={closeOnBackground}
      footerStart={
        isLoading ? undefined : (
          <KitButton variant="ghost" onClick={handleReset}>
            Reset to defaults
          </KitButton>
        )
      }
      footerNote={
        saveSuccess ? (
          <span className={styles.saved}>
            <CheckCircle2 aria-hidden="true" />
            Configuration saved
          </span>
        ) : undefined
      }
      footer={
        isLoading ? undefined : (
          <>
            <KitDialogCancel />
            <KitButton
              variant="primary"
              onClick={() => void handleSave()}
              disabled={isSaving}
            >
              {isSaving ? "Saving…" : "Save"}
            </KitButton>
          </>
        )
      }
    >
      {isLoading ? (
        <KitStaticRow tone="muted" label="Loading…" />
      ) : (
        <>
          {saveError ? (
            <KitStaticRow
              tone="danger"
              icon={<AlertTriangle aria-hidden="true" />}
              label={saveError}
            />
          ) : null}
          <KitToggleRow
            label="Include project information"
            ariaLabel="Include project information"
            hint="Turn off to send chats without any project context"
            checked={localConfig.enabled}
            onCheckedChange={handleMasterToggle}
          />
          <KitDivider />
          <KitSection>Sections</KitSection>
          {SECTION_KEYS.map((sectionKey) => (
            <SectionRow
              key={sectionKey}
              sectionKey={sectionKey}
              config={localConfig.sections[sectionKey]}
              blocks={blocksBySection[sectionKey]}
              onToggle={handleSectionToggle}
              onLimitChange={handleLimitChange}
              onFileToggle={handleFileToggle}
              onPreviewBlock={setPreviewBlock}
            />
          ))}
          {warnings.length > 0 ? (
            <KitStaticRow
              tone="muted"
              icon={
                <AlertTriangle aria-hidden="true" className={styles.warn} />
              }
              label={`${warnings.length} warning(s): ${warnings[0]}${
                warnings.length > 1 ? ` (+${warnings.length - 1} more)` : ""
              }`}
            />
          ) : null}
          <ContentPreviewDialog
            block={previewBlock}
            onClose={() => setPreviewBlock(null)}
          />
        </>
      )}
    </KitDialog>
  );
};
