import { ActionCreatorWithPayload } from '@reduxjs/toolkit';
import { ChatMessage } from "../../services/refact";
export type InputActionPayload = {
    value?: string;
    messages?: ChatMessage[];
    send_immediately: boolean;
};
export declare const addInputValue: ActionCreatorWithPayload<InputActionPayload, string>;
export declare const setInputValue: ActionCreatorWithPayload<InputActionPayload, string>;
