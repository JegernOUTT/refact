import React, { useCallback, useEffect, useMemo, useRef } from "react";
import { useComboboxStore, Combobox } from "@ariakit/react";
import { getAnchorRect, getTriggerOffset, replaceRange } from "./utils";
import type { TextAreaProps } from "../TextArea/TextArea";
import { Item } from "./Item";
import { Portal } from "../Portal";
import { Popover } from "./Popover";
import { TruncateLeft } from "../Text";
import { type DebouncedState } from "usehooks-ts";
import { CommandCompletionResponse } from "../../services/refact";
import { useAppSelector, useEventsBusForIDE } from "../../hooks";
import { SlashCommandSuggestion } from "../SlashCommands";
import { selectSubmitOption } from "../../features/Config/configSlice";
import {
  nextPlaceholder,
  parseHintPlaceholders,
  placeholderAt,
  previousPlaceholder,
  selectionIsPlaceholder,
  type PlaceholderRange,
} from "./argumentPlaceholders";

const TRIGGER_CHARS = ["@", "/"];

function isCompletionAcceptKey(key: string) {
  return key === "Tab" || key === "Enter";
}

function isSlashCommandCompletion(
  commands: CommandCompletionResponse,
  command: string,
): boolean {
  return (
    command.startsWith("/") &&
    commands.completion_details?.[command] !== undefined
  );
}

export type ComboBoxProps = {
  commands: CommandCompletionResponse;
  onChange: (value: string) => void;
  value: string;
  onSubmit: React.KeyboardEventHandler<HTMLTextAreaElement>;
  onArgumentPlaceholdersChange?: (placeholders: string[]) => void;
  placeholder?: string;
  render: (props: TextAreaProps) => React.ReactElement;
  requestCommandsCompletion: DebouncedState<
    (query: string, cursor: number) => void
  >;
  onHelpClick: () => void;
};

