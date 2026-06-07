import React from "react";
interface SkillActivatedCardProps {
    name: string;
    body: string;
    allowedTools: string[];
    modelOverride: string | null;
}
export declare const SkillActivatedCard: React.FC<SkillActivatedCardProps>;
export {};
