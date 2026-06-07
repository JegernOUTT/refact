import React from "react";
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
    <div className={styles.chatFormShell}>
      <ScrollArea scrollbars="vertical">
        <form
          onSubmit={(event) => {
            event.preventDefault();
            onSubmit(event);
          }}
          {...props}
        />
      </ScrollArea>
    </div>
  );
};
