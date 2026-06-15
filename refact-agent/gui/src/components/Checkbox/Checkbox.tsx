import React from "react";
import styles from "./Checkbox.module.css";

export type CheckboxCheckedState = boolean | "indeterminate";

export type CheckboxProps = Omit<
  React.InputHTMLAttributes<HTMLInputElement>,
  "children" | "checked" | "onChange" | "size" | "type"
> & {
  checked?: CheckboxCheckedState;
  children?: React.ReactNode;
  onCheckedChange?: (checked: CheckboxCheckedState) => void;
  size?: string;
};

export const Checkbox: React.FC<CheckboxProps> = ({
  name,
  checked,
  disabled,
  onCheckedChange,
  children,
  title,
  size: _size,
  ...props
}) => {
  const ref = React.useRef<HTMLInputElement>(null);
  const isIndeterminate = checked === "indeterminate";

  React.useEffect(() => {
    if (ref.current) {
      ref.current.indeterminate = isIndeterminate;
    }
  }, [isIndeterminate]);

  return (
    <label className={styles.label} title={title}>
      <input
        {...props}
        ref={ref}
        className={styles.control}
        name={name}
        checked={checked === true}
        disabled={disabled}
        type="checkbox"
        onChange={(event) => onCheckedChange?.(event.currentTarget.checked)}
      />
      {children ? <span className={styles.content}>{children}</span> : null}
    </label>
  );
};
