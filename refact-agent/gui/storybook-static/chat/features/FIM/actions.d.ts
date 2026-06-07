import { ActionCreatorWithoutPayload, ActionCreatorWithPayload } from '@reduxjs/toolkit';
import { FimDebugData } from "../../services/refact/fim";
export declare const request: ActionCreatorWithoutPayload<"fim/request">;
export declare const receive: ActionCreatorWithPayload<FimDebugData, string>;
export declare const error: ActionCreatorWithPayload<string, string>;
export declare const ready: ActionCreatorWithoutPayload<"fim/ready">;
export declare const clearError: ActionCreatorWithoutPayload<"fim/clear_error">;
export declare const reset: ActionCreatorWithoutPayload<"fim/reset">;
