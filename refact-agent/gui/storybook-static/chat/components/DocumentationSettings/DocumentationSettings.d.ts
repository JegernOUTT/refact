export interface DocumentationSource {
    url: string;
    maxDepth: number;
    maxPages: number;
    pages: number;
}
export type DocumentationSettingsProps = {
    sources: DocumentationSource[];
    addDocumentation: (url: string, maxDepth: number, maxPages: number) => void;
    deleteDocumentation: (url: string) => void;
    refetchDocumentation: (url: string) => void;
    editDocumentation: (url: string, maxDepth: number, maxPages: number) => void;
};
export declare const DocumentationSettings: React.FC<DocumentationSettingsProps>;