export const ComboBox: React.FC<ComboBoxProps> = ({
  commands,
  onSubmit,
  onArgumentPlaceholdersChange,
  placeholder,
  onChange,
  value,
  render,
  requestCommandsCompletion,
  onHelpClick,
}) => {
  const ref = React.useRef<HTMLTextAreaElement>(null);
  const [pendingSelection, setPendingSelection] =
    React.useState<PlaceholderRange | null>(null);
  const [lastPasteTimestamp, setLastPasteTimestamp] = React.useState(0);
  const shiftEnterToSubmit = useAppSelector(selectSubmitOption);
  const { escapeKeyPressed } = useEventsBusForIDE();

  const argModeActiveRef = useRef(false);
  const insertedPlaceholdersRef = useRef<string[]>([]);
  const suppressOpenValueRef = useRef<string | null>(null);
  const dismissedTriggerRef = useRef<number | null>(null);

  const combobox = useComboboxStore({
    defaultOpen: false,
    placement: "top-start",
    defaultActiveId: undefined,
  });

  const state = combobox.useState();

  const matches = commands.completions;

  const hasMatches = useMemo(() => {
    return matches.length > 0;
  }, [matches]);

  React.useEffect(() => {
    if (pendingSelection === null) return;
    if (ref.current) {
      ref.current.focus();
      ref.current.setSelectionRange(
        pendingSelection.start,
        pendingSelection.end,
      );
    }
    setPendingSelection(null);
  }, [pendingSelection]);

  React.useLayoutEffect(() => {
    let suppress = false;
    if (
      suppressOpenValueRef.current !== null &&
      value === suppressOpenValueRef.current
    ) {
      suppress = true;
    }
    if (!suppress && dismissedTriggerRef.current !== null && ref.current) {
      const triggerOffset = getTriggerOffset(ref.current, TRIGGER_CHARS);
      if (triggerOffset === dismissedTriggerRef.current) suppress = true;
      else dismissedTriggerRef.current = null;
    }
    combobox.setOpen(hasMatches && !suppress);
  }, [combobox, hasMatches, matches, value]);

  React.useEffect(() => {
    combobox.render();
  }, [combobox, value, matches]);

  React.useEffect(() => {
    if (!ref.current) return;
    const cursor = Math.min(
      ref.current.selectionStart,
      ref.current.selectionEnd,
    );
    requestCommandsCompletion(value, cursor);
  }, [requestCommandsCompletion, value]);

  const closeCombobox = useCallback(() => {
    combobox.hide();
    combobox.setState("items", []);
    combobox.setState("activeId", null);
    combobox.setState("activeValue", undefined);
  }, [combobox]);

  const reportPlaceholders = useCallback(
    (nextValue: string) => {
      const remaining = insertedPlaceholdersRef.current.filter((token) =>
        nextValue.includes(token),
      );
      if (remaining.length === 0) argModeActiveRef.current = false;
      onArgumentPlaceholdersChange?.(remaining);
    },
    [onArgumentPlaceholdersChange],
  );

  const replaceWith = useCallback(
    (input: string): string | null => {
      if (!ref.current) return null;
      const nextValue = replaceRange(
        ref.current.value,
        commands.replace,
        input,
      );
      closeCombobox();
      requestCommandsCompletion.cancel();
      onChange(nextValue);
      return nextValue;
    },
    [closeCombobox, commands.replace, onChange, requestCommandsCompletion],
  );

  const acceptCompletion = useCallback(
    (command: string) => {
      if (!ref.current) return;

      if (command === "@help") {
        replaceWith(command);
        closeCombobox();
        onHelpClick();
        return;
      }

      if (!isSlashCommandCompletion(commands, command)) {
        const nextValue = replaceWith(command);
        if (nextValue !== null) {
          const caret =
            Math.min(commands.replace[0], commands.replace[1]) + command.length;
          setPendingSelection({ start: caret, end: caret });
        }
        return;
      }

      const replaceStart = Math.min(commands.replace[0], commands.replace[1]);
      const hint =
        commands.completion_details?.[command]?.argument_hint?.trim();

      if (hint) {
        const inserted = `${command} ${hint}`;
        const nextValue = replaceWith(inserted);
        if (nextValue === null) return;
        const tokens = parseHintPlaceholders(hint);
        insertedPlaceholdersRef.current = tokens;
        argModeActiveRef.current = tokens.length > 0;
        suppressOpenValueRef.current = nextValue;
        const first = nextPlaceholder(nextValue, replaceStart + command.length);
        const caret = replaceStart + inserted.length;
        setPendingSelection(first ?? { start: caret, end: caret });
        reportPlaceholders(nextValue);
        return;
      }

      const inserted = `${command} `;
      const nextValue = replaceWith(inserted);
      if (nextValue === null) return;
      insertedPlaceholdersRef.current = [];
      argModeActiveRef.current = false;
      suppressOpenValueRef.current = nextValue;
      const caret = replaceStart + inserted.length;
      setPendingSelection({ start: caret, end: caret });
      reportPlaceholders(nextValue);
    },
    [closeCombobox, commands, onHelpClick, replaceWith, reportPlaceholders],
  );

  const navigatePlaceholders = useCallback(
    (event: React.KeyboardEvent<HTMLTextAreaElement>): boolean => {
      const textarea = ref.current;
      if (!textarea || !argModeActiveRef.current) return false;

      const selectionStart = textarea.selectionStart;
      const selectionEnd = textarea.selectionEnd;
      const onPlaceholder = selectionIsPlaceholder(
        textarea.value,
        selectionStart,
        selectionEnd,
      );

      const store = combobox.getState();
      const wantForward =
        (event.key === "Tab" && !event.shiftKey) ||
        (event.key === "Enter" && !event.shiftKey && !store.open) ||
        (event.key === "ArrowRight" && onPlaceholder);
      const wantBackward =
        (event.key === "Tab" && event.shiftKey) ||
        (event.key === "ArrowLeft" && onPlaceholder);

      if (wantForward) {
        const next = nextPlaceholder(textarea.value, selectionEnd);
        if (next) {
          event.preventDefault();
          event.stopPropagation();
          setPendingSelection(next);
          return true;
        }
        argModeActiveRef.current = false;
        if (event.key === "Tab") {
          event.preventDefault();
          setPendingSelection({ start: selectionEnd, end: selectionEnd });
          return true;
        }
        return false;
      }

      if (wantBackward) {
        const prev = previousPlaceholder(textarea.value, selectionStart);
        if (prev) {
          event.preventDefault();
          event.stopPropagation();
          setPendingSelection(prev);
          return true;
        }
        if (event.key === "Tab") {
          event.preventDefault();
          return true;
        }
      }

      return false;
    },
    [combobox],
  );

  const onKeyDown = useCallback(
    (event: React.KeyboardEvent<HTMLTextAreaElement>) => {
      if (navigatePlaceholders(event)) return;

      const store = combobox.getState();

      if (store.open && isCompletionAcceptKey(event.key)) {
        event.preventDefault();
      }
      if (store.open) return;

      if (!shiftEnterToSubmit && event.key === "Enter" && !event.shiftKey) {
        event.stopPropagation();
        onSubmit(event);
        setPendingSelection(null);
        return;
      }
      if (shiftEnterToSubmit && event.key === "Enter" && event.shiftKey) {
        event.stopPropagation();
        onSubmit(event);
        setPendingSelection(null);
        return;
      }

      if (!shiftEnterToSubmit && event.key === "Enter" && event.shiftKey) {
        return;
      }
      if (shiftEnterToSubmit && event.key === "Enter" && !event.shiftKey) {
        onChange(value + "\n");
        return;
      }
    },
    [
      combobox,
      navigatePlaceholders,
      onChange,
      onSubmit,
      shiftEnterToSubmit,
      value,
    ],
  );

  const onKeyUp = useCallback(
    (event: React.KeyboardEvent<HTMLTextAreaElement>) => {
      if (!ref.current) return;
      const store = combobox.getState();

      const wasArrowLeftOrRight =
        event.key === "ArrowLeft" || event.key === "ArrowRight";
      if (wasArrowLeftOrRight && store.open) {
        closeCombobox();
      }

      const activeItemValue = combobox.item(store.activeId)?.value;
      const selectedItemValue =
        activeItemValue && matches.includes(activeItemValue)
          ? activeItemValue
          : undefined;
      const activeValue =
        typeof store.activeValue === "string" &&
        matches.includes(store.activeValue)
          ? store.activeValue
          : undefined;
      const command = selectedItemValue ?? activeValue ?? matches[0];

      if (store.open && isCompletionAcceptKey(event.key) && command) {
        event.preventDefault();
        event.stopPropagation();
        acceptCompletion(command);
      }

      if (event.key === "Escape") {
        const triggerOffset = getTriggerOffset(ref.current, TRIGGER_CHARS);
        if (triggerOffset >= 0) dismissedTriggerRef.current = triggerOffset;
        argModeActiveRef.current = false;
        closeCombobox();
        escapeKeyPressed("combobox");
      }
    },
    [acceptCompletion, matches, closeCombobox, escapeKeyPressed, combobox],
  );

  const handleChange = useCallback(
    (event: React.ChangeEvent<HTMLTextAreaElement>) => {
      const newValue = event.target.value;
      const nativeEvent = event.nativeEvent as InputEvent;
      const currentEventTimestamp = nativeEvent.timeStamp;

      const inputType = nativeEvent.inputType;
      const isPasteEvent = [
        "insertFromPaste",
        "insertFromDrop",
        "insertFromYank",
        "insertReplacementText",
      ].includes(inputType);

      const timeSinceLastChange = currentEventTimestamp - lastPasteTimestamp;

      if (isPasteEvent && timeSinceLastChange < 100) return;

      if (isPasteEvent) {
        setLastPasteTimestamp(currentEventTimestamp);
        closeCombobox();
        requestCommandsCompletion.cancel();
      }
      onChange(newValue);
      suppressOpenValueRef.current = null;

      if (newValue.length === 0) {
        insertedPlaceholdersRef.current = [];
        argModeActiveRef.current = false;
        dismissedTriggerRef.current = null;
        onArgumentPlaceholdersChange?.([]);
      } else if (insertedPlaceholdersRef.current.length > 0) {
        reportPlaceholders(newValue);
      }
    },
    [
      onChange,
      closeCombobox,
      requestCommandsCompletion,
      lastPasteTimestamp,
      onArgumentPlaceholdersChange,
      reportPlaceholders,
    ],
  );

  const handleClick = useCallback(() => {
    const textarea = ref.current;
    if (!textarea || !argModeActiveRef.current) return;
    if (textarea.selectionStart !== textarea.selectionEnd) return;
    const range = placeholderAt(textarea.value, textarea.selectionStart);
    if (range) setPendingSelection(range);
  }, []);

  const onItemClick = useCallback(
    (item: string, event: React.MouseEvent<HTMLDivElement>) => {
      event.stopPropagation();
      event.preventDefault();
      acceptCompletion(item);
    },
    [acceptCompletion],
  );

  const popoverWidth = ref.current
    ? ref.current.getBoundingClientRect().width - 8
    : null;

  useEffect(() => {
    const maybeItem = combobox.item(state.activeId);
    if (state.open && maybeItem === null) {
      const first = combobox.first();
      if (combobox.item(first)) {
        combobox.setActiveId(first);
      }
    }
  }, [combobox, state]);

  return (
    <>
      <Combobox
        store={combobox}
        autoSelect
        value={value}
        showOnChange={false}
        showOnKeyDown={false}
        showOnMouseDown={false}
        setValueOnChange={false}
        render={render({
          ref,
          placeholder,
          onScroll: combobox.render,
          onPointerDown: combobox.hide,
          onChange: handleChange,
          onKeyUp: onKeyUp,
          onKeyDown: onKeyDown,
          onClick: handleClick,
          onSubmit: onSubmit,
        })}
      />
      <Portal>
        <Popover
          store={combobox}
          hidden={!hasMatches}
          getAnchorRect={() => {
            const textarea = ref.current;
            if (!textarea) return null;
            return getAnchorRect(textarea, ["@", "/", " "]);
          }}
          maxWidth={popoverWidth}
        >
          {matches.map((item, index) => {
            const detail = commands.completion_details?.[item];
            return (
              <Item
                key={item + "-" + index}
                value={item}
                onClick={(e) => onItemClick(item, e)}
              >
                {detail ? (
                  <SlashCommandSuggestion name={item} detail={detail} />
                ) : (
                  <TruncateLeft>{item}</TruncateLeft>
                )}
              </Item>
            );
          })}
        </Popover>
      </Portal>
    </>
  );
};
