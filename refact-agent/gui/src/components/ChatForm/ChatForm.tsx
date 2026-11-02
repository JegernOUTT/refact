import React, { useCallback, useEffect, useMemo } from "react";
import { useStore } from "react-redux";
import type { AppStore } from "../../app/store";
import {
  captureSubmittedAttachments,
  clearSubmittedAttachments,
} from "./chatSubmission";
import { useDebounceCallback } from "usehooks-ts";

import { Flex, Text } from "@radix-ui/themes";
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
  useThreadId,
  clearChatError,
} from "../../features/Chat/Thread";
import { useReportErrorMutation } from "../../services/refact/buddy";

import { useUsageCounter } from "../UsageCounter/useUsageCounter";
import { ChatInputTopControls } from "./ChatInputTopControls";

import classNames from "classnames";
import { ComposerToolbar } from "./ComposerToolbar";
type ComposerHelpProps = {
  children: React.ReactNode;
};

const ComposerHelp: React.FC<ComposerHelpProps> = ({ children }) => (
  <div className={styles.helpText}>{children}</div>
);

export type SendPolicy = "immediate" | "after_flow";

export type ChatFormProps = {
  onSubmit:
    | ((str: string, sendPolicy?: SendPolicy) => void)
    | ((
        str: string,
        sendPolicy?: SendPolicy,
      ) => boolean | Promise<boolean | undefined>);
  draft?: { value: string; onChange: (value: string) => void };
  submissionDisabled?: boolean;
  settingsLocked?: boolean;
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
  draft,
  submissionDisabled = false,
  settingsLocked = false,
}) => {
  const dispatch = useAppDispatch();
  const store = useStore.withTypes<AppStore>()();
  const submitting = React.useRef(false);
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
  const isBuddyChat =
    useAppSelector((state) => selectIsBuddyChat(state, chatId)) ||
    settingsLocked;
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
  const attachedFiles = useAttachedFiles(!draft);
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
  } = useAttachedImages();
  const handlePastingFile = useCallback(
    (event: React.ClipboardEvent<HTMLTextAreaElement>) => {
      if (settingsLocked) return;
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
      settingsLocked,
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
  } = useCheckboxes(!draft);

  const [storedValue, setStoredValue, isSendImmediately, setIsSendImmediately] =
    useInputValue(() => unCheckAll(), !draft);
  const value = draft?.value ?? storedValue;
  const setValue = useCallback(
    (next: React.SetStateAction<string>) => {
      if (draft)
        draft.onChange(typeof next === "function" ? next(draft.value) : next);
      else setStoredValue(next);
    },
    [draft, setStoredValue],
  );

  const displayedInputValue =
    isVoiceActive && liveTranscript
      ? value.trim()
        ? `${value}\n${liveTranscript}`
        : liveTranscript
      : value;

  useEffect(() => {
    if (value.trim().length > 0) setIsComposerExpanded(true);
  }, [value]);

  const valueRef = React.useRef(value);
  valueRef.current = value;
  const draftRef = React.useRef(draft);
  draftRef.current = draft;

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
        (settingsLocked || !allDisabled) &&
        !contextRebuildRequired &&
        !submissionDisabled &&
        !submitting.current &&
        !pauseReasonsWithPause.pause;

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
        const submitted = captureSubmittedAttachments(store, chatId);
        const cleanup = (accepted: boolean | undefined) => {
          if (accepted === false) return;
          clearSubmittedAttachments(store, chatId, {
            ...submitted,
            images: accepted === true ? submitted.images : [],
          });
          attachedFiles.files.forEach(attachedFiles.removeFile);
          if (valueRef.current !== inputValue) return;
          argumentPlaceholdersRef.current = [];
          if (draftRef.current) draftRef.current.onChange("");
          else setStoredValue("");
          setInputResetKey((k) => k + 1);
          unCheckAll();
          setIsComposerExpanded(false);
        };
        const result = onSubmit(valueIncludingChecks, sendPolicy);
        if (result instanceof Promise) {
          submitting.current = true;
          void result
            .then(cleanup, () => undefined)
            .finally(() => {
              submitting.current = false;
            });
        } else cleanup(typeof result === "boolean" ? result : undefined);
      }
    },
    [
      value,
      contextRebuildRequired,
      submissionDisabled,
      settingsLocked,
      pauseReasonsWithPause.pause,
      allDisabled,
      isOnline,
      attachedImages,
      textFiles,
      attachedFiles,
      checkboxes,
      setLineSelectionInteracted,
      store,
      chatId,
      onSubmit,
      setStoredValue,
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
        setIsComposerExpanded(
          openComposerMenus > 0 || valueRef.current.trim().length > 0,
        );
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
        setIsComposerExpanded(valueRef.current.trim().length > 0);
      }
    };

    document.addEventListener("pointerdown", handlePointerDown, true);
    return () =>
      document.removeEventListener("pointerdown", handlePointerDown, true);
  }, [isComposerExpanded, openComposerMenus]);

  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      const active = document.activeElement;
      const root = composerRef.current;
      if (draft && !root?.contains(active)) return;
      if (
        !root?.contains(active) &&
        active instanceof Element &&
        active.closest('[role="dialog"], form')
      )
        return;
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
  }, [disableMicrophone, focusComposerInput, draft]);

  return (
    <div
      ref={composerRef}
      className={styles.composerRoot}
      onBlur={handleComposerBlur}
      onFocusCapture={handleComposerFocusCapture}
    >
      {pauseReasonsWithPause.pause && (
        <ToolConfirmation pauseReasons={pauseReasonsWithPause.pause_reasons} />
      )}
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
                </div>

                <ComboBox
                  key={inputResetKey}
                  onHelpClick={handleHelpCommand}
                  commands={commands}
                  requestCommandsCompletion={
                    settingsLocked ? ignoreCompletion : requestCompletion
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
                      autoFocus={isComposerExpanded && autoFocus}
                      readOnly={isVoiceActive}
                      onPaste={handlePastingFile}
                    />
                  )}
                />
              </div>
            </div>
          </div>
          <ComposerToolbar
            menuCount={openComposerMenus}
            onOpenChange={handleComposerMenuOpenChange}
            items={[
              {
                id: "model",
                width: 208,
                content: (
                  <ChatSettingsDropdown
                    disabled={isBuddyChat}
                    onOpenChange={handleComposerMenuOpenChange}
                  />
                ),
              },
              {
                id: "mode",
                width: 220,
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
                width: 260,
                content: (
                  <WorktreeControl
                    disabled={isBuddyChat}
                    onOpenChange={handleComposerMenuOpenChange}
                  />
                ),
              },
              {
                id: "permissions",
                width: 200,
                content: (
                  <ChatInputTopControls
                    checkboxes={checkboxes}
                    onCheckedChange={onToggleCheckbox}
                    attachedFiles={attachedFiles}
                    disabled={isBuddyChat}
                  />
                ),
              },
              {
                id: "actions",
                width: 340,
                content: settingsLocked ? null : (
                  <div className={styles.toolbarActions}>
                    <TrajectoryButton
                      disabled={isBuddyChat}
                      onOpenChange={handleComposerMenuOpenChange}
                    />{" "}
                    <span>
                      <BrowserToggleButton
                        chatId={chatId}
                        disabled={isBuddyChat}
                      />
                    </span>
                    <span>
                      <AutoEnrichmentToggleButton
                        disabled={isStreaming || isWaiting}
                      />
                    </span>
                    <span>
                      <AutoCompactToggleButton disabled={isStreaming} />
                    </span>
                    <span>
                      <WandButton
                        currentText={value}
                        disabled={isStreaming || isWaiting}
                        onUpdateText={handleChange}
                      />
                    </span>
                    {onClose && (
                      <span>
                        <BackToSideBarButton
                          disabled={isStreaming}
                          title="Return to sidebar"
                          onClick={onClose}
                        />
                      </span>
                    )}
                    {config.features?.images !== false &&
                      isMultimodalitySupportedForCurrentModel && (
                        <span>
                          <AttachImagesButton />
                        </span>
                      )}
                    <span>
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
                        disabled={disableMicrophone || settingsLocked}
                      />
                    </span>
                    <span>
                      <ThreadInfoButton
                        chatId={chatId}
                        onOpenChange={handleComposerMenuOpenChange}
                      />
                    </span>
                  </div>
                ),
              },
              {
                id: "usage",
                width: 160,
                content: (
                  <div
                    className={styles.toolbarActions}
                    data-composer-no-expand="true"
                    data-testid="composer-collapsed-status"
                  >
                    <StreamingTokenCounter />
                    <ProviderUsageIndicator
                      onOpenChange={handleComposerMenuOpenChange}
                    />
                    <UsageCounter onOpenChange={handleComposerMenuOpenChange} />
                  </div>
                ),
              },
            ]}
            send={
              <UnifiedSendButton
                disabled={
                  submissionDisabled ||
                  pauseReasonsWithPause.pause ||
                  isVoiceActive ||
                  !isOnline ||
                  (!settingsLocked && allDisabled) ||
                  contextRebuildRequired
                }
                isStreaming={isStreaming || isWaiting || recoveryActive}
                hasText={
                  value.trim().length > 0 ||
                  attachedImages.length > 0 ||
                  textFiles.length > 0
                }
                hasMessages={hasMessages}
                queuedCount={queuedItems.length}
                onSend={() => handleSubmit("after_flow")}
                onSendImmediately={handleSendImmediately}
                onStop={() => void abort()}
                onResend={() => {
                  if (!submissionDisabled && !pauseReasonsWithPause.pause)
                    void regenerate();
                }}
              />
            }
          />
        </Form>
      </div>
    </div>
  );
};
