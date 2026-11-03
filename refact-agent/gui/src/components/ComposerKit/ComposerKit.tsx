import React from "react";
import classNames from "classnames";
import * as DialogPrimitive from "@radix-ui/react-dialog";
import * as TabsPrimitive from "@radix-ui/react-tabs";
import {
  Check,
  ChevronDown,
  ChevronLeft,
  ChevronRight,
  Copy,
  Search,
  X,
} from "lucide-react";
import type { LucideIcon } from "lucide-react";

import { Portal } from "../Portal";
import { Slider, Switch, Tooltip } from "../ui";
import { ModalOverlayProvider } from "../ui/ModalOverlayContext";
import { overlayStyle } from "../ui/overlayTypes";
import dialogStyles from "../ui/Dialog/Dialog.module.css";
import styles from "./ComposerKit.module.css";

export type KitTone = "accent" | "success" | "warning" | "danger" | "muted";

export type ComposerTriggerProps = Omit<
  React.ButtonHTMLAttributes<HTMLButtonElement>,
  "type"
> & {
  icon?: LucideIcon;
  label?: React.ReactNode;
  chevron?: boolean;
  strong?: boolean;
  iconOnly?: boolean;
  tone?: "danger" | "warning";
};

export const ComposerTrigger = React.forwardRef<
  HTMLButtonElement,
  ComposerTriggerProps
>(
  (
    {
      icon: IconComponent,
      label,
      chevron = false,
      strong = false,
      iconOnly = false,
      tone,
      className,
      children,
      ...props
    },
    ref,
  ) => (
    <button
      ref={ref}
      type="button"
      data-tone={tone}
      className={classNames(
        styles.trigger,
        strong && styles.strong,
        iconOnly && styles.iconOnly,
        className,
      )}
      {...props}
    >
      {IconComponent ? <IconComponent aria-hidden="true" /> : null}
      {label !== undefined ? (
        <span className={styles.label}>{label}</span>
      ) : null}
      {children}
      {chevron ? (
        <ChevronDown aria-hidden="true" className={styles.chevron} />
      ) : null}
    </button>
  ),
);
ComposerTrigger.displayName = "ComposerTrigger";

export const KitTooltip: React.FC<{
  label: React.ReactNode;
  state?: React.ReactNode;
  stateTone?: KitTone;
  hint?: React.ReactNode;
  children: React.ReactElement;
}> = ({ label, state, stateTone, hint, children }) => (
  <Tooltip>
    <Tooltip.Trigger asChild>{children}</Tooltip.Trigger>
    <Tooltip.Content className={styles.tooltipChip}>
      <span className={styles.tooltip}>
        {label}
        {state !== undefined ? (
          <span className={styles.tooltipState} data-tone={stateTone}>
            {state}
          </span>
        ) : null}
      </span>
      {hint ? <span className={styles.tooltipHint}>{hint}</span> : null}
    </Tooltip.Content>
  </Tooltip>
);

type KitPanelProps = Omit<
  React.HTMLAttributes<HTMLDivElement>,
  "className" | "children"
> & { className?: string; children: React.ReactNode };

export const KitPanel: React.FC<KitPanelProps> = ({
  className,
  children,
  ...props
}) => (
  <div className={classNames(styles.panel, className)} {...props}>
    {children}
  </div>
);

export const KitHeader: React.FC<{
  title: React.ReactNode;
  meta?: React.ReactNode;
  metaStrong?: boolean;
  onBack?: () => void;
  backLabel?: string;
  children?: React.ReactNode;
}> = ({ title, meta, metaStrong, onBack, backLabel = "Back", children }) => (
  <div className={styles.header}>
    {onBack ? (
      <button
        type="button"
        className={styles.back}
        aria-label={backLabel}
        onClick={onBack}
      >
        <ChevronLeft aria-hidden="true" size={14} />
      </button>
    ) : null}
    <span className={styles.headerTitle}>{title}</span>
    {children}
    {meta !== undefined && meta !== null ? (
      <span
        className={styles.headerMeta}
        data-strong={metaStrong ? "true" : undefined}
      >
        {meta}
      </span>
    ) : null}
  </div>
);

