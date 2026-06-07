import { SetupServerApi } from 'msw/node';
import type { Store } from "../app/store";
export * from "../__fixtures__/msw";
export declare const resetApi: (store: Store) => void;
export declare const server: SetupServerApi;
