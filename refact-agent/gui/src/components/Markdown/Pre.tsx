import React from "react";
import { IconButton, Tooltip } from "@radix-ui/themes";
import { CopyIcon } from "@radix-ui/react-icons";
import styles from "./Markdown.module.css";

const PreTagWithButtons: React.FC<
  React.PropsWithChildren<{
    onCopyClick: () => void;
    className?: string;
  }>
> = ({ children, onCopyClick, className, ...props }) => {
  return (
    <pre className={className} {...props}>
      <Tooltip content="Copy">
        <IconButton
          size="1"
          variant="soft"
          className={styles.copy_button}
          onClick={onCopyClick}
          aria-label="Copy code"
        >
          <CopyIcon width={12} height={12} />
        </IconButton>
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
