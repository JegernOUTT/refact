export const EDIT_PLAYER_STEP_MS = 1_100;

export const EDIT_PLAYER_SPEEDS = [1, 2, 4] as const;

export type EditPlayerSpeed = (typeof EDIT_PLAYER_SPEEDS)[number];

export const nextEditPlayerSpeed = (speed: number): EditPlayerSpeed => {
  const index = EDIT_PLAYER_SPEEDS.indexOf(speed as EditPlayerSpeed);
  return EDIT_PLAYER_SPEEDS[(index + 1) % EDIT_PLAYER_SPEEDS.length];
};
