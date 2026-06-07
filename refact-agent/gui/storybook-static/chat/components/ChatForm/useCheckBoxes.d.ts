import { Dispatch, SetStateAction } from 'react';
import { FileInfo } from "../../features/Chat/activeFile";
type CheckboxHelp = {
    text: string;
    link?: string;
    linkText?: string;
};
export type Checkbox = {
    name: string;
    label: string;
    checked: boolean;
    value?: string;
    disabled: boolean;
    fileName?: string;
    hide?: boolean;
    info?: CheckboxHelp;
    locked?: boolean;
};
export declare function useAttachedFiles(): {
    files: FileInfo[];
    activeFile: FileInfo;
    addFile: () => void;
    removeFile: (fileToRemove: FileInfo) => void;
    attached: boolean;
    addFilesToInput: (str: string) => string;
    removeAll: () => void;
    setInteracted: Dispatch<SetStateAction<boolean>>;
};
export type Checkboxes = {
    selected_lines: Checkbox;
};
export declare const useCheckboxes: () => {
    checkboxes: {
        selected_lines: Checkbox;
    };
    onToggleCheckbox: (name: string) => void;
    setLineSelectionInteracted: Dispatch<SetStateAction<boolean>>;
    unCheckAll: () => void;
};
export {};
