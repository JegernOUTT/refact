import { Check, Copy } from "lucide-react";
import React, { useCallback, useState } from "react";
import { Flex } from "@radix-ui/themes";
import { Button } from "../../ui";

import { useCopyToClipboard } from "../../../hooks/useCopyToClipboard";
import styles from "./ExecToolCard.module.css";

type CopyTarget = "command" | "output" | "process";

type ProcessControlsProps = {
  command?: string;
  output?: string;
  processId?: string;
};

type CopyButtonProps = {
  target: CopyTarget;
  label: string;
  value?: string;
  copiedTarget: CopyTarget | null;
  onCopy: (target: CopyTarget, value: string) => void;
};

const CopyButton: React.FC<CopyButtonProps> = ({
  target,
  label,
  value,
  copiedTarget,
  onCopy,
}) => {
  if (!value) return null;
  const copied = copiedTarget === target;

  return (
    <Button
      type="button"
      size="sm"
      variant="soft"
      leftIcon={copied ? Check : Copy}
      className={styles.copyButton}
      onClick={(event) => {
        event.stopPropagation();
        onCopy(target, value);
      }}
    >
      {copied ? "Copied" : label}
    </Button>
  );
};

export const ProcessControls: React.FC<ProcessControlsProps> = ({
  command,
  output,
  processId,
}) => {
  const copyToClipboard = useCopyToClipboard();
  const [copiedTarget, setCopiedTarget] = useState<CopyTarget | null>(null);

  const handleCopy = useCallback(
    (target: CopyTarget, value: string) => {
      copyToClipboard(value);
      setCopiedTarget(target);
      window.setTimeout(() => setCopiedTarget(null), 1600);
    },
    [copyToClipboard],
  );

  if (!command && !output && !processId) return null;

  return (
    <Flex gap="2" wrap="wrap" className={styles.controls}>
      <CopyButton
        target="command"
        label="Copy command"
        value={command}
        copiedTarget={copiedTarget}
        onCopy={handleCopy}
      />
      <CopyButton
        target="output"
        label="Copy output"
        value={output}
        copiedTarget={copiedTarget}
        onCopy={handleCopy}
      />
      <CopyButton
        target="process"
        label="Copy process ID"
        value={processId}
        copiedTarget={copiedTarget}
        onCopy={handleCopy}
      />
    </Flex>
  );
};

export default ProcessControls;
