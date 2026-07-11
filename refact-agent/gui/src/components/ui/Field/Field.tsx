import classNames from "classnames";
import { Check, LoaderCircle, TriangleAlert } from "lucide-react";
import React, { useId } from "react";

import { Icon } from "../Icon";
import { Select } from "../Select";
import type { SelectProps } from "../Select";
import { Slider } from "../Slider";
import type { SliderProps } from "../Slider";
import { Switch } from "../Switch";
import type { SwitchProps } from "../Switch";
import styles from "./Field.module.css";

export type SaveStatusState = "idle" | "saving" | "saved" | "error";

type FieldLayout = "stack" | "row";

export interface FieldProps extends React.ComponentProps<"div"> {
  label?: React.ReactNode;
  htmlFor?: string;
  helper?: React.ReactNode;
  error?: React.ReactNode;
  required?: boolean;
  layout?: FieldLayout;
  control?: React.ReactNode;
  children?: React.ReactNode;
}

export type FieldRowProps = Omit<FieldProps, "layout">;
export type FieldStackProps = Omit<FieldProps, "layout">;

export interface FieldErrorProps
  extends Omit<React.ComponentProps<"p">, "children" | "className"> {
  children?: React.ReactNode;
  className?: string;
}

export interface SaveStatusProps extends React.ComponentProps<"span"> {
  state?: SaveStatusState;
  label?: string;
}

export interface FieldTextProps
  extends Omit<React.ComponentProps<"input">, "onChange" | "value"> {
  value: string;
  onChange: (value: string) => void;
  onCommit?: (value: string) => void;
}

export interface FieldTextareaProps
  extends Omit<React.ComponentProps<"textarea">, "onChange" | "value"> {
  value: string;
  onChange: (value: string) => void;
  onCommit?: (value: string) => void;
}

export interface FieldSelectOption {
  value: string;
  label: React.ReactNode;
  disabled?: boolean;
}

export interface FieldSelectProps
  extends Omit<SelectProps, "children" | "onValueChange"> {
  value: string;
  options: FieldSelectOption[];
  placeholder?: string;
  id?: string;
  "aria-label"?: string;
  onChange: (value: string) => void;
  onCommit?: (value: string) => void;
}

export interface FieldSwitchProps
  extends Omit<
    SwitchProps,
    "checked" | "label" | "onChange" | "onCheckedChange"
  > {
  checked: boolean;
  onChange: (checked: boolean) => void;
  onCommit?: (checked: boolean) => void;
}

export interface FieldSliderProps
  extends Omit<
    SliderProps,
    "onChange" | "onValueChange" | "onValueCommit" | "value"
  > {
  value: number[];
  onChange: (value: number[]) => void;
  onCommit?: (value: number[]) => void;
}

function FieldBase({
  children,
  className,
  control,
  error,
  helper,
  htmlFor,
  label,
  layout = "stack",
  required = false,
  ...props
}: FieldProps) {
  const descriptionId = useId();
  const errorId = useId();

  return (
    <div
      {...props}
      className={classNames(styles.field, styles[layout], className)}
      data-invalid={error ? true : undefined}
    >
      {label ? (
        <div className={styles.labelBlock}>
          <label className={styles.label} htmlFor={htmlFor}>
            {label}
            {required ? <span className={styles.required}>*</span> : null}
          </label>
          {helper ? (
            <p className={styles.helper} id={descriptionId}>
              {helper}
            </p>
          ) : null}
        </div>
      ) : null}
      <div className={styles.controlBlock}>
        {control ?? children}
        {error ? <FieldError id={errorId}>{error}</FieldError> : null}
      </div>
    </div>
  );
}

export function Field(props: FieldProps) {
  return <FieldBase {...props} />;
}

export function FieldRow(props: FieldRowProps) {
  return <FieldBase {...props} layout="row" />;
}

export function FieldStack(props: FieldStackProps) {
  return <FieldBase {...props} layout="stack" />;
}

export function FieldError({ children, className, ...props }: FieldErrorProps) {
  return (
    <p {...props} className={classNames(styles.error, className)} role="alert">
      {children}
    </p>
  );
}

