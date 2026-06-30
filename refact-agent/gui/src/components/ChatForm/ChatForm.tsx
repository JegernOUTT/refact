import React, { useCallback, useEffect, useMemo } from "react";

import { Flex, Text } from "@radix-ui/themes";
import styles from "./ChatForm.module.css";

const TEXT_FILE_EXTENSIONS = new Set([
  ".txt",
  ".md",
  ".json",
  ".yaml",
  ".yml",
  ".toml",
  ".xml",
  ".csv",
  ".js",
  ".ts",
  ".tsx",
  ".jsx",
  ".py",
  ".rs",
  ".go",
  ".java",
  ".kt",
  ".c",
  ".cpp",
  ".h",
  ".hpp",
  ".cs",
  ".rb",
  ".php",
  ".swift",
  ".sh",
  ".bash",
  ".zsh",
  ".html",
  ".css",
  ".scss",
  ".sass",
  ".less",
  ".sql",
  ".graphql",
  ".env",
  ".gitignore",
  ".dockerignore",
]);

function isTextFile(filename: string): boolean {
  const ext = filename.slice(filename.lastIndexOf(".")).toLowerCase();
  return TEXT_FILE_EXTENSIONS.has(ext);
}

function isEditableElement(element: Element | null): boolean {
  if (!element) return false;
  if (element instanceof HTMLElement && element.isContentEditable) return true;
  return Boolean(
    element.closest(
      'input, textarea, select, button, a, [role="button"], [role="menuitem"], [data-radix-popper-content-wrapper]',
    ),
  );
}

function isInsideRadixPortal(element: Element | null): boolean {
  if (!element) return false;
  return Boolean(
    element.closest(
      '[data-radix-popper-content-wrapper], [data-radix-portal], [role="dialog"], [role="menu"], [role="listbox"]',
    ),
  );
}

function isInsideComposerSurface(
  target: EventTarget | null,
  composerRoot: HTMLElement | null,
): boolean {
  if (!(target instanceof Node)) return false;
  if (composerRoot?.contains(target)) return true;
  if (target instanceof Element && isInsideRadixPortal(target)) return true;
  return false;
}

function isInsideComposerControls(element: Element): boolean {
  return Boolean(
    element.closest(
      [
        `.${styles.inputHeader}`,
        `.${styles.topControlsRow}`,
        `.${styles.topStatusControls}`,
        `.${styles.bottomControlsRow}`,
        `.${styles.bottomActionControls}`,
      ].join(","),
    ),
  );
}

function isInsideNoExpandControl(target: EventTarget | null): boolean {
  if (!(target instanceof Element)) return false;
  // Radix portals re-dispatch events through the React tree, so a focus or
  // click inside portaled content reaches composer handlers with a DOM target
  // outside the composer. Treat those as non-expanding too: explicit menu
  // tracking (handleComposerMenuOpenChange) owns expansion for menus.
  return (
    Boolean(target.closest("[data-composer-no-expand]")) ||
    isInsideRadixPortal(target)
  );
}

