import { Reducer } from 'redux';
import { Draft } from "@reduxjs/toolkit";
import { Chat } from "./types";
import { ChatMessages } from "../../../services/refact";
export declare const chatReducer: Reducer<Chat> & {
    getInitialState: () => Chat;
};
export declare function maybeAppendToolCallResultFromIdeToMessages(messages: Draft<ChatMessages>, toolCallId: string, accepted: boolean | "indeterminate", replaceOnly?: boolean): void;
