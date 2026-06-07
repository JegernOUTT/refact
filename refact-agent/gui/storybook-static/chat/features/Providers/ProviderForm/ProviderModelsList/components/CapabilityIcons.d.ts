import type { ComponentProps, FC, ReactNode } from "react";
import { Text } from "@radix-ui/themes";
import { WidthIcon } from "@radix-ui/react-icons";
import type { ModelCapabilities } from "../utils/groupModelsWithPricing";
export type CapabilityIconsProps = {
    capabilities?: ModelCapabilities;
    size?: "1" | "2";
};
type ModelDetailIconProps = {
    icon: ReactNode;
    children?: ReactNode;
    color?: ComponentProps<typeof Text>["color"];
    tone?: "default" | "accent";
};
export declare const ModelDetailIcon: FC<ModelDetailIconProps>;
type DetailSvgIconProps = ComponentProps<typeof WidthIcon>;
export declare const ContextWindowIcon: FC<DetailSvgIconProps>;
export declare const MaxOutputIcon: FC<DetailSvgIconProps>;
export declare const PricingIcon: FC<DetailSvgIconProps>;
export declare const ToolsIcon: FC<DetailSvgIconProps>;
export declare const VisionIcon: FC<DetailSvgIconProps>;
export declare const ReasoningIcon: FC<DetailSvgIconProps>;
export declare const CapabilityIcons: FC<CapabilityIconsProps>;
export {};
