import type { BuddyPage, BuddyPetState, BuddyPulse, BuddyQuest, BuddyRuntimeEvent, BuddySemanticState } from "./types";
export type BuddyWorldPhase = "morning" | "day" | "evening" | "night";
export type BuddyWorldWeather = "clear" | "aurora" | "busy" | "wind" | "rain" | "storm" | "dream";
export type BuddyWorldMood = "serene" | "curious" | "busy" | "sleepy" | "hungry" | "bored" | "affectionate" | "unstable";
export type BuddyWorldLayer = "sun_motes" | "moths" | "fireflies" | "stars" | "aurora" | "dream_mist" | "workshop_runes" | "provider_storm" | "provider_flicker" | "memory_orbs" | "cozy_home_glow" | "toy_glow" | "empty_food_nook";
export interface BuddyWorldAtmosphere {
    phase: BuddyWorldPhase;
    mood: BuddyWorldMood;
    primaryWeather: BuddyWorldWeather;
    layers: BuddyWorldLayer[];
    intensity: number;
    paletteHint: "dawn" | "day" | "dusk" | "night" | "dream" | "storm";
    serious: boolean;
}
export type BuddyWorldTone = "good" | "neutral" | "warning" | "danger";
export type BuddyWorldSprite = "task_grove" | "memory_fireflies" | "observatory" | "satellite" | "git_vane" | "market_comet" | "seed";
export type BuddyWorldObjectState = "calm" | "active" | "attention" | "critical" | "celebrating";
export type BuddyWorldObjectAnimation = "none" | "breathe" | "sparkle" | "flicker" | "orbit" | "wobble" | "storm" | "stream" | "dim";
export interface BuddyWorldObject {
    id: string;
    sprite: BuddyWorldSprite;
    label: string;
    value: string;
    description: string;
    page: BuddyPage;
    tone: BuddyWorldTone;
    x: number;
    y: number;
    size: number;
    state: BuddyWorldObjectState;
    intensity: number;
    animation: BuddyWorldObjectAnimation;
    interactionX: number;
    interactionY: number;
    depthScale: number;
    magicalLabel?: string;
}
export interface BuddyWorldState {
    phase: BuddyWorldPhase;
    phaseLabel: string;
    phaseMessage: string;
    celestialEmoji: string;
    celestialLabel: string;
    celestialAction: string;
    celestialX: number;
    celestialY: number;
    weather: BuddyWorldWeather;
    weatherLabel: string;
    weatherDescription: string;
    weatherX: number;
    weatherY: number;
    vitality: "lush" | "growing" | "tangled";
    vitalityLabel: string;
    objects: BuddyWorldObject[];
    atmosphere: BuddyWorldAtmosphere;
    headline: string;
}
export declare function buildBuddyWorldState(args: {
    now: Date;
    pulse: BuddyPulse | null | undefined;
    pet: BuddyPetState | undefined;
    nowPlaying: BuddyRuntimeEvent | null;
    activeQuest: BuddyQuest | null;
    semanticState?: BuddySemanticState;
}): BuddyWorldState;
