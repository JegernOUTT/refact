import { InternalLinkContextValue } from './InternalLinkContext';
export declare const useInternalLinkHandler: () => InternalLinkContextValue | null;
export declare const parseRefactLink: (url: string) => {
    type: string;
    id: string;
} | null;
