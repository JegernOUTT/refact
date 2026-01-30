import React, { useCallback, useMemo } from "react";
import {
  Text,
  Flex,
  HoverCard,
  Link,
  Skeleton,
  Box,
  Button,
} from "@radix-ui/themes";
import { Select, type SelectProps } from "../Select";
import { type Config } from "../../features/Config/configSlice";
import { TruncateLeft } from "../Text";
import styles from "./ChatForm.module.css";
import classNames from "classnames";
import { PromptSelect } from "./PromptSelect";
import { Checkbox } from "../Checkbox";
import {
  LockClosedIcon,
  LockOpen1Icon,
  QuestionMarkCircledIcon,
} from "@radix-ui/react-icons";
import { useTourRefs } from "../../features/Tour";
import { ModeSelect } from "./ModeSelect";
import {
  selectChatId,
  selectIsStreaming,
  selectIsWaiting,
  selectMessages,
  selectThreadMode,
  setThreadMode,
} from "../../features/Chat/Thread";
import { DEFAULT_MODE } from "../../features/Chat/Thread/types";
import { useAppSelector, useAppDispatch, useCapsForToolUse } from "../../hooks";
import { useAttachedFiles } from "./useCheckBoxes";
import { push } from "../../features/Pages/pagesSlice";
import { RichModelSelectItem } from "../Select/RichModelSelectItem";
import { enrichAndGroupModels } from "../../utils/enrichModels";

export const CapsSelect: React.FC<{ disabled?: boolean }> = ({ disabled }) => {
  const refs = useTourRefs();
  const caps = useCapsForToolUse();
  const dispatch = useAppDispatch();

  const handleAddNewModelClick = useCallback(() => {
    dispatch(push({ name: "providers page" }));
  }, [dispatch]);

  const onSelectChange = useCallback(
    (value: string) => {
      if (value === "add-new-model") {
        handleAddNewModelClick();
        return;
      }
      caps.setCapModel(value);
    },
    [handleAddNewModelClick, caps],
  );

  const optionsWithToolTips: SelectProps["options"] = useMemo(() => {
    const groupedModels = enrichAndGroupModels(
      caps.usableModelsForPlan,
      caps.data,
    );

    if (groupedModels.length === 0) {
      return [
        ...caps.usableModelsForPlan,
        { type: "separator" },
        {
          value: "add-new-model",
          textValue: "Add new model",
        },
      ];
    }

    const flatOptions: SelectProps["options"] = [];
    groupedModels.forEach((group, index) => {
      if (index > 0) {
        flatOptions.push({ type: "separator" });
      }
      group.models.forEach((model) => {
        flatOptions.push({
          value: model.value,
          textValue: model.displayName,
          disabled: model.disabled,
          children: (
            <RichModelSelectItem
              displayName={model.displayName}
              pricing={model.pricing}
              nCtx={model.nCtx}
              capabilities={model.capabilities}
              isDefault={model.isDefault}
              isThinking={model.isThinking}
              isLight={model.isLight}
            />
          ),
        });
      });
    });

    return [
      ...flatOptions,
      { type: "separator" },
      {
        value: "add-new-model",
        textValue: "Add new model",
      },
    ];
  }, [caps.data, caps.usableModelsForPlan]);

  const allDisabled = caps.usableModelsForPlan.every((option) => {
    if (typeof option === "string") return false;
    return option.disabled;
  });

  return (
    <Flex
      gap="2"
      align="center"
      wrap="wrap"
      ref={(x) => refs.setUseModel(x)}
    >
      <Skeleton loading={caps.loading}>
        <Box>
          {allDisabled ? (
            <Text size="1" color="gray">
              No models available
            </Text>
          ) : (
            <Select
              title="chat model"
              options={optionsWithToolTips}
              value={caps.currentModel}
              onChange={onSelectChange}
              disabled={disabled}
            />
          )}
        </Box>
      </Skeleton>
    </Flex>
  );
};

type CheckboxHelp = {
  text: string;
  link?: string;
  linkText?: string;
};

export type Checkbox = {
  name: string;
  label: string;
  checked: boolean;
  value?: string;
  disabled: boolean;
  fileName?: string;
  hide?: boolean;
  info?: CheckboxHelp;
  locked?: boolean;
};

