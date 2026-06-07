import { DebouncedState } from 'usehooks-ts';
import { Checkboxes } from "./useCheckBoxes";
import { type CommandCompletionResponse } from "../../services/refact/commands";
import { ChatContextFile } from "../../services/refact/types";
export declare function useCommandCompletionAndPreviewFiles(checkboxes: Checkboxes, addFilesToInput: (str: string) => string): {
    commands: CommandCompletionResponse;
    requestCompletion: DebouncedState<(query: string, cursor: number) => void>;
    previewFiles: (string | ChatContextFile)[];
};
