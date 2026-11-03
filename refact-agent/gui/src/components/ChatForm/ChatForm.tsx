import React, { useCallback, useEffect, useMemo } from "react";
import { useDebounceCallback } from "usehooks-ts";

import { Flex, Text } from "@radix-ui/themes";
import { IconButton } from "../ui";
import styles from "./ChatForm.module.css";
import {
  attachmentFileError,
  isSupportedImageFile,
  isSupportedTextFile,
} from "../../utils/attachmentFiles";

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
import { useEventsBusForIDE } from "../../hooks/useEventBusForIDE";
import { Form } from "./Form";
import { useOnPressedEnter } from "../../hooks/useOnPressedEnter";
import { useIsOnline } from "../../hooks/useIsOnline";
import { useConfig } from "../../hooks/useConfig";
import { useCapsForToolUse } from "../../hooks/useCapsForToolUse";
import { useAutoFocusOnce } from "../../hooks/useAutoFocusOnce";
import { useChatActions } from "../../hooks/useChatActions";
import { useFirstSendAutoFlip } from "../../hooks/useFirstSendAutoFlip";
import { Callout } from "../Callout";
import { ComboBox } from "../ComboBox";
import { UnifiedAttachmentsTray } from "./UnifiedAttachmentsTray";
import { ChatSettingsDropdown } from "./ChatSettingsDropdown";
import { ModeSelect } from "./ModeSelect";
import { WorktreeControl } from "../../features/Worktrees";
import { addCheckboxValuesToInput } from "./utils";
import { stripUnfilledPlaceholders } from "../ComboBox/argumentPlaceholders";
import { useCommandCompletionAndPreviewFiles } from "./useCommandCompletionAndPreviewFiles";
import { useAppSelector } from "../../hooks/useAppSelector";
import { useAppDispatch } from "../../hooks/useAppDispatch";
import {
  clearError,
  getRouteErrorMessage,
} from "../../features/Errors/errorsSlice";
import { useAttachedFiles, useCheckboxes } from "./useCheckBoxes";
import { useInputValue } from "./useInputValue";
import {
  clearInformation,
  getInformationMessage,
} from "../../features/Errors/informationSlice";
import { ErrorCallout, InformationCallout } from "../Callout";
import { ToolConfirmation } from "./ToolConfirmation";
import { selectThreadConfirmationById } from "../../features/Chat/Thread";
import { AttachImagesButton } from "../Dropzone";
import { MicrophoneButton, MicrophoneButtonRef } from "./MicrophoneButton";
import { useAttachedImages } from "../../hooks/useAttachedImages";
import {
  selectChatErrorById,
  selectContextRebuildMessageById,
  selectContextRebuildRequiredById,
  selectHasMessagesById,
  selectIsStreamingById,
  selectRecoveryById,
  selectIsWaitingById,
  selectQueuedItemsById,
  selectThreadImagesById,
  selectThreadModeById,
  selectManualPreviewItemsById,
  removeManualPreviewItem,
  setThreadMode,
  DEFAULT_MODE,
  selectIsBuddyChat,
  selectIsCompanionChat,
  useThreadId,
  clearChatError,
} from "../../features/Chat/Thread";
import { useReportErrorMutation } from "../../services/refact/buddy";

import { useUsageCounter } from "../UsageCounter/useUsageCounter";
import { ChatInputTopControls } from "./ChatInputTopControls";

