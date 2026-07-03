import React, { useCallback, useEffect, useRef, useState } from "react";
import { Check, Copy } from "lucide-react";

import { IconButton, Tooltip } from "../../components/ui";

export type CopyButtonProps = {
  text: string;
  label?: string;
};

export const CopyButton: React.FC<CopyButtonProps> = ({
  text,
  label = "Copy",
}) => {
  const [copied, setCopied] = useState(false);
  const timeoutRef = useRef<number | null>(null);

  useEffect(() => {
    return () => {
      if (timeoutRef.current !== null) {
        window.clearTimeout(timeoutRef.current);
      }
    };
  }, []);

  const handleCopy = useCallback(() => {
    void navigator.clipboard
      .writeText(text)
      .then(() => {
        setCopied(true);
        if (timeoutRef.current !== null) {
          window.clearTimeout(timeoutRef.current);
        }
        timeoutRef.current = window.setTimeout(() => {
          setCopied(false);
        }, 1500);
      })
      .catch(() => undefined);
  }, [text]);

  return (
    <Tooltip>
      <Tooltip.Trigger asChild>
        <IconButton
          aria-label={label}
          icon={copied ? Check : Copy}
          onClick={handleCopy}
          size="sm"
          variant="plain"
        />
      </Tooltip.Trigger>
      <Tooltip.Content side="top">{copied ? "Copied" : label}</Tooltip.Content>
    </Tooltip>
  );
};