import {
  BackToSideBarButton,
  UnifiedSendButton,
  BrowserToggleButton,
  WandButton,
  AutoEnrichmentToggleButton,
  AutoCompactToggleButton,
  ThreadInfoButton,
} from "../Buttons";
import {
  StreamingTokenCounter,
  UsageCounter,
  ProviderUsageIndicator,
} from "../UsageCounter";
import { TrajectoryButton } from "../Trajectory";
import { TextAreaWithChips } from "../TextAreaWithChips";
import { selectHost } from "../../features/Config/configSlice";
import { useEventsBusForIDE } from "../../hooks";
import { Form } from "./Form";
import {
  useOnPressedEnter,
  useIsOnline,
  useConfig,
  useCapsForToolUse,
  useAutoFocusOnce,
  useChatActions,
  useFirstSendAutoFlip,
} from "../../hooks";
import { Callout } from "../Callout";
import { ComboBox } from "../ComboBox";
import { UnifiedAttachmentsTray } from "./UnifiedAttachmentsTray";
import { ChatSettingsDropdown } from "./ChatSettingsDropdown";
import { ModeSelect } from "./ModeSelect";
import { WorktreeControl } from "../../features/Worktrees";
import { addCheckboxValuesToInput } from "./utils";
import { stripUnfilledPlaceholders } from "../ComboBox/argumentPlaceholders";
import { useCommandCompletionAndPreviewFiles } from "./useCommandCompletionAndPreviewFiles";
import { useAppSelector, useAppDispatch } from "../../hooks";
import { getErrorMessage } from "../../features/Errors/errorsSlice";
import { useAttachedFiles, useCheckboxes } from "./useCheckBoxes";
import { useInputValue } from "./useInputValue";
import {
  clearInformation,
  getInformationMessage,
} from "../../features/Errors/informationSlice";
import { InformationCallout } from "../Callout/Callout";
import { ToolConfirmation } from "./ToolConfirmation";
import { selectThreadConfirmationById } from "../../features/Chat/Thread";
import { AttachImagesButton } from "../Dropzone";
import { MicrophoneButton, MicrophoneButtonRef } from "./MicrophoneButton";
import { useAttachedImages } from "../../hooks/useAttachedImages";
import {
  selectChatErrorById,
  selectIsStreamingById,
  selectIsWaitingById,
  selectMessagesById,
  selectQueuedItemsById,
  selectThreadImagesById,
  selectThreadModeById,
  selectManualPreviewItemsById,
  removeManualPreviewItem,
  setThreadMode,
  DEFAULT_MODE,
  selectIsBuddyChat,
  useThreadId,
} from "../../features/Chat/Thread";
import { useReportErrorMutation } from "../../services/refact/buddy";

import { useUsageCounter } from "../UsageCounter/useUsageCounter";
import { ChatInputTopControls } from "./ChatInputTopControls";

import classNames from "classnames";
type ComposerHelpProps = {
  children: React.ReactNode;
};

const ComposerHelp: React.FC<ComposerHelpProps> = ({ children }) => (
  <div className={styles.helpText}>{children}</div>
);

export type SendPolicy = "immediate" | "after_flow";

export type ChatFormProps = {
  onSubmit: (str: string, sendPolicy?: SendPolicy) => void;
  onClose?: () => void;
  className?: string;
  embedded?: boolean;
  onExpandedChange?: (expanded: boolean) => void;
};

