import type { BuddyWorldState } from "./buddyWorldModel";
import type { Palette } from "./types";
import {
  drawAmbientLayers,
  drawCelestial,
  drawObservatoryStructures,
  drawSkyGradient,
  drawStarField,
  drawWeatherAtmosphere,
  shouldDrawStarField,
} from "./buddyWorldDrawAtmosphere";
import {
  drawBuddyLandingPad,
  drawCampfire,
  drawDistantHills,
  drawForegroundCozyDetails,
  drawGround,
  drawHomePath,
  drawLanterns,
  drawMailbox,
  drawMidgroundGarden,
  drawPond,
  drawVitality,
  drawWinterGroundDust,
  drawWorkshopZones,
} from "./buddyWorldDrawDiorama";
import {
  drawAirship,
  drawAlpineRidge,
  drawGhibliClouds,
  drawGreatTree,
  drawKodama,
  drawKomorebi,
  drawMeadowCritters,
  drawNightSkyDust,
  drawRainPuddles,
  drawSkyIsland,
  drawSootSprites,
  drawStream,
  drawWindStreaks,
} from "./buddyWorldDrawScenery";
import { drawBuddyHomeDoor, drawWorldObjects } from "./buddyWorldDrawObjects";
import {
  safeDimension,
  safeFrame,
  type BuddyWorldTokenPalette,
  type DrawBuddyWorldBaseArgs,
} from "./buddyWorldDrawHelpers";

export interface DrawBuddyWorldArgs {
  ctx: CanvasRenderingContext2D;
  world: BuddyWorldState;
  palette: Palette;
  tokenPalette?: BuddyWorldTokenPalette;
  frame: number;
  width: number;
  height: number;
  compact: boolean;
  reducedMotion: boolean;
}

export function drawBuddyWorld(args: DrawBuddyWorldArgs): void {
  const width = safeDimension(args.width, 720);
  const height = safeDimension(args.height, args.compact ? 190 : 260);
  const drawArgs: DrawBuddyWorldBaseArgs = {
    ...args,
    frame: safeFrame(args.frame),
    width,
    height,
  };

  const { ctx } = args;
  ctx.globalAlpha = 1;
  ctx.globalCompositeOperation = "source-over";
  ctx.clearRect(0, 0, width, height);
  ctx.beginPath();
  ctx.globalAlpha = 1;
  ctx.imageSmoothingEnabled = false;

  drawSkyGradient(drawArgs);
  if (shouldDrawStarField(args.world)) drawStarField(drawArgs);
  drawNightSkyDust(drawArgs);
  drawSkyIsland(drawArgs);
  drawObservatoryStructures(drawArgs);
  drawCelestial(drawArgs);
  drawGhibliClouds(drawArgs);
  drawAirship(drawArgs);
  drawAmbientLayers(drawArgs);
  drawAlpineRidge(drawArgs);
  drawDistantHills(drawArgs);
  drawGreatTree(drawArgs);
  drawMidgroundGarden(drawArgs);
  drawWorkshopZones(drawArgs);
  drawKomorebi(drawArgs);
  drawWeatherAtmosphere(drawArgs);
  drawWorldObjects(drawArgs);
  drawGround(drawArgs);
  drawWinterGroundDust(drawArgs);
  drawStream(drawArgs);
  drawPond(drawArgs);
  drawRainPuddles(drawArgs);
  drawHomePath(drawArgs);
  drawLanterns(drawArgs);
  drawBuddyHomeDoor(drawArgs);
  drawMailbox(drawArgs);
  drawVitality(drawArgs);
  drawKodama(drawArgs);
  drawMeadowCritters(drawArgs);
  drawSootSprites(drawArgs);
  drawBuddyLandingPad(drawArgs);
  drawCampfire(drawArgs);
  drawForegroundCozyDetails(drawArgs);
  drawWindStreaks(drawArgs);
}
