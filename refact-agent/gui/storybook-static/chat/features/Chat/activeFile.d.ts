import { Reducer } from 'redux';
import { ActionCreatorWithPayload } from '@reduxjs/toolkit';
import { RootState } from "../../app/store";
export type FileInfo = {
    name: string;
    line1: number | null;
    line2: number | null;
    can_paste: boolean;
    path: string;
    content?: string;
    usefulness?: number;
    cursor: number | null;
};
export declare const setFileInfo: ActionCreatorWithPayload<FileInfo, string>;
export declare const activeFileReducer: Reducer<FileInfo> & {
    getInitialState: () => FileInfo;
};
export declare const selectActiveFile: (state: RootState) => FileInfo;
export declare const selectCanPaste: (state: RootState) => boolean;
