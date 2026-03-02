import { useGetSkillsStatusQuery } from "../services/refact/skillsStatus";

export function useSkillsStatus(chatId: string) {
  const { data } = useGetSkillsStatusQuery(chatId, {
    pollingInterval: 5000,
    skip: !chatId,
  });
  return {
    skillsEnabled: data?.skills_enabled ?? false,
    skillsAvailable: data?.skills_available ?? 0,
    skillsIncluded: data?.skills_included ?? [],
    activeSkill: data?.active_skill ?? null,
  };
}
