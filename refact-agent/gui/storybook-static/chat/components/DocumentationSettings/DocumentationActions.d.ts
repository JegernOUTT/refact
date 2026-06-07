import { DocumentationSource } from "./DocumentationSettings";
type DocumentationActionsProps = {
    source: DocumentationSource;
    deleteDocumentation: (url: string) => void;
    editDocumentation: (url: string, maxDepth: number, maxPages: number) => void;
    refetchDocumentation: (url: string) => void;
};
export declare const DocumentationActions: React.FC<DocumentationActionsProps>;
export {};
