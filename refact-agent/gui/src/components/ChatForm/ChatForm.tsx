import React, { useCallback, useEffect, useMemo } from "react";

import { Flex, Box, Text } from "@radix-ui/themes";
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

import {
  BackToSideBarButton,
  UnifiedSendButton,
  BrowserToggleButton,
  WandButton,
  AutoEnrichmentToggleButton,
  AutoCompactToggleButton,
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
import { selectThreadConfirmation } from "../../features/Chat";
import { AttachImagesButton } from "../Dropzone";
import { MicrophoneButton, MicrophoneButtonRef } from "./MicrophoneButton";
import { useAttachedImages } from "../../hooks/useAttachedImages";
import {
  selectChatErrorById,
  selectCurrentThreadId,
  selectIsStreaming,
  selectIsWaiting,
  selectMessages,
  selectQueuedItems,
  selectThreadImages,
  selectThreadMode,
  selectManualPreviewItems,
  removeManualPreviewItem,
  setThreadMode,
  DEFAULT_MODE,
  selectIsBuddyChat,
} from "../../features/Chat";
import { useReportErrorMutation } from "../../services/refact/buddy";

import { useUsageCounter } from "../UsageCounter/useUsageCounter";
import { ChatInputTopControls } from "./ChatInputTopControls";

import classNames from "classnames";

export type SendPolicy = "immediate" | "after_flow";

export type ChatFormProps = {
  onSubmit: (str: string, sendPolicy?: SendPolicy) => void;
  onClose?: () => void;
  className?: string;
};

export const ChatForm: React.FC<ChatFormProps> = ({
  onSubmit,
  onClose,
  className,
}) => {
  const dispatch = useAppDispatch();
  const isStreaming = useAppSelector(selectIsStreaming);
  const isWaiting = useAppSelector(selectIsWaiting);
  const caps = useCapsForToolUse();
  const { isMultimodalitySupportedForCurrentModel } = caps;
  const config = useConfig();
  const host = useAppSelector(selectHost);
  const { queryPathThenOpenFile } = useEventsBusForIDE();
  const globalError = useAppSelector(getErrorMessage);
  const chatId = useAppSelector(selectCurrentThreadId);
  const chatError = useAppSelector((state) =>
    selectChatErrorById(state, chatId),
  );
  const isBuddyChat = useAppSelector((state) =>
    selectIsBuddyChat(state, chatId),
  );
  const information = useAppSelector(getInformationMessage);
  const pauseReasonsWithPause = useAppSelector(selectThreadConfirmation);
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
  const isOnline = useIsOnline();
  const { isContextFull } = useUsageCounter();
  const messages = useAppSelector(selectMessages);
  const queuedItems = useAppSelector(selectQueuedItems);
  const threadMode = useAppSelector(selectThreadMode);
  const manualPreviewItems = useAppSelector(selectManualPreviewItems);
  const autoFocus = useAutoFocusOnce();
  const { abort, regenerate } = useChatActions();
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
  const attachedImages = useAppSelector(selectThreadImages);
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
    (sendPolicy: SendPolicy = "after_flow") => {
      const trimmedValue = value.trim();
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

  const handleComposerPointerDownCapture = useCallback(
    (event: React.PointerEvent<HTMLFormElement>) => {
      const target = event.target;
      if (!(target instanceof Element)) return;
      if (isEditableElement(target)) return;

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
        const activeElement = document.activeElement;
        if (isInsideComposerSurface(activeElement, root)) return;
        setIsComposerExpanded(openComposerMenus > 0);
      }, 0);
    },
    [openComposerMenus],
  );

  const handleComposerFocusCapture = useCallback(() => {
    setIsComposerExpanded(true);
  }, []);

  useEffect(() => {
    if (openComposerMenus > 0) {
      setIsComposerExpanded(true);
    }
  }, [openComposerMenus]);

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
        if (target instanceof Element && isEditableElement(target)) return;
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
    <Box
      ref={composerRef}
      onBlur={handleComposerBlur}
      onFocusCapture={handleComposerFocusCapture}
      style={{ flexShrink: 0, position: "relative" }}
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

      <Flex
        style={{
          flexDirection: "column",
          alignSelf: "stretch",
          flex: 1,
          width: "100%",
        }}
      >
        {helpInfo && (
          <Flex mb="3" direction="column">
            {helpInfo}
          </Flex>
        )}
        <Form
          disabled={disableSend}
          className={classNames(
            styles.chatForm,
            styles.chatForm__form,
            styles.chatFormMain,
            isComposerExpanded
              ? styles.chatFormExpanded
              : styles.chatFormCollapsed,
            className,
          )}
          onClick={() => {
            if (!isComposerExpanded) {
              focusComposerInput();
            }
          }}
          onPointerDownCapture={handleComposerPointerDownCapture}
          onSubmit={() => handleSubmit("after_flow")}
        >
          <Box
            className={styles.expandedComposerContent}
            onFocus={() => setIsComposerExpanded(true)}
          >
            <Box className={styles.expandedComposerContentInner}>
              <Box className={styles.textareaWrapper}>
                <Box className={styles.inputHeader}>
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
                </Box>

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
                      style={{ boxShadow: "none", outline: "none" }}
                      onPaste={handlePastingFile}
                    />
                  )}
                />
              </Box>
            </Box>
          </Box>
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
              <span className={styles.hideActionFirst}>
                <BrowserToggleButton chatId={chatId} />
              </span>
              <span className={styles.hideActionSecond}>
                <AutoEnrichmentToggleButton
                  disabled={isStreaming || isWaiting}
                />
              </span>
              <span className={styles.hideActionThird}>
                <AutoCompactToggleButton disabled={isStreaming || isWaiting} />
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
            </Flex>
          </Flex>
        </Form>
      </Flex>
    </Box>
  );
};
