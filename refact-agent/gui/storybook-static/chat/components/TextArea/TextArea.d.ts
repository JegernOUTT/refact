import React from "react";
import { TextArea as RadixTextArea } from "@radix-ui/themes";
export type TextAreaProps = React.ComponentProps<typeof RadixTextArea> & React.JSX.IntrinsicElements["textarea"] & {
    onTextAreaHeightChange?: (scrollHeight: number) => void;
    onChange: (event: React.ChangeEvent<HTMLTextAreaElement>) => void;
};
export declare const TextArea: React.ForwardRefExoticComponent<Omit<TextAreaProps, "ref"> & React.RefAttributes<HTMLTextAreaElement>>;
