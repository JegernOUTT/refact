import { Reducer } from 'redux';
import { ActionCreatorWithPayload } from '@reduxjs/toolkit';
import { RootState } from "../../app/store";
export type Snippet = {
    language: string;
    code: string;
    path: string;
    basename: string;
    start_line?: number;
    end_line?: number;
};
export declare const setSelectedSnippet: ActionCreatorWithPayload<Snippet, string>;
export declare const selectedSnippetReducer: Reducer<Snippet> & {
    getInitialState: () => Snippet;
};
export declare const selectSelectedSnippet: (state: RootState) => Snippet;
