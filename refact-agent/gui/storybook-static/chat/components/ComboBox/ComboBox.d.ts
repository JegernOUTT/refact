import React from "react";
import type { TextAreaProps } from "../TextArea/TextArea";
import { type DebouncedState } from "usehooks-ts";
import { CommandCompletionResponse } from "../../services/refact";
export type ComboBoxProps = {
    commands: CommandCompletionResponse;
    onChange: (value: string) => void;
    value: string;
    onSubmit: React.KeyboardEventHandler<HTMLTextAreaElement>;
    onSubmitAcceptedValue?: (value: string) => void;
    placeholder?: string;
    render: (props: TextAreaProps) => React.ReactElement;
    requestCommandsCompletion: DebouncedState<(query: string, cursor: number) => void>;
    onHelpClick: () => void;
};
export declare const ComboBox: React.FC<ComboBoxProps>;
