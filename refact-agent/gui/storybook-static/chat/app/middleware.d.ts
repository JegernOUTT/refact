import { UnknownAction } from 'redux';
import { ThunkDispatch } from 'redux-thunk';
import { ListenerMiddlewareInstance } from '@reduxjs/toolkit';
export declare const listenerMiddleware: ListenerMiddlewareInstance<unknown, ThunkDispatch<unknown, unknown, UnknownAction>, unknown>;
export declare function resetHandoffSets(): void;
export declare function handoffSetsSize(): {
    processed: number;
    inFlight: number;
};