export function SaveStatus({
  className,
  label,
  state = "idle",
  ...props
}: SaveStatusProps) {
  if (state === "idle") {
    return null;
  }

  const statusLabel = label ?? getSaveStatusLabel(state);
  const icon = getSaveStatusIcon(state);

  return (
    <span
      {...props}
      aria-live="polite"
      className={classNames(
        styles.saveStatus,
        styles[`saveStatus-${state}`],
        className,
      )}
      role="status"
    >
      <Icon
        icon={icon}
        size="sm"
        tone={
          state === "error" ? "danger" : state === "saved" ? "success" : "muted"
        }
      />
      {statusLabel}
    </span>
  );
}

export const FieldText = React.forwardRef<HTMLInputElement, FieldTextProps>(
  (
    { className, onBlur, onChange, onCommit, type = "text", value, ...props },
    ref,
  ) => (
    <input
      {...props}
      ref={ref}
      className={classNames(styles.input, className)}
      type={type}
      value={value}
      onBlur={(event) => {
        onBlur?.(event);
        onCommit?.(event.currentTarget.value);
      }}
      onChange={(event) => onChange(event.currentTarget.value)}
    />
  ),
);
FieldText.displayName = "FieldText";

export const FieldTextarea = React.forwardRef<
  HTMLTextAreaElement,
  FieldTextareaProps
>(({ className, onBlur, onChange, onCommit, value, ...props }, ref) => (
  <textarea
    {...props}
    ref={ref}
    className={classNames(styles.textarea, className)}
    value={value}
    onBlur={(event) => {
      onBlur?.(event);
      onCommit?.(event.currentTarget.value);
    }}
    onChange={(event) => onChange(event.currentTarget.value)}
  />
));
FieldTextarea.displayName = "FieldTextarea";

export function FieldSelect({
  onChange,
  onCommit,
  options,
  placeholder,
  id,
  "aria-label": ariaLabel,
  ...props
}: FieldSelectProps) {
  return (
    <Select
      {...props}
      onValueChange={(nextValue) => {
        onChange(nextValue);
        onCommit?.(nextValue);
      }}
    >
      <Select.Trigger
        aria-label={ariaLabel}
        className={styles.selectTrigger}
        id={id}
        placeholder={placeholder}
      />
      <Select.Content
        maxHeight="calc(var(--rf-control-h) * 8 + var(--rf-space-2))"
        maxWidth="var(--rf-input-max, 360px)"
      >
        {options.map((option) => (
          <Select.Item
            disabled={option.disabled}
            key={option.value}
            value={option.value}
          >
            {option.label}
          </Select.Item>
        ))}
      </Select.Content>
    </Select>
  );
}

export const FieldSwitch = React.forwardRef<
  HTMLButtonElement,
  FieldSwitchProps
>(({ checked, className, onChange, onCommit, ...props }, ref) => (
  <Switch
    {...props}
    ref={ref}
    checked={checked}
    className={classNames(styles.switchControl, className)}
    onCheckedChange={(nextChecked) => {
      onChange(nextChecked);
      onCommit?.(nextChecked);
    }}
  />
));
FieldSwitch.displayName = "FieldSwitch";

export const FieldSlider = React.forwardRef<HTMLSpanElement, FieldSliderProps>(
  ({ className, onChange, onCommit, value, ...props }, ref) => (
    <Slider
      {...props}
      ref={ref}
      className={classNames(styles.sliderControl, className)}
      value={value}
      onValueChange={onChange}
      onValueCommit={onCommit}
    />
  ),
);
FieldSlider.displayName = "FieldSlider";

function getSaveStatusLabel(state: Exclude<SaveStatusState, "idle">): string {
  if (state === "saving") {
    return "Saving…";
  }

  if (state === "saved") {
    return "Saved";
  }

  return "Error";
}

function getSaveStatusIcon(state: Exclude<SaveStatusState, "idle">) {
  if (state === "saving") {
    return LoaderCircle;
  }

  if (state === "saved") {
    return Check;
  }

  return TriangleAlert;
}
