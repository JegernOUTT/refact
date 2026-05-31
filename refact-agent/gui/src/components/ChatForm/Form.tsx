import React from "react";
import { Box } from "@radix-ui/themes";
import styles from "./ChatForm.module.css";
import { ScrollArea } from "../ScrollArea";

export const Form: React.FC<
  React.PropsWithChildren<{
    className?: string;
    onClick?: React.MouseEventHandler<HTMLFormElement>;
    onSubmit: React.FormEventHandler<HTMLFormElement>;
    onPointerDownCapture?: React.PointerEventHandler<HTMLFormElement>;
    disabled?: boolean;
  }>
> = ({ onSubmit, ...props }) => {
  return (
    <Box mt="1" className={styles.chatForm}>
      <ScrollArea scrollbars="vertical">
        <form
          onSubmit={(event) => {
            event.preventDefault();
            onSubmit(event);
          }}
          {...props}
        />
      </ScrollArea>
    </Box>
  );
};
