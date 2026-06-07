import React, { ReactNode, useMemo } from "react";
import classnames from "classnames";
import {
  Select as KitSelect,
  Tooltip,
  type SelectContentProps,
  type SelectItemProps,
  type SelectProps as KitSelectProps,
  type SelectSeparatorProps,
  type SelectTriggerProps,
} from "../ui";
import styles from "./select.module.css";

type SeparatorOption = { type: "separator"; key?: string };
function isSeparator(option: unknown): option is SeparatorOption {
  if (!option) return false;
  if (typeof option !== "object") return false;
  if (!("type" in option)) return false;
  return option.type === "separator";
}
export type SelectProps = Omit<KitSelectProps, "onValueChange"> & {
  onChange: (value: string) => void;
  options: (string | ItemProps | SeparatorOption)[];
  title?: string;
  contentPosition?: "item-aligned" | "popper";
  value?: string;
  disabled?: boolean;
};

export type SelectRootProps = KitSelectProps;
export const Root: React.FC<SelectRootProps> = KitSelect;

export type TriggerProps = SelectTriggerProps;
export const Trigger: React.FC<TriggerProps> = KitSelect.Trigger;

export type ContentProps = SelectContentProps;
export const Content: React.FC<ContentProps & { className?: string }> = (
  props,
) => (
  <KitSelect.Content
    {...props}
    className={classnames(styles.content, props.className)}
  />
);

export type ItemProps = SelectItemProps & {
  tooltip?: ReactNode;
};

export const Item: React.FC<ItemProps & { className?: string }> = (props) => (
  <KitSelect.Item
    {...props}
    className={classnames(styles.item, props.className)}
  />
);

export type SeparatorProps = SelectSeparatorProps;
export const Separator: React.FC<SeparatorProps> = KitSelect.Separator;

export const Select: React.FC<SelectProps> = ({
  title,
  options,
  onChange,
  contentPosition,
  ...props
}) => {
  const [isOpen, setIsOpen] = React.useState(
    props.open ?? props.defaultOpen ?? false,
  );
  const maybeSelectedOption = useMemo(() => {
    if (typeof props.value === "undefined") return null;
    const selectOption = options.find(
      (option) =>
        typeof option !== "string" &&
        !isSeparator(option) &&
        option.value === props.value,
    );
    if (!selectOption) return null;
    if (typeof selectOption === "string") return null;
    if (isSeparator(selectOption)) return null;
    return selectOption;
  }, [props.value, options]);

  return (
    <Root {...props} onValueChange={onChange} onOpenChange={setIsOpen}>
      {maybeSelectedOption && maybeSelectedOption.tooltip && !isOpen ? (
        <Tooltip delayDuration={1000}>
          <Tooltip.Trigger asChild>
            <span>
              <Trigger title={title} />
            </span>
          </Tooltip.Trigger>
          <Tooltip.Content>{maybeSelectedOption.tooltip}</Tooltip.Content>
        </Tooltip>
      ) : (
        <Trigger title={title} />
      )}
      <Content position={contentPosition ?? "popper"}>
        {options.map((option, index) => {
          if (typeof option === "string") {
            return (
              <Item key={`select-item-${index}-${option}`} value={option}>
                {option}
              </Item>
            );
          }
          if (isSeparator(option)) {
            return <Separator key={option.key ?? `separator-${index}`} />;
          }
          if (option.tooltip) {
            return (
              <Item key={`select-item-${index}-${option.value}`} {...option}>
                <Tooltip delayDuration={1000}>
                  <Tooltip.Trigger asChild>
                    <div>
                      <span className={styles.trigger_only}>
                        {option.textValue ?? option.value}
                      </span>
                      <span className={styles.dropdown_only}>
                        {option.children}
                      </span>
                    </div>
                  </Tooltip.Trigger>
                  <Tooltip.Content>{option.tooltip}</Tooltip.Content>
                </Tooltip>
              </Item>
            );
          }
          return (
            <Item key={`select-item-${index}-${option.value}`} {...option}>
              <span className={styles.trigger_only}>
                {option.textValue ?? option.value}
              </span>
              <span className={styles.dropdown_only}>{option.children}</span>
            </Item>
          );
        })}
      </Content>
    </Root>
  );
};