export type ChatControlsProps = {
  checkboxes: Record<string, Checkbox>;
  onCheckedChange: (
    name: keyof ChatControlsProps["checkboxes"],
    checked: boolean | string,
  ) => void;

  host: Config["host"];
  attachedFiles: ReturnType<typeof useAttachedFiles>;
};

const ChatControlCheckBox: React.FC<{
  name: string;
  checked: boolean;
  disabled?: boolean;
  onCheckChange: (value: boolean | string) => void;
  label: string;
  fileName?: string;
  infoText?: string;
  href?: string;
  linkText?: string;
  locked?: boolean;
}> = ({
  name,
  checked,
  disabled,
  onCheckChange,
  label,
  fileName,
  infoText,
  href,
  linkText,
  locked,
}) => {
  return (
    <Flex justify="between">
      <Checkbox
        size="1"
        name={name}
        checked={checked}
        disabled={disabled}
        onCheckedChange={onCheckChange}
      >
        {label}
        {fileName && (
          <Flex ml="-3px">
            <TruncateLeft>{fileName}</TruncateLeft>
          </Flex>
        )}
        {locked && <LockClosedIcon opacity="0.6" />}
        {locked === false && <LockOpen1Icon opacity="0.6" />}
      </Checkbox>
      {infoText && (
        <HoverCard.Root>
          <HoverCard.Trigger>
            <QuestionMarkCircledIcon style={{ marginLeft: 4 }} />
          </HoverCard.Trigger>
          <HoverCard.Content maxWidth="240px" size="1">
            <Flex direction="column" gap="4">
              <Text as="div" size="1">
                {infoText}
              </Text>

              {href && linkText && (
                <Text size="1">
                  Read more on our{" "}
                  <Link size="1" href={href}>
                    {linkText}
                  </Link>
                </Text>
              )}
            </Flex>
          </HoverCard.Content>
        </HoverCard.Root>
      )}
    </Flex>
  );
};

export const ChatControls: React.FC<ChatControlsProps> = ({
  checkboxes,
  onCheckedChange,
  host,
  attachedFiles,
}) => {
  const refs = useTourRefs();
  const dispatch = useAppDispatch();
  const isStreaming = useAppSelector(selectIsStreaming);
  const isWaiting = useAppSelector(selectIsWaiting);
  const messages = useAppSelector(selectMessages);
  const chatId = useAppSelector(selectChatId);
  const threadMode = useAppSelector(selectThreadMode);
  const onSetMode = useCallback(
    (modeId: string) => dispatch(setThreadMode({ chatId, mode: modeId })),
    [dispatch, chatId],
  );

  const showControls = useMemo(
    () => messages.length === 0 && !isStreaming && !isWaiting,
    [isStreaming, isWaiting, messages],
  );

  return (
    <Flex
      pt="2"
      pb="2"
      gap="2"
      direction="column"
      align="start"
      className={classNames(styles.controls)}
    >
      {Object.entries(checkboxes).map(([key, checkbox]) => {
        if (host === "web" && checkbox.name === "file_upload") {
          return null;
        }
        if (checkbox.hide === true) {
          return null;
        }
        return (
          <ChatControlCheckBox
            key={key}
            name={checkbox.name}
            label={checkbox.label}
            checked={checkbox.checked}
            disabled={checkbox.disabled}
            onCheckChange={(value) => onCheckedChange(key, value)}
            infoText={checkbox.info?.text}
            href={checkbox.info?.link}
            linkText={checkbox.info?.linkText}
            fileName={checkbox.fileName}
            locked={checkbox.locked}
          />
        );
      })}

      {host !== "web" && (
        <Button
          title="Attach current file"
          onClick={attachedFiles.addFile}
          disabled={!attachedFiles.activeFile.name || attachedFiles.attached}
          size="1"
          radius="medium"
        >
          Attach: {attachedFiles.activeFile.name}
        </Button>
      )}

      {showControls && (
        <Flex gap="2" direction="column" ref={(x) => refs.setUseTools(x)}>
          <ModeSelect
            selectedMode={threadMode ?? DEFAULT_MODE}
            onModeChange={onSetMode}
          />
          <PromptSelect />
        </Flex>
      )}
    </Flex>
  );
};
