import { Reducer } from 'redux';
import { ActionCreatorWithPayload, ActionCreatorWithOptionalPayload } from '@reduxjs/toolkit';
import { RootState } from "../../app/store";
export type CurrentProjectInfo = {
    name: string;
    workspaceRoots?: string[];
};
export declare const setCurrentProjectInfo: ActionCreatorWithPayload<CurrentProjectInfo, string>;
export declare const resetSidebarReadiness: ActionCreatorWithOptionalPayload<undefined, string>;
export declare const currentProjectInfoReducer: Reducer<CurrentProjectInfo> & {
    getInitialState: () => CurrentProjectInfo;
};
export declare const selectThreadProjectOrCurrentProject: (state: RootState) => string;
export declare const selectHasActiveProject: (state: RootState) => boolean;
