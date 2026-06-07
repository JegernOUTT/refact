import { Provider } from 'react';
import type { CollapsibleStore } from "./CollapsibleStore";
export declare const CollapsibleStoreProvider: Provider<CollapsibleStore | null>;
export declare function useCollapsibleStore(): CollapsibleStore | null;
export declare function useStoredOpen(storeKey: string | undefined, defaultOpen?: boolean): [boolean, () => void, (open: boolean) => void];