type KitSearchProps = Omit<
  React.InputHTMLAttributes<HTMLInputElement>,
  "className"
> & { className?: string };

export const KitSearch = React.forwardRef<HTMLInputElement, KitSearchProps>(
  ({ className, ...props }, ref) => (
    <label className={classNames(styles.search, className)}>
      <Search aria-hidden="true" size={13} />
      <input ref={ref} {...props} />
    </label>
  ),
);
KitSearch.displayName = "KitSearch";

export const KitSection: React.FC<{ children: React.ReactNode }> = ({
  children,
}) => <div className={styles.section}>{children}</div>;

export const KitBadge: React.FC<{
  tone?: KitTone;
  children: React.ReactNode;
  className?: string;
}> = ({ tone, children, className }) => (
  <span className={classNames(styles.badge, className)} data-tone={tone}>
    {children}
  </span>
);

type KitRowContent = {
  icon?: React.ReactNode;
  label: React.ReactNode;
  badges?: React.ReactNode;
  sub?: React.ReactNode;
  hint?: React.ReactNode;
  meta?: React.ReactNode;
  selected?: boolean;
  strong?: boolean;
  tone?: "muted" | "danger";
  trailing?: React.ReactNode;
};

function rowInner({
  icon,
  label,
  badges,
  sub,
  hint,
  meta,
  selected,
  strong,
  trailing,
}: KitRowContent) {
  return (
    <>
      {icon ? <span className={styles.rowIcon}>{icon}</span> : null}
      <span className={styles.rowBody}>
        <span
          className={styles.rowLabel}
          data-strong={strong ? "true" : undefined}
        >
          <span className={styles.rowLabelText}>{label}</span>
          {badges}
        </span>
        {sub ? <span className={styles.rowSub}>{sub}</span> : null}
        {hint ? <span className={styles.rowHint}>{hint}</span> : null}
      </span>
      {meta !== undefined && meta !== null ? (
        <span className={styles.rowMeta}>{meta}</span>
      ) : null}
      {trailing}
      {selected ? (
        <span className={styles.rowCheck}>
          <Check aria-hidden="true" />
        </span>
      ) : null}
    </>
  );
}

export type KitRowProps = KitRowContent &
  Omit<React.ButtonHTMLAttributes<HTMLButtonElement>, "type" | "children">;

export const KitRow = React.forwardRef<HTMLButtonElement, KitRowProps>(
  (
    {
      icon,
      label,
      badges,
      sub,
      hint,
      meta,
      selected,
      strong,
      tone,
      trailing,
      className,
      ...props
    },
    ref,
  ) => (
    <button
      ref={ref}
      type="button"
      className={classNames(
        styles.row,
        sub ?? hint ? styles.rowTwoLine : undefined,
        className,
      )}
      data-selected={selected ? "true" : undefined}
      data-tone={tone}
      {...props}
    >
      {rowInner({
        icon,
        label,
        badges,
        sub,
        hint,
        meta,
        selected,
        strong,
        trailing,
      })}
    </button>
  ),
);
KitRow.displayName = "KitRow";

export const KitStaticRow: React.FC<
  KitRowContent & { className?: string; children?: React.ReactNode }
> = ({ tone, className, children, ...content }) => (
  <div
    className={classNames(
      styles.row,
      content.sub ?? content.hint ? styles.rowTwoLine : undefined,
      className,
    )}
    data-tone={tone}
  >
    {rowInner(content)}
    {children}
  </div>
);

export const KitToggleRow: React.FC<{
  label: React.ReactNode;
  hint?: React.ReactNode;
  icon?: React.ReactNode;
  meta?: React.ReactNode;
  trailing?: React.ReactNode;
  ariaLabel?: string;
  checked: boolean;
  disabled?: boolean;
  onCheckedChange: (checked: boolean) => void;
}> = ({
  label,
  hint,
  icon,
  meta,
  trailing,
  ariaLabel,
  checked,
  disabled,
  onCheckedChange,
}) => {
  const id = React.useId();
  return (
    <label
      className={classNames(styles.row, hint ? styles.rowTwoLine : undefined)}
      htmlFor={id}
    >
      {rowInner({ icon, label, hint, meta, trailing })}
      <Switch
        aria-label={ariaLabel}
        id={id}
        className={styles.switch}
        checked={checked}
        disabled={disabled}
        onCheckedChange={onCheckedChange}
      />
    </label>
  );
};

