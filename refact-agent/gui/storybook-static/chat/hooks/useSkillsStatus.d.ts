export declare function useSkillsStatus(chatId: string): {
    skillsEnabled: boolean;
    skillsAvailable: number;
    skillsIncluded: string[];
    activeSkill: string | null;
};
