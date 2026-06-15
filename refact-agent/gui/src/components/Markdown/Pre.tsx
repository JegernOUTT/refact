import React from "react";
import { Copy } from "lucide-react";
import { IconButton, Tooltip } from "../ui";
import styles from "./Markdown.module.css";

const PreTagWithButtons: React.FC<
  React.PropsWithChildren<{
    onCopyClick: () => void;
    className?: string;
  }>
> = ({ children, onCopyClick, className, ...props }) => {
  return (
    <pre className={className} {...props}>
      <Tooltip>
        <Tooltip.Trigger asChild>
          <IconButton
            size="sm"
            variant="soft"
            className={styles.copy_button}
            onClick={onCopyClick}
            aria-label="Copy code"
            icon={Copy}
          />
        </Tooltip.Trigger>
        <Tooltip.Content>Copy</Tooltip.Content>
      </Tooltip>
      {children}
    </pre>
  );
};

export type PreTagProps = {
  onCopyClick?: () => void;
  className?: string;
};

export const PreTag: React.FC<React.PropsWithChildren<PreTagProps>> = ({
  onCopyClick,
  className,
  children,
  ...rest
}) => {
  if (onCopyClick) {
    return (
      <PreTagWithButtons
        onCopyClick={onCopyClick}
        className={className}
        {...rest}
      >
        {children}
      </PreTagWithButtons>
    );
  }
  return (
    <pre className={className} {...rest}>
      {children}
    </pre>
  );
};