export const KitCheckRow: React.FC<{
  label: React.ReactNode;
  hint?: React.ReactNode;
  meta?: React.ReactNode;
  trailing?: React.ReactNode;
  title?: string;
  ariaLabel?: string;
  checked: boolean;
  disabled?: boolean;
  onCheckedChange: (checked: boolean) => void;
}> = ({
  label,
  hint,
  meta,
  trailing,
  title,
  ariaLabel,
  checked,
  disabled,
  onCheckedChange,
}) => (
  <label
    className={classNames(styles.row, hint ? styles.rowTwoLine : undefined)}
    title={title}
  >
    <input
      type="checkbox"
      className={styles.check}
      aria-label={ariaLabel}
      checked={checked}
      disabled={disabled}
      onChange={(event) => onCheckedChange(event.currentTarget.checked)}
    />
    {rowInner({ label, hint, meta, trailing })}
  </label>
);

const KitNumberInput: React.FC<{
  value: number;
  min: number;
  step?: number;
  ariaLabel?: string;
  onCommit: (value: number) => void;
}> = ({ value, min, step, ariaLabel, onCommit }) => {
  const [draft, setDraft] = React.useState<string | null>(null);
  const commit = () => {
    if (draft === null) return;
    const next = Math.round(Number(draft));
    if (draft.trim() !== "" && Number.isFinite(next) && next >= min) {
      onCommit(next);
    }
    setDraft(null);
  };
  return (
    <input
      type="number"
      inputMode="numeric"
      className={styles.sliderInput}
      value={draft ?? String(value)}
      min={min}
      step={step}
      aria-label={ariaLabel}
      onChange={(event) => setDraft(event.currentTarget.value)}
      onBlur={commit}
      onKeyDown={(event) => {
        if (event.key === "Enter") {
          event.preventDefault();
          commit();
        }
      }}
    />
  );
};

export const KitSliderRow: React.FC<{
  label: React.ReactNode;
  value: number;
  min: number;
  max: number;
  step?: number;
  ariaLabel?: string;
  format?: (value: number) => React.ReactNode;
  editable?: boolean;
  unit?: React.ReactNode;
  onValueChange: (value: number) => void;
}> = ({
  label,
  value,
  min,
  max,
  step,
  ariaLabel,
  format,
  editable = false,
  unit,
  onValueChange,
}) => (
  <div className={classNames(styles.row, styles.sliderRow)}>
    <span className={styles.sliderLabel}>{label}</span>
    <Slider
      className={styles.slider}
      value={[Math.min(value, max)]}
      min={min}
      max={max}
      step={step}
      aria-label={ariaLabel}
      onValueChange={([next]) => onValueChange(next)}
    />
    {editable ? (
      <span className={styles.sliderValue}>
        <KitNumberInput
          value={value}
          min={min}
          step={step}
          ariaLabel={ariaLabel ? `${ariaLabel} number` : undefined}
          onCommit={onValueChange}
        />
        {unit ? <span className={styles.sliderUnit}>{unit}</span> : null}
      </span>
    ) : (
      <span className={styles.sliderValue}>
        {format ? format(value) : value.toLocaleString()}
      </span>
    )}
  </div>
);

