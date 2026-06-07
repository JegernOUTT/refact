export declare function getTriggerOffset(element: HTMLTextAreaElement, triggers: string[]): number;
export type AnchorRect = {
    x: number;
    y: number;
    height: number;
};
export declare function getAnchorRect(element: HTMLTextAreaElement, triggers: string[]): AnchorRect;
export declare function replaceRange(str: string, range: [number, number], replacement: string): string;
