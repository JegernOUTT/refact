import { JSX } from 'react/jsx-runtime';
type ChatRawJSONProps = {
    thread: {
        title?: string;
        [key: string]: unknown;
    };
    copyHandler: () => void;
};
export declare const ChatRawJSON: ({ thread, copyHandler }: ChatRawJSONProps) => JSX.Element;
export {};
