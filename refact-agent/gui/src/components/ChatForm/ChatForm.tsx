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

import {
  BackToSideBarButton,
  AgentIntegrationsButton,
  UnifiedSendButton,
  BrowserToggleButton,
} from "../Buttons";
import { StreamingTokenCounter, UsageCounter } from "../UsageCounter";
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
} from "../../hooks";
import { ErrorCallout, Callout } from "../Callout";
import { ComboBox } from "../ComboBox";
import { UnifiedAttachmentsTray } from "./UnifiedAttachmentsTray";
import { ChatSettingsDropdown } from "./ChatSettingsDropdown";
import { ModeSelect } from "./ModeSelect";
import { addCheckboxValuesToInput } from "./utils";
import { useCommandCompletionAndPreviewFiles } from "./useCommandCompletionAndPreviewFiles";
import { useAppSelector, useAppDispatch } from "../../hooks";
import {
  clearError,
  getErrorMessage,
  getErrorType,
} from "../../features/Errors/errorsSlice";
import { useAttachedFiles, useCheckboxes } from "./useCheckBoxes";
import { useInputValue } from "./useInputValue";
import {
  clearInformation,
  getInformationMessage,
  showBalanceLowCallout,
} from "../../features/Errors/informationSlice";
import {
  BallanceCallOut,
  BallanceLowInformation,
  InformationCallout,
} from "../Callout/Callout";
import { ToolConfirmation } from "./ToolConfirmation";
import { selectThreadConfirmation } from "../../features/Chat";
import { AttachImagesButton } from "../Dropzone";
import { MicrophoneButton, MicrophoneButtonRef } from "./MicrophoneButton";
import { useAttachedImages } from "../../hooks/useAttachedImages";
import {
  clearChatError,
  selectChatError,
  selectCurrentThreadId,
  selectIsStreaming,
  selectIsWaiting,
  selectMessages,
  selectQueuedItems,
  selectThreadImages,
  selectThreadMode,
  setThreadMode,
  DEFAULT_MODE,
} from "../../features/Chat";
import { telemetryApi } from "../../services/refact";
import { push } from "../../features/Pages/pagesSlice";

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
  const globalErrorType = useAppSelector(getErrorType);
  const chatError = useAppSelector(selectChatError);
  const chatId = useAppSelector(selectCurrentThreadId);
  const information = useAppSelector(getInformationMessage);
  const pauseReasonsWithPause = useAppSelector(selectThreadConfirmation);
  const [helpInfo, setHelpInfo] = React.useState<React.ReactNode | null>(null);
  const [isVoiceActive, setIsVoiceActive] = React.useState(false);
  const [liveTranscript, setLiveTranscript] = React.useState("");
  const [inputResetKey, setInputResetKey] = React.useState(0);
  const isOnline = useIsOnline();
  const { isContextFull } = useUsageCounter();
  const messages = useAppSelector(selectMessages);
  const queuedItems = useAppSelector(selectQueuedItems);
  const threadMode = useAppSelector(selectThreadMode);
  const autoFocus = useAutoFocusOnce();
  const { abort, regenerate } = useChatActions();

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
  const shouldShowBalanceLow = useAppSelector(showBalanceLowCallout);
  const attachedImages = useAppSelector(selectThreadImages);
  const microphoneRef = React.useRef<MicrophoneButtonRef>(null);

  const onClearError = useCallback(() => {
    dispatch(clearError());
    if (chatId) {
      dispatch(clearChatError({ id: chatId }));
    }
  }, [dispatch, chatId]);

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

  const [sendTelemetryEvent] =
    telemetryApi.useLazySendTelemetryChatEventQuery();

  const [value, setValue, isSendImmediately, setIsSendImmediately] =
    useInputValue(() => unCheckAll());

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

  const handleAgentIntegrationsClick = useCallback(() => {
    dispatch(push({ name: "integrations page" }));
    void sendTelemetryEvent({
      scope: `openIntegrations`,
      success: true,
      error_message: "",
    });
  }, [dispatch, sendTelemetryEvent]);

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

  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.ctrlKey && event.shiftKey && event.code === "Space") {
        event.preventDefault();
        if (!disableMicrophone && microphoneRef.current) {
          void microphoneRef.current.toggleRecording();
        }
      }
    };

    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [disableMicrophone]);

  if (globalError) {
    return (
      <Flex direction="column" mt="2" gap="2">
        <ErrorCallout onClick={onClearError} timeout={null}>
          {globalError}
        </ErrorCallout>
      </Flex>
    );
  }

  if (chatError) {
    return (
      <Flex direction="column" mt="2" gap="2">
        <ErrorCallout onClick={onClearError} timeout={null}>
          {chatError}
        </ErrorCallout>
      </Flex>
    );
  }

  if (information) {
    return (
      <InformationCallout mt="2" onClick={onClearInformation} timeout={2000}>
        {information}
      </InformationCallout>
    );
  }

  if (pauseReasonsWithPause.pause) {
    return (
      <ToolConfirmation pauseReasons={pauseReasonsWithPause.pause_reasons} />
    );
  }

  return (
    <Box style={{ flexShrink: 0, position: "relative" }}>
      {globalErrorType === "balance" && (
        <BallanceCallOut
          mt="0"
          mb="2"
          mx="0"
          onClick={() => dispatch(clearError())}
        />
      )}
      {shouldShowBalanceLow && <BallanceLowInformation mt="0" mb="2" mx="0" />}
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
            className,
          )}
          onSubmit={() => handleSubmit("after_flow")}
        >
          <Box className={styles.textareaWrapper}>
            <Box className={styles.inputHeader}>
              <UnifiedAttachmentsTray
                attachedFiles={attachedFiles}
                previewFiles={previewFiles}
                onOpenFile={queryPathThenOpenFile}
              />
              <Flex align="center" gap="2" justify="between" wrap="wrap">
                <ChatInputTopControls
                  checkboxes={checkboxes}
                  onCheckedChange={onToggleCheckbox}
                  attachedFiles={attachedFiles}
                />
                <Flex align="center" gap="2">
                  <StreamingTokenCounter />
                  <UsageCounter />
                  <TrajectoryButton />
                </Flex>
              </Flex>
            </Box>

            <ComboBox
              key={inputResetKey}
              onHelpClick={handleHelpCommand}
              commands={commands}
              requestCommandsCompletion={requestCompletion}
              value={
                isVoiceActive && liveTranscript
                  ? value.trim()
                    ? `${value}\n${liveTranscript}`
                    : liveTranscript
                  : value
              }
              onChange={handleChange}
              onSubmit={(event) => {
                handleEnter(event);
              }}
              placeholder={
                isVoiceActive
                  ? "Listening..."
                  : commands.completions.length < 1
                    ? "Type @ for commands"
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
                  style={{ boxShadow: "none", outline: "none" }}
                  onPaste={handlePastingFile}
                />
              )}
            />
          </Box>
          <Flex gap="2" wrap="wrap" py="2" px="3" align="center">
            <ChatSettingsDropdown />
            <ModeSelect
              selectedMode={threadMode ?? DEFAULT_MODE}
              onModeChange={onSetMode}
              disabled={isModeDisabled}
            />

            <Flex justify="end" flexGrow="1" wrap="wrap" gap="2" align="center">
              <BrowserToggleButton chatId={chatId} />
              <AgentIntegrationsButton
                title="Set up Agent Integrations"
                onClick={handleAgentIntegrationsClick}
              />
              {onClose && (
                <BackToSideBarButton
                  disabled={isStreaming}
                  title="Return to sidebar"
                  onClick={onClose}
                />
              )}
              {config.features?.images !== false &&
                isMultimodalitySupportedForCurrentModel && (
                  <AttachImagesButton />
                )}
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
              <UnifiedSendButton
                disabled={isVoiceActive || !isOnline || allDisabled}
                isStreaming={isStreaming || isWaiting}
                hasText={value.trim().length > 0 || attachedImages.length > 0}
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
