export type CollapsibleStore = {
    get: (key: string) => boolean | undefined;
    set: (key: string, open: boolean) => void;
};
