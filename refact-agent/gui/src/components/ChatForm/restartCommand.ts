export function restartAgentCommand(cardId: string): string {
  return `restart_agent(card_id="${cardId}", mode="resume")`;
}