export const ChatForm: React.FC<ChatFormProps> = ({
  onSubmit,
  onClose,
  className,
  embedded = false,
  onExpandedChange,
}) => {
  const dispatch = useAppDispatch();
  const chatId = useThreadId();
  const isStreaming = useAppSelector((state) =>
    selectIsStreamingById(state, chatId),
  );
  const isWaiting = useAppSelector((state) =>
    selectIsWaitingById(state, chatId),
  );
  const caps = useCapsForToolUse();
  const { isMultimodalitySupportedForCurrentModel } = caps;
  const config = useConfig();
  const host = useAppSelector(selectHost);
  const { queryPathThenOpenFile } = useEventsBusForIDE();
  const globalError = useAppSelector(getErrorMessage);
  const chatError = useAppSelector((state) =>
    selectChatErrorById(state, chatId),
  );
  const isBuddyChat = useAppSelector((state) =>
    selectIsBuddyChat(state, chatId),
  );
  const information = useAppSelector(getInformationMessage);
  const pauseReasonsWithPause = useAppSelector((state) =>
    selectThreadConfirmationById(state, chatId),
  );
  const [reportError] = useReportErrorMutation();
  useEffect(() => {
    if (chatError) {
      void reportError({ error: chatError, chat_id: chatId });
    }
  }, [chatError, chatId, reportError]);
  const [helpInfo, setHelpInfo] = React.useState<React.ReactNode | null>(null);
  const [isVoiceActive, setIsVoiceActive] = React.useState(false);
  const [liveTranscript, setLiveTranscript] = React.useState("");
  const [inputResetKey, setInputResetKey] = React.useState(0);
  const [isComposerExpanded, setIsComposerExpanded] = React.useState(false);
  const [openComposerMenus, setOpenComposerMenus] = React.useState(0);
  const composerRef = React.useRef<HTMLDivElement>(null);
  const composerPointerDownInsideRef = React.useRef(false);
  const clearComposerPointerDownRef = React.useRef<number | null>(null);
  const isOnline = useIsOnline();
  const { isContextFull } = useUsageCounter();
  const messages = useAppSelector((state) => selectMessagesById(state, chatId));
  const queuedItems = useAppSelector((state) =>
    selectQueuedItemsById(state, chatId),
  );
  const threadMode = useAppSelector((state) =>
    selectThreadModeById(state, chatId),
  );
  const manualPreviewItems = useAppSelector((state) =>
    selectManualPreviewItemsById(state, chatId),
  );
  const autoFocus = useAutoFocusOnce();
  const { abort, regenerate } = useChatActions(chatId);
  useFirstSendAutoFlip();

  const onSetMode = useCallback(
    (
      modeId: string,
      threadDefaults?: Parameters<typeof setThreadMode>[0]["threadDefaults"],
    ) => {
      if (chatId) {
        dispatch(setThreadMode({ chatId, mode: modeId, threadDefaults }));
      }
    },
    [dispatch, chatId],
  );

  const isModeDisabled = useMemo(() => isStreaming, [isStreaming]);
  const attachedFiles = useAttachedFiles();
  const attachedImages = useAppSelector((state) =>
    selectThreadImagesById(state, chatId),
  );
  const microphoneRef = React.useRef<MicrophoneButtonRef>(null);

  const allDisabled = caps.usableModelsForPlan.every((option) => {
    if (typeof option === "string") return false;
    return option.disabled;
  });

  const disableSend = useMemo(() => {
    if (allDisabled) return true;
    if (messages.length === 0) return false;
    if (isContextFull) return true;
    return isWaiting || isStreaming || !isOnline;
  }, [
    allDisabled,
    messages.length,
    isWaiting,
    isStreaming,
    isOnline,
    isContextFull,
  ]);

  const disableMicrophone = useMemo(() => {
    if (allDisabled) return true;
    if (isContextFull) return true;
    if (!isOnline) return true;
    return false;
  }, [allDisabled, isContextFull, isOnline]);

  const {
    processAndInsertImages,
    processAndInsertTextFiles,
    textFiles,
    resetAllTextFiles,
  } = useAttachedImages();
  const handlePastingFile = useCallback(
    (event: React.ClipboardEvent<HTMLTextAreaElement>) => {
      const imageFiles: File[] = [];
      const textFilesList: File[] = [];
      const items = event.clipboardData.items;

      for (const item of items) {
        if (item.kind === "file") {
          const file = item.getAsFile();
          if (file) {
            if (file.type === "image/jpeg" || file.type === "image/png") {
              if (isMultimodalitySupportedForCurrentModel) {
                imageFiles.push(file);
              }
            } else if (file.type.startsWith("text/") || isTextFile(file.name)) {
              textFilesList.push(file);
            }
          }
        }
      }

      if (imageFiles.length > 0 || textFilesList.length > 0) {
        event.preventDefault();
        if (imageFiles.length > 0) {
          processAndInsertImages(imageFiles);
        }
        if (textFilesList.length > 0) {
          processAndInsertTextFiles(textFilesList);
        }
      }
    },
    [
      processAndInsertImages,
      processAndInsertTextFiles,
      isMultimodalitySupportedForCurrentModel,
    ],
  );

  const {
    checkboxes,
    onToggleCheckbox,
    unCheckAll,
    setLineSelectionInteracted,
  } = useCheckboxes();

  const [value, setValue, isSendImmediately, setIsSendImmediately] =
    useInputValue(() => unCheckAll());

  const displayedInputValue =
    isVoiceActive && liveTranscript
      ? value.trim()
        ? `${value}\n${liveTranscript}`
        : liveTranscript
      : value;

  const valueRef = React.useRef(value);
  valueRef.current = value;

  const argumentPlaceholdersRef = React.useRef<string[]>([]);

  const onClearInformation = useCallback(
    () => dispatch(clearInformation()),
    [dispatch],
  );

  const { previewFiles, commands, requestCompletion } =
    useCommandCompletionAndPreviewFiles(
      checkboxes,
      attachedFiles.addFilesToInput,
    );

  const handleSubmit = useCallback(
    (sendPolicy: SendPolicy = "after_flow", inputValue = value) => {
      const trimmedValue = stripUnfilledPlaceholders(
        inputValue,
        argumentPlaceholdersRef.current,
      ).trim();
      const hasImages = attachedImages.length > 0;
      const hasTextFiles = textFiles.length > 0;
      const canSubmit =
        (trimmedValue.length > 0 || hasImages || hasTextFiles) &&
        isOnline &&
        !allDisabled;

      if (canSubmit) {
        const valueWithFiles = attachedFiles.addFilesToInput(trimmedValue);
        const valueWithTextFiles = textFiles.reduce((acc, file) => {
          const ext = file.name.split(".").pop() ?? "";
          return `\`\`\`${ext} ${file.name}\n${file.content}\n\`\`\`\n\n${acc}`;
        }, valueWithFiles);
        const valueIncludingChecks = addCheckboxValuesToInput(
          valueWithTextFiles,
          checkboxes,
        );
        setLineSelectionInteracted(false);
        onSubmit(valueIncludingChecks, sendPolicy);
        argumentPlaceholdersRef.current = [];
        setValue("");
        setInputResetKey((k) => k + 1);
        unCheckAll();
        attachedFiles.removeAll();
        resetAllTextFiles();
        setIsComposerExpanded(false);
      }
    },
    [
      value,
      allDisabled,
      isOnline,
      attachedImages,
      textFiles,
      attachedFiles,
      checkboxes,
      setLineSelectionInteracted,
      resetAllTextFiles,
      onSubmit,
      setValue,
      unCheckAll,
    ],
  );

  const handleSendImmediately = useCallback(() => {
    handleSubmit("immediate");
  }, [handleSubmit]);

  const handleEnter = useOnPressedEnter(() => handleSubmit("after_flow"));

  const handleHelpInfo = useCallback((info: React.ReactNode | null) => {
    setHelpInfo(info);
  }, []);

  const helpText = () => (
    <Flex direction="column">
      <Text size="2" weight="bold">
        Quick help for @-commands:
      </Text>
      <Text size="2">
        @definition &lt;class_or_function_name&gt; — find the definition and
        attach it.
      </Text>
      <Text size="2">
        @references &lt;class_or_function_name&gt; — find all references and
        attach them.
      </Text>
      <Text size="2">
        @file &lt;dir/filename.ext&gt; — attaches a single file to the chat.
      </Text>
      <Text size="2">@tree — workspace directory and files tree.</Text>
      <Text size="2">@web &lt;url&gt; — attach a webpage to the chat.</Text>
    </Flex>
  );

  const handleHelpCommand = useCallback(() => {
    setHelpInfo(helpText());
  }, []);

  const handleChange = useCallback(
    (command: string) => {
      setValue(command);
      const trimmedCommand = command.trim();
      if (!trimmedCommand) {
        setLineSelectionInteracted(false);
      } else {
        setLineSelectionInteracted(true);
      }

      if (trimmedCommand === "@help") {
        handleHelpInfo(helpText());
      } else {
        handleHelpInfo(null);
      }
    },
    [handleHelpInfo, setValue, setLineSelectionInteracted],
  );

  useEffect(() => {
    if (isSendImmediately && !isWaiting && !isStreaming) {
      handleSubmit();
      setIsSendImmediately(false);
    }
  }, [
    isSendImmediately,
    isWaiting,
    isStreaming,
    handleSubmit,
    setIsSendImmediately,
  ]);

  const handleLiveTranscript = useCallback((text: string) => {
    setLiveTranscript(text);
  }, []);

  const handleRecordingChange = useCallback(
    (isRecording: boolean, isFinishing: boolean) => {
      setIsVoiceActive(isRecording || isFinishing);
      if (!isRecording && !isFinishing) {
        setLiveTranscript("");
      }
    },
    [],
  );

  const focusComposerInput = useCallback(() => {
    setIsComposerExpanded(true);
    window.requestAnimationFrame(() => {
      composerRef.current?.querySelector("textarea")?.focus();
    });
  }, []);

  const handleComposerMenuOpenChange = useCallback((open: boolean) => {
    setOpenComposerMenus((count) => Math.max(0, count + (open ? 1 : -1)));
    if (open) {
      setIsComposerExpanded(true);
    }
  }, []);

  useEffect(
    () => () => {
      if (clearComposerPointerDownRef.current !== null) {
        window.clearTimeout(clearComposerPointerDownRef.current);
      }
    },
    [],
  );

  const handleComposerPointerDownCapture = useCallback(
    (event: React.PointerEvent<HTMLFormElement>) => {
      const target = event.target;
      if (!(target instanceof Element)) return;
      if (isInsideNoExpandControl(target)) return;
      composerPointerDownInsideRef.current = true;
      if (clearComposerPointerDownRef.current !== null) {
        window.clearTimeout(clearComposerPointerDownRef.current);
      }
      clearComposerPointerDownRef.current = window.setTimeout(() => {
        composerPointerDownInsideRef.current = false;
        clearComposerPointerDownRef.current = null;
      }, 80);

      if (isEditableElement(target)) return;

      setIsComposerExpanded(true);
      if (isInsideComposerControls(target)) return;

      event.preventDefault();
      focusComposerInput();
    },
    [focusComposerInput],
  );

  const handleComposerBlur = useCallback(
    (event: React.FocusEvent<HTMLDivElement>) => {
      const root = event.currentTarget;
      const nextTarget = event.relatedTarget;
      if (isInsideComposerSurface(nextTarget, root)) return;

      window.setTimeout(() => {
        if (composerPointerDownInsideRef.current) {
          setIsComposerExpanded(true);
          return;
        }

        const activeElement = document.activeElement;
        if (isInsideComposerSurface(activeElement, root)) return;
        setIsComposerExpanded(openComposerMenus > 0);
      }, 0);
    },
    [openComposerMenus],
  );

  const handleComposerFocusCapture = useCallback(
    (event: React.FocusEvent<HTMLDivElement>) => {
      if (isInsideNoExpandControl(event.target)) return;
      setIsComposerExpanded(true);
    },
    [],
  );

  useEffect(() => {
    if (openComposerMenus > 0) {
      setIsComposerExpanded(true);
    }
  }, [openComposerMenus]);

  useEffect(() => {
    onExpandedChange?.(isComposerExpanded);
  }, [isComposerExpanded, onExpandedChange]);

  useEffect(() => {
    if (openComposerMenus <= 0) return;

    const handlePointerDown = (event: PointerEvent) => {
      const target = event.target;
      if (!(target instanceof Element)) return;
      if (isInsideRadixPortal(target)) {
        setIsComposerExpanded(true);
      }
    };

    document.addEventListener("pointerdown", handlePointerDown, true);
    return () =>
      document.removeEventListener("pointerdown", handlePointerDown, true);
  }, [openComposerMenus]);

  useEffect(() => {
    if (!isComposerExpanded) return;

    const handlePointerDown = (event: PointerEvent) => {
      const root = composerRef.current;
      if (isInsideComposerSurface(event.target, root)) {
        setIsComposerExpanded(true);
        return;
      }

      if (openComposerMenus <= 0) {
        setIsComposerExpanded(false);
      }
    };

    document.addEventListener("pointerdown", handlePointerDown, true);
    return () =>
      document.removeEventListener("pointerdown", handlePointerDown, true);
  }, [isComposerExpanded, openComposerMenus]);

  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.ctrlKey && event.shiftKey && event.code === "Space") {
        event.preventDefault();
        if (!disableMicrophone && microphoneRef.current) {
          void microphoneRef.current.toggleRecording();
        }
      }

      if (
        event.key === "Enter" &&
        !event.ctrlKey &&
        !event.metaKey &&
        !event.altKey
      ) {
        const target = event.target;
        if (target instanceof Element) {
          if (isEditableElement(target)) return;
          if (isInsideRadixPortal(target)) return;
        }
        event.preventDefault();
        focusComposerInput();
      }
    };

    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [disableMicrophone, focusComposerInput]);

  if (pauseReasonsWithPause.pause) {
    return (
      <ToolConfirmation pauseReasons={pauseReasonsWithPause.pause_reasons} />
    );
  }

  return (
    <div
      ref={composerRef}
      className={styles.composerRoot}
      onBlur={handleComposerBlur}
      onFocusCapture={handleComposerFocusCapture}
    >
      {!globalError && !chatError && information && (
        <InformationCallout
          mt="2"
          mb="2"
          onClick={onClearInformation}
          timeout={2000}
        >
          {information}
        </InformationCallout>
      )}
      {!isOnline && (
        <Callout type="info" mb="2">
          Oops, seems that connection was lost... Check your internet connection
        </Callout>
      )}

      <div className={styles.composerStack}>
        {helpInfo && <ComposerHelp>{helpInfo}</ComposerHelp>}
        <Form
          disabled={disableSend}
          className={classNames(
            styles.chatForm,
            styles.chatForm__form,
            styles.chatFormMain,
            isComposerExpanded
              ? styles.chatFormExpanded
              : styles.chatFormCollapsed,
            { [styles.chatFormEmbedded]: embedded },
            className,
          )}
          onClick={(event) => {
            if (isInsideNoExpandControl(event.target)) return;
            if (!isComposerExpanded) {
              focusComposerInput();
            }
          }}
          onPointerDownCapture={handleComposerPointerDownCapture}
          onSubmit={() => handleSubmit("after_flow")}
        >
          <div
            className={styles.expandedComposerContent}
            onFocus={() => setIsComposerExpanded(true)}
          >
            <div className={styles.expandedComposerContentInner}>
              <div className={styles.textareaWrapper}>
                <div className={styles.inputHeader}>
                  <UnifiedAttachmentsTray
                    attachedFiles={attachedFiles}
                    previewFiles={previewFiles}
                    manualPreviewItems={manualPreviewItems}
                    onRemoveManualPreviewItem={
                      chatId
                        ? (index) =>
                            dispatch(removeManualPreviewItem({ chatId, index }))
                        : undefined
                    }
                    onOpenFile={queryPathThenOpenFile}
                  />
                  <Flex
                    align="center"
                    gap="2"
                    justify="between"
                    wrap="wrap"
                    className={styles.topControlsRow}
                  >
                    <ChatInputTopControls
                      checkboxes={checkboxes}
                      onCheckedChange={onToggleCheckbox}
                      attachedFiles={attachedFiles}
                      disabled={isBuddyChat}
                    />
                    <Flex
                      align="center"
                      gap="2"
                      className={styles.topStatusControls}
                    >
                      <span className={styles.hideTopTokensFirst}>
                        <StreamingTokenCounter />
                      </span>
                      <span className={styles.hideTopTokensFirst}>
                        <ProviderUsageIndicator />
                      </span>
                      <span className={styles.hideTopTokensFirst}>
                        <UsageCounter />
                      </span>
                      <span className={styles.hideTopCompressLast}>
                        <TrajectoryButton
                          disabled={isBuddyChat}
                          onOpenChange={handleComposerMenuOpenChange}
                        />
                      </span>
                    </Flex>
                  </Flex>
                </div>

                <ComboBox
                  key={inputResetKey}
                  onHelpClick={handleHelpCommand}
                  commands={commands}
                  requestCommandsCompletion={requestCompletion}
                  value={displayedInputValue}
                  onChange={handleChange}
                  onSubmit={(event) => {
                    handleEnter(event);
                  }}
                  onArgumentPlaceholdersChange={(placeholders) => {
                    argumentPlaceholdersRef.current = placeholders;
                  }}
                  placeholder={
                    isVoiceActive
                      ? "Listening..."
                      : commands.completions.length < 1
                        ? "Type @ or / for commands"
                        : ""
                  }
                  render={(props) => (
                    <TextAreaWithChips
                      data-testid="chat-form-textarea"
                      required={true}
                      {...props}
                      host={host}
                      onOpenFile={queryPathThenOpenFile}
                      autoFocus={isComposerExpanded && autoFocus}
                      readOnly={isVoiceActive}
                      onPaste={handlePastingFile}
                    />
                  )}
                />
              </div>
            </div>
          </div>
          <Flex
            gap="2"
            wrap="nowrap"
            py="2"
            px="3"
            align="center"
            className={styles.bottomControlsRow}
          >
            <span className={styles.bottomModelControl}>
              <ChatSettingsDropdown
                disabled={isBuddyChat}
                onOpenChange={handleComposerMenuOpenChange}
              />
            </span>
            <span className={styles.bottomModeControl}>
              <ModeSelect
                selectedMode={threadMode ?? DEFAULT_MODE}
                onModeChange={onSetMode}
                disabled={isBuddyChat || isModeDisabled}
                onOpenChange={handleComposerMenuOpenChange}
              />
            </span>
            <span className={styles.bottomWorkspaceControl}>
              <WorktreeControl
                disabled={isBuddyChat}
                onOpenChange={handleComposerMenuOpenChange}
              />
            </span>

            <Flex
              justify="end"
              wrap="nowrap"
              gap="2"
              align="center"
              className={styles.bottomActionControls}
            >
              <div className={styles.actionControlsSwap}>
                <div
                  className={classNames(
                    styles.controlsSwapItem,
                    styles.expandedActionsSet,
                  )}
                >
                  <span className={styles.hideActionFirst}>
                    <BrowserToggleButton chatId={chatId} />
                  </span>
                  <span className={styles.hideActionSecond}>
                    <AutoEnrichmentToggleButton
                      disabled={isStreaming || isWaiting}
                    />
                  </span>
                  <span className={styles.hideActionThird}>
                    <AutoCompactToggleButton
                      disabled={isStreaming || isWaiting}
                    />
                  </span>
                  <span className={styles.hideActionFourth}>
                    <WandButton
                      currentText={value}
                      disabled={isStreaming || isWaiting}
                      onUpdateText={handleChange}
                    />
                  </span>
                  {onClose && (
                    <span className={styles.hideActionFifth}>
                      <BackToSideBarButton
                        disabled={isStreaming}
                        title="Return to sidebar"
                        onClick={onClose}
                      />
                    </span>
                  )}
                  {config.features?.images !== false &&
                    isMultimodalitySupportedForCurrentModel && (
                      <span className={styles.hideActionSixth}>
                        <AttachImagesButton />
                      </span>
                    )}
                  <span className={styles.hideActionSeventh}>
                    <MicrophoneButton
                      ref={microphoneRef}
                      onTranscript={(text) => {
                        setValue((prev) => {
                          if (prev.trim()) {
                            return `${prev}\n${text}`;
                          }
                          return text;
                        });
                      }}
                      onLiveTranscript={handleLiveTranscript}
                      onRecordingChange={handleRecordingChange}
                      disabled={disableMicrophone}
                    />
                  </span>
                  <span className={styles.hideActionSeventh}>
                    <ThreadInfoButton
                      chatId={chatId}
                      onOpenChange={handleComposerMenuOpenChange}
                    />
                  </span>
                </div>
                <div
                  className={classNames(
                    styles.controlsSwapItem,
                    styles.collapsedStatusSet,
                  )}
                  data-composer-no-expand="true"
                  data-testid="composer-collapsed-status"
                >
                  <span className={styles.hideCollapsedStatusFirst}>
                    <StreamingTokenCounter />
                  </span>
                  <span className={styles.hideCollapsedStatusSecond}>
                    <ProviderUsageIndicator />
                  </span>
                  <UsageCounter />
                </div>
              </div>
              <span data-composer-no-expand="true">
                <UnifiedSendButton
                  disabled={isVoiceActive || !isOnline || allDisabled}
                  isStreaming={isStreaming || isWaiting}
                  hasText={
                    value.trim().length > 0 ||
                    attachedImages.length > 0 ||
                    textFiles.length > 0
                  }
                  hasMessages={messages.length > 0}
                  queuedCount={queuedItems.length}
                  onSend={() => handleSubmit("after_flow")}
                  onSendImmediately={handleSendImmediately}
                  onStop={() => void abort()}
                  onResend={() => void regenerate()}
                />
              </span>
            </Flex>
          </Flex>
        </Form>
      </div>
    </div>
  );
};
