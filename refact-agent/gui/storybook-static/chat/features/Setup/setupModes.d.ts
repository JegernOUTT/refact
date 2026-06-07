export interface SetupMode {
    label: string;
    mode: string;
}
export declare const SETUP_MODES: SetupMode[];
export declare const SETUP_MODE_IDS: Set<string>;
export declare function isValidSetupMode(mode: string): boolean;
