import { type ImageFile, type TextFile } from "../features/Chat";
export declare function useAttachedImages(): {
    images: ImageFile[];
    textFiles: TextFile[];
    setError: (error: string) => void;
    setWarning: (warning: string) => void;
    insertImage: (file: ImageFile) => void;
    removeImage: (index: number) => void;
    processAndInsertImages: (files: File[]) => void;
    removeTextFile: (index: number) => void;
    processAndInsertTextFiles: (files: File[]) => void;
    resetAllTextFiles: () => void;
};
