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
  drawObservatoryStructures(drawArgs);
  drawCelestial(drawArgs);
  drawAmbientLayers(drawArgs);
  drawDistantHills(drawArgs);
  drawMidgroundGarden(drawArgs);
  drawWorkshopZones(drawArgs);
  drawWeatherAtmosphere(drawArgs);
  drawWorldObjects(drawArgs);
  drawGround(drawArgs);
  drawWinterGroundDust(drawArgs);
  drawPond(drawArgs);
  drawHomePath(drawArgs);
  drawLanterns(drawArgs);
  drawBuddyHomeDoor(drawArgs);
  drawMailbox(drawArgs);
  drawVitality(drawArgs);
  drawBuddyLandingPad(drawArgs);
  drawCampfire(drawArgs);
  drawForegroundCozyDetails(drawArgs);
}