import classNames from "classnames";
import { Maximize2, Minimize2 } from "lucide-react";
import { ComposerToolbar } from "./ComposerToolbar";
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
  const recoveryActive = useAppSelector((state) => {
    const recovery = selectRecoveryById(state, chatId);
    return recovery?.state === "waiting" || recovery?.state === "attempting";
  });
  const caps = useCapsForToolUse();
  const { isMultimodalitySupportedForCurrentModel } = caps;
  const config = useConfig();
  const host = useAppSelector(selectHost);
  const { queryPathThenOpenFile } = useEventsBusForIDE();
  const globalError = useAppSelector(getRouteErrorMessage);
  const chatError = useAppSelector((state) =>
    selectChatErrorById(state, chatId),
  );
  const contextRebuildRequired = useAppSelector((state) =>
    selectContextRebuildRequiredById(state, chatId),
  );
  const contextRebuildMessage = useAppSelector((state) =>
    selectContextRebuildMessageById(state, chatId),
  );
  const isBuddyChat = useAppSelector((state) =>
    selectIsBuddyChat(state, chatId),
  );
  const isCompanionChat = useAppSelector((state) =>
    selectIsCompanionChat(state, chatId),
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
  const editorId = React.useId();
  const isOnline = useIsOnline();
  const { isContextFull, isWarning } = useUsageCounter();
  const hasMessages = useAppSelector((state) =>
    selectHasMessagesById(state, chatId),
  );
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
    if (contextRebuildRequired) return true;
    if (allDisabled) return true;
    if (!hasMessages) return false;
    if (isContextFull) return true;
    return isWaiting || isStreaming || !isOnline;
  }, [
    contextRebuildRequired,
    allDisabled,
    hasMessages,
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
    setError,
    textFiles,
    resetAllTextFiles,
  } = useAttachedImages();
  const handlePastingFile = useCallback(
    (event: React.ClipboardEvent<HTMLTextAreaElement>) => {
      if (isCompanionChat) {
        const hasFile = Array.from(event.clipboardData.items).some(
          (item) => item.kind === "file",
        );
        if (hasFile) event.preventDefault();
        return;
      }
      const imageFiles: File[] = [];
      const textFilesList: File[] = [];
      const items = event.clipboardData.items;
      let handledFile = false;

      for (const item of items) {
        if (item.kind === "file") {
          const file = item.getAsFile();
          if (file) {
            handledFile = true;
            const validationError = attachmentFileError(file);
            if (validationError) {
              setError(validationError);
            } else if (isSupportedImageFile(file)) {
              if (isMultimodalitySupportedForCurrentModel) {
                imageFiles.push(file);
              } else {
                setError("Current model does not support images");
              }
            } else if (isSupportedTextFile(file)) {
              textFilesList.push(file);
            }
          }
        }
      }

      if (handledFile) {
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
      isCompanionChat,
      processAndInsertImages,
      processAndInsertTextFiles,
      isMultimodalitySupportedForCurrentModel,
      setError,
    ],
  );

  const visibleError = chatError ?? globalError;
  const clearVisibleError = useCallback(() => {
    if (chatError) {
      dispatch(clearChatError({ id: chatId }));
    } else {
      dispatch(clearError());
    }
  }, [chatError, chatId, dispatch]);

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

  const ignoreCompletion = useDebounceCallback(() => undefined, 500);

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
        (isCompanionChat || !allDisabled) &&
        !contextRebuildRequired;

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
      }
    },
    [
      value,
      contextRebuildRequired,
      isCompanionChat,
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
    window.requestAnimationFrame(() => {
      composerRef.current?.querySelector("textarea")?.focus();
    });
  }, []);

  const handleComposerMenuOpenChange = useCallback((open: boolean) => {
    setOpenComposerMenus((count) => Math.max(0, count + (open ? 1 : -1)));
  }, []);

  useEffect(() => {
    onExpandedChange?.(isComposerExpanded);
  }, [isComposerExpanded, onExpandedChange]);

  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      const active = document.activeElement;
      const root = composerRef.current;
      if (
        !root?.contains(active) &&
        active instanceof Element &&
        active.closest('[role="dialog"], form')
      )
        return;
      if (event.ctrlKey && event.shiftKey && event.code === "Space") {
        event.preventDefault();
        if (
          (!isCompanionChat || isVoiceActive) &&
          !disableMicrophone &&
          microphoneRef.current
        ) {
          void microphoneRef.current.toggleRecording();
        }
      }

      if (
        !isCompanionChat &&
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
  }, [disableMicrophone, focusComposerInput, isCompanionChat, isVoiceActive]);

  if (pauseReasonsWithPause.pause) {
    return (
      <ToolConfirmation pauseReasons={pauseReasonsWithPause.pause_reasons} />
    );
  }

  return (
    <div ref={composerRef} className={styles.composerRoot}>
      {visibleError && (
        <ErrorCallout timeout={3000} onClick={clearVisibleError} preventRetry>
          {visibleError}
        </ErrorCallout>
      )}
      {!globalError && !chatError && information && (
        <InformationCallout onClick={onClearInformation} timeout={2000}>
          {information}
        </InformationCallout>
      )}
      {!isOnline && (
        <Callout type="info">
          Oops, seems that connection was lost... Check your internet connection
        </Callout>
      )}
      {contextRebuildRequired && (
        <Flex
          role="status"
          align="center"
          justify="between"
          gap="2"
          wrap="wrap"
          px="3"
          py="2"
          data-composer-no-expand="true"
        >
          <Text size="2">
            {contextRebuildMessage ??
              "Rebuild the context to continue generating."}
          </Text>
          <TrajectoryButton
            label="Rebuild context"
            initialTab="llm-compress"
            chatId={chatId}
            disabled={isBuddyChat}
          />
        </Flex>
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
          onSubmit={() => handleSubmit("after_flow")}
        >
          <ChatInputTopControls
            checkboxes={checkboxes}
            onCheckedChange={onToggleCheckbox}
            attachedFiles={attachedFiles}
            disabled={isBuddyChat}
            renderControls={(permissions, dangerous) => (
              <>
                {dangerous && (
                  <Text role="status" size="1" color="red">
                    Dangerous commands auto-approved
                  </Text>
                )}
                {isWarning && (
                  <Text role="status" size="1" color="red">
                    {isContextFull
                      ? "Context is full"
                      : "Context usage is high"}
                  </Text>
                )}
                <ComposerToolbar
                  menuCount={openComposerMenus}
                  editor={
                    <div id={editorId} className={styles.textareaWrapper}>
                      <IconButton
                        className={styles.expansionButton}
                        icon={isComposerExpanded ? Minimize2 : Maximize2}
                        variant="plain"
                        size="sm"
                        type="button"
                        aria-label={
                          isComposerExpanded
                            ? "Collapse composer"
                            : "Expand composer"
                        }
                        aria-expanded={isComposerExpanded}
                        aria-controls={editorId}
                        data-composer-expansion="true"
                        onClick={() =>
                          setIsComposerExpanded((expanded) => !expanded)
                        }
                      />
                      <div className={styles.inputHeader}>
                        <UnifiedAttachmentsTray
                          attachedFiles={attachedFiles}
                          previewFiles={previewFiles}
                          manualPreviewItems={manualPreviewItems}
                          onRemoveManualPreviewItem={
                            chatId
                              ? (index) =>
                                  dispatch(
                                    removeManualPreviewItem({ chatId, index }),
                                  )
                              : undefined
                          }
                          onOpenFile={queryPathThenOpenFile}
                        />
                      </div>

                      <ComboBox
                        key={inputResetKey}
                        onHelpClick={handleHelpCommand}
                        commands={commands}
                        requestCommandsCompletion={
                          isCompanionChat ? ignoreCompletion : requestCompletion
                        }
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
                            autoFocus={autoFocus}
                            readOnly={isVoiceActive}
                            onPaste={handlePastingFile}
                          />
                        )}
                      />
                    </div>
                  }
                  items={[
                    ...Object.entries(permissions).map(
                      ([id, content], index) => ({
                        id,
                        content,
                        row: "top" as const,
                        priority: index === 0 ? 0 : index + 1,
                        hidden: !isComposerExpanded || isBuddyChat,
                      }),
                    ),
                    {
                      id: "model",
                      row: "bottom",
                      priority: isComposerExpanded ? 9 : 3,
                      hidden: isBuddyChat,
                      content: (
                        <ChatSettingsDropdown
                          disabled={isBuddyChat}
                          onOpenChange={handleComposerMenuOpenChange}
                        />
                      ),
                    },
                    {
                      id: "mode",
                      row: "bottom",
                      priority: isComposerExpanded ? 10 : 5,
                      hidden: isBuddyChat,
                      content: (
                        <ModeSelect
                          selectedMode={threadMode ?? DEFAULT_MODE}
                          onModeChange={onSetMode}
                          disabled={isBuddyChat || isModeDisabled}
                          onOpenChange={handleComposerMenuOpenChange}
                        />
                      ),
                    },
                    {
                      id: "worktree",
                      row: "bottom",
                      priority: isComposerExpanded ? 8 : 2,
                      hidden: isBuddyChat,
                      content: (
                        <WorktreeControl
                          disabled={isBuddyChat}
                          onOpenChange={handleComposerMenuOpenChange}
                        />
                      ),
                    },
                    {
                      id: "quota",
                      row: isComposerExpanded ? "top" : "bottom",
                      side: "right",
                      priority: isComposerExpanded ? 1 : 0,
                      content: (
                        <ProviderUsageIndicator
                          onOpenChange={handleComposerMenuOpenChange}
                        />
                      ),
                    },
                    {
                      id: "streaming",
                      row: isComposerExpanded ? "top" : "bottom",
                      side: "right",
                      priority: 4,
                      content: <StreamingTokenCounter />,
                    },
                    {
                      id: "context",
                      row: isComposerExpanded ? "top" : "bottom",
                      side: "right",
                      pinned: isWarning,
                      priority: isComposerExpanded ? 5 : 4,
                      content: (
                        <UsageCounter
                          onOpenChange={handleComposerMenuOpenChange}
                        />
                      ),
                    },
                    {
                      id: "trajectory",
                      row: isComposerExpanded ? "top" : "bottom",
                      side: "right",
                      pinned: isWarning,
                      priority: isComposerExpanded ? 4 : 1,
                      hidden: isCompanionChat,
                      content: (
                        <TrajectoryButton
                          disabled={isBuddyChat}
                          onOpenChange={handleComposerMenuOpenChange}
                        />
                      ),
                    },
                    {
                      id: "browser",
                      row: "bottom",
                      side: "right",
                      priority: 2,
                      hidden: !isComposerExpanded || isCompanionChat,
                      content: (
                        <BrowserToggleButton
                          chatId={chatId}
                          disabled={isBuddyChat}
                        />
                      ),
                    },
                    {
                      id: "enrichment",
                      row: "bottom",
                      side: "right",
                      priority: 3,
                      hidden: !isComposerExpanded || isCompanionChat,
                      content: (
                        <AutoEnrichmentToggleButton
                          disabled={isStreaming || isWaiting}
                        />
                      ),
                    },
                    {
                      id: "autoCompact",
                      row: "bottom",
                      side: "right",
                      priority: 4,
                      hidden: !isComposerExpanded || isCompanionChat,
                      content: (
                        <AutoCompactToggleButton disabled={isStreaming} />
                      ),
                    },
                    {
                      id: "wand",
                      row: "bottom",
                      side: "right",
                      priority: 1,
                      hidden: !isComposerExpanded || isCompanionChat,
                      content: (
                        <WandButton
                          currentText={value}
                          disabled={isStreaming || isWaiting}
                          onUpdateText={handleChange}
                        />
                      ),
                    },
                    {
                      id: "sidebar",
                      row: "bottom",
                      side: "right",
                      priority: 0.5,
                      hidden: !isComposerExpanded || !onClose,
                      content: (
                        <BackToSideBarButton
                          disabled={isStreaming}
                          title="Return to sidebar"
                          onClick={onClose}
                        />
                      ),
                    },
                    {
                      id: "image",
                      row: "bottom",
                      side: "right",
                      priority: 6,
                      hidden:
                        !isComposerExpanded ||
                        isCompanionChat ||
                        config.features?.images === false ||
                        !isMultimodalitySupportedForCurrentModel,
                      content: <AttachImagesButton />,
                    },
                    {
                      id: "mic",
                      row: "bottom",
                      side: "right",
                      priority: 5,
                      pinned: isVoiceActive,
                      hidden:
                        !isVoiceActive &&
                        (!isComposerExpanded || isCompanionChat),
                      content: (
                        <MicrophoneButton
                          ref={microphoneRef}
                          onTranscript={(text) =>
                            setValue((prev) =>
                              prev.trim() ? `${prev}\n${text}` : text,
                            )
                          }
                          onLiveTranscript={handleLiveTranscript}
                          onRecordingChange={handleRecordingChange}
                          disabled={
                            !isVoiceActive &&
                            (disableMicrophone || isCompanionChat)
                          }
                        />
                      ),
                    },
                    {
                      id: "threadInfo",
                      row: "bottom",
                      side: "right",
                      priority: 0,
                      hidden: !isComposerExpanded || isCompanionChat,
                      content: (
                        <ThreadInfoButton
                          chatId={chatId}
                          onOpenChange={handleComposerMenuOpenChange}
                        />
                      ),
                    },
                    {
                      id: "send",
                      row: "bottom",
                      side: "right",
                      priority: 100,
                      pinned: true,
                      content: (
                        <UnifiedSendButton
                          disabled={
                            isVoiceActive ||
                            !isOnline ||
                            (!isCompanionChat && allDisabled) ||
                            contextRebuildRequired
                          }
                          isStreaming={
                            isStreaming || isWaiting || recoveryActive
                          }
                          hasText={
                            value.trim().length > 0 ||
                            attachedImages.length > 0 ||
                            textFiles.length > 0
                          }
                          hasMessages={hasMessages && !isCompanionChat}
                          queuedCount={queuedItems.length}
                          onSend={() => handleSubmit("after_flow")}
                          onSendImmediately={handleSendImmediately}
                          onStop={() => void abort()}
                          onResend={() => void regenerate()}
                        />
                      ),
                    },
                  ]}
                />
              </>
            )}
          />
        </Form>
      </div>
    </div>
  );
};
