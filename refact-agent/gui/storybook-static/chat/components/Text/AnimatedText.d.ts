import { JSX } from 'react/jsx-runtime';
import { TextProps } from "@radix-ui/themes";
export type AnimatedTextProps = TextProps & {
    animating?: boolean;
};
export declare const AnimatedText: ({ animating, ...props }: AnimatedTextProps) => JSX.Element;
