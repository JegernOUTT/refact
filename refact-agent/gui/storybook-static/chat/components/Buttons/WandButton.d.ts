import { JSX } from 'react/jsx-runtime';
type WandButtonProps = {
    currentText: string;
    disabled?: boolean;
    onUpdateText?: (text: string) => void;
};
export declare const WandButton: {
    ({ currentText, disabled, onUpdateText, }: WandButtonProps): JSX.Element;
    displayName: string;
};
export {};
