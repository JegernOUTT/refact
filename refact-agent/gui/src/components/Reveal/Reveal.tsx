import React, { useCallback, useEffect, useState } from "react";
import { Box, Button, Flex } from "@radix-ui/themes";
import { useCollapsibleStore } from "../ChatContent/useStoredOpen";
import styles from "./reveal.module.css";
import classNames from "classnames";

export type RevealProps = {
  children: React.ReactNode;
  defaultOpen: boolean;
  isRevealingCode?: boolean;
  onClose?: () => void;
  storeKey?: string;
};

const RevealButton: React.FC<{
  onClick: () => void;
  isInline: boolean;
  children: React.ReactNode;
}> = ({ onClick, isInline, children }) => (
  <Button
    variant="ghost"
    onClick={onClick}
    asChild
    className={classNames(styles.reveal_button, {
      [styles.reveal_button_inline]: isInline,
    })}
  >
    {children}
  </Button>
);

const RevealText: React.FC<{
  isRevealingCode: boolean;
  text: string;
}> = ({ isRevealingCode, text }) => (
  <Flex position="absolute" bottom="2" width="100%" justify="center">
    {isRevealingCode ? text : <Box className={styles.reveal_text}>{text}</Box>}
  </Flex>
);

export const Reveal: React.FC<RevealProps> = ({
  children,
  defaultOpen,
  isRevealingCode = false,
  onClose,
  storeKey,
}) => {
  const store = useCollapsibleStore();
  const [open, setOpen] = useState(() => {
    if (storeKey && store) {
      const stored = store.get(storeKey);
      if (stored !== undefined) return stored;
    }
    return defaultOpen;
  });

  useEffect(() => {
    if (storeKey && store) store.set(storeKey, open);
  }, [storeKey, store, open]);

  const handleClick = useCallback(() => {
    if (defaultOpen) return;
    setOpen((v) => !v);
  }, [defaultOpen]);

  const handleClose = useCallback(() => {
    if (defaultOpen) return;
    setOpen(false);
    onClose && onClose();
  }, [defaultOpen, onClose]);

  if (open) {
    return (
      <Box width="100%" position="relative" pb="5">
        {children}
        <RevealButton onClick={handleClose} isInline={!isRevealingCode}>
          {!defaultOpen && (
            <Box
              className={classNames(
                styles.reveal_hidden,
                styles.reveal_hidden_exposed,
              )}
            >
              <RevealText
                isRevealingCode={isRevealingCode}
                text="Hide details"
              />
            </Box>
          )}
        </RevealButton>
      </Box>
    );
  }

  return (
    <RevealButton onClick={handleClick} isInline={!isRevealingCode}>
      <Flex direction="column" position="relative" align="start">
        <Box
          className={classNames({
            [styles.reveal_hidden]: !open,
          })}
          width="100%"
        >
          {children}
        </Box>
        {!defaultOpen && (
          <Box
            className={classNames({
              [styles.reveal_button_box]: open,
            })}
          >
            <RevealText
              isRevealingCode={isRevealingCode}
              text="Click for more"
            />
          </Box>
        )}
      </Flex>
    </RevealButton>
  );
};
