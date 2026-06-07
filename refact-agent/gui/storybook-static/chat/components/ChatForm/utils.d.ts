import { ChatContextFile } from "../../services/refact";
import { FileInfo } from "../../features/Chat/activeFile";
import type { Checkboxes } from "./useCheckBoxes";
export declare function addCheckboxValuesToInput(input: string, checkboxes: Checkboxes): string;
export declare function activeFileToContextFile(fileInfo: FileInfo): ChatContextFile;