export const KitKeyValue: React.FC<{
  label: React.ReactNode;
  value: React.ReactNode;
  mono?: boolean;
  title?: string;
  tone?: "danger" | "muted";
  onCopy?: () => void;
  copyLabel?: string;
  copied?: boolean;
}> = ({ label, value, mono, title, tone, onCopy, copyLabel, copied }) => (
  <div className={styles.kv} data-mono={mono ? "true" : undefined}>
    <span className={styles.kvLabel}>{label}</span>
    <span className={styles.kvValue} title={title} data-tone={tone}>
      {value}
    </span>
    {onCopy ? (
      <button
        type="button"
        className={styles.kvCopy}
        aria-label={copyLabel ?? "Copy"}
        data-copied={copied ? "true" : undefined}
        onClick={onCopy}
      >
        {copied ? <Check aria-hidden="true" /> : <Copy aria-hidden="true" />}
      </button>
    ) : null}
  </div>
);

export type KitIconButtonProps = Omit<
  React.ButtonHTMLAttributes<HTMLButtonElement>,
  "type" | "children"
> & { icon: LucideIcon; "aria-label": string };

export const KitIconButton = React.forwardRef<
  HTMLButtonElement,
  KitIconButtonProps
>(({ icon: IconComponent, className, ...props }, ref) => (
  <button
    ref={ref}
    type="button"
    className={classNames(styles.iconButton, className)}
    {...props}
  >
    <IconComponent aria-hidden="true" />
  </button>
));
KitIconButton.displayName = "KitIconButton";

export const KitChevron: React.FC = () => (
  <ChevronRight aria-hidden="true" className={styles.rowChevron} />
);

export const KitDivider: React.FC = () => (
  <div className={styles.divider} role="separator" />
);

export const KitFooter: React.FC<{
  note?: React.ReactNode;
  children?: React.ReactNode;
}> = ({ note, children }) => (
  <div className={styles.footer}>
    {note ? <span className={styles.footerNote}>{note}</span> : null}
    {children}
  </div>
);

export const KitProgress: React.FC<{
  value: number;
  tone?: "warning" | "danger";
  label?: string;
}> = ({ value, tone, label }) => {
  const clamped = Math.max(0, Math.min(100, value));
  return (
    <div
      className={styles.progress}
      data-tone={tone}
      role="progressbar"
      aria-label={label}
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={Math.round(clamped)}
    >
      <div className={styles.progressFill} style={{ width: `${clamped}%` }} />
    </div>
  );
};

export const KitNote: React.FC<{ children: React.ReactNode }> = ({
  children,
}) => <div className={styles.note}>{children}</div>;

export type KitButtonProps = Omit<
  React.ButtonHTMLAttributes<HTMLButtonElement>,
  "type"
> & {
  variant?: "default" | "primary" | "ghost" | "danger";
  icon?: LucideIcon;
};

export const KitButton = React.forwardRef<HTMLButtonElement, KitButtonProps>(
  (
    { variant = "default", icon: IconComponent, className, children, ...props },
    ref,
  ) => (
    <button
      ref={ref}
      type="button"
      className={classNames(styles.button, className)}
      data-variant={variant}
      {...props}
    >
      {IconComponent ? <IconComponent aria-hidden="true" size={13} /> : null}
      {children}
    </button>
  ),
);
KitButton.displayName = "KitButton";

export type KitTab = {
  value: string;
  label: React.ReactNode;
  content: React.ReactNode;
};

export const KitTabs: React.FC<{
  value: string;
  onValueChange: (value: string) => void;
  tabs: readonly KitTab[];
  label?: string;
}> = ({ value, onValueChange, tabs, label }) => (
  <TabsPrimitive.Root value={value} onValueChange={onValueChange}>
    <TabsPrimitive.List className={styles.segmented} aria-label={label}>
      {tabs.map((tab) => (
        <TabsPrimitive.Trigger
          key={tab.value}
          value={tab.value}
          className={styles.segment}
        >
          {tab.label}
        </TabsPrimitive.Trigger>
      ))}
    </TabsPrimitive.List>
    {tabs.map((tab) => (
      <TabsPrimitive.Content
        key={tab.value}
        value={tab.value}
        className={styles.tabPanel}
      >
        {tab.content}
      </TabsPrimitive.Content>
    ))}
  </TabsPrimitive.Root>
);

export type KitDialogProps = Pick<
  DialogPrimitive.DialogContentProps,
  | "onOpenAutoFocus"
  | "onCloseAutoFocus"
  | "onEscapeKeyDown"
  | "onInteractOutside"
  | "onPointerDownOutside"
> & {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  title: React.ReactNode;
  description?: React.ReactNode;
  meta?: React.ReactNode;
  width?: string;
  maxHeight?: string;
  footer?: React.ReactNode;
  footerStart?: React.ReactNode;
  footerNote?: React.ReactNode;
  closeLabel?: string;
  className?: string;
  bodyClassName?: string;
  bodyProps?: React.HTMLAttributes<HTMLDivElement>;
  children?: React.ReactNode;
};

export const KitDialog: React.FC<KitDialogProps> = ({
  open,
  onOpenChange,
  title,
  description,
  meta,
  width = "520px",
  maxHeight,
  footer,
  footerStart,
  footerNote,
  closeLabel = "Close",
  className,
  bodyClassName,
  bodyProps,
  children,
  onOpenAutoFocus,
  onCloseAutoFocus,
  ...contentProps
}) => {
  const returnFocusRef = React.useRef<HTMLElement | null>(null);
  return (
    <DialogPrimitive.Root open={open} onOpenChange={onOpenChange}>
      <DialogPrimitive.Portal container={document.body}>
        <Portal>
          <DialogPrimitive.Overlay className={dialogStyles.overlay} />
        </Portal>
        <Portal>
          <DialogPrimitive.Content
            className={classNames(
              dialogStyles.content,
              "rf-popover-motion",
              styles.dialog,
              className,
            )}
            style={overlayStyle(width, maxHeight)}
            {...(description ? {} : { "aria-describedby": undefined })}
            {...contentProps}
            onOpenAutoFocus={(event) => {
              const active = document.activeElement;
              returnFocusRef.current =
                active instanceof HTMLElement && active !== document.body
                  ? active
                  : null;
              onOpenAutoFocus?.(event);
            }}
            onCloseAutoFocus={(event) => {
              onCloseAutoFocus?.(event);
              if (event.defaultPrevented) return;
              const target = returnFocusRef.current;
              returnFocusRef.current = null;
              if (target?.isConnected) {
                event.preventDefault();
                target.focus({ preventScroll: true });
              }
            }}
          >
            <ModalOverlayProvider value>
              <div className={styles.dialogHeader}>
                <div className={styles.dialogHeading}>
                  <DialogPrimitive.Title className={styles.dialogTitle}>
                    {title}
                  </DialogPrimitive.Title>
                  {description ? (
                    <DialogPrimitive.Description
                      className={styles.dialogDescription}
                    >
                      {description}
                    </DialogPrimitive.Description>
                  ) : null}
                </div>
                {meta !== undefined && meta !== null ? (
                  <span className={styles.dialogMeta}>{meta}</span>
                ) : null}
                <DialogPrimitive.Close asChild>
                  <button
                    type="button"
                    className={styles.dialogClose}
                    aria-label={closeLabel}
                  >
                    <X aria-hidden="true" />
                  </button>
                </DialogPrimitive.Close>
              </div>
              <div
                {...bodyProps}
                className={classNames(styles.dialogBody, bodyClassName)}
              >
                {children}
              </div>
              {footer ?? footerNote ?? footerStart ? (
                <div className={styles.dialogFooter}>
                  {footerStart}
                  {footerNote ? (
                    <span className={styles.footerNote}>{footerNote}</span>
                  ) : (
                    <span className={styles.dialogFooterSpacer} />
                  )}
                  {footer}
                </div>
              ) : null}
            </ModalOverlayProvider>
          </DialogPrimitive.Content>
        </Portal>
      </DialogPrimitive.Portal>
    </DialogPrimitive.Root>
  );
};

export const KitDialogCancel: React.FC<
  Omit<KitButtonProps, "variant"> & { children?: React.ReactNode }
> = ({ children, ...props }) => (
  <DialogPrimitive.Close asChild>
    <KitButton variant="ghost" {...props}>
      {children ?? "Cancel"}
    </KitButton>
  </DialogPrimitive.Close>
);
