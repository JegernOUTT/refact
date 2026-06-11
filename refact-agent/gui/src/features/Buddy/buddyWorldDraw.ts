import type { BuddyWorldState } from "./buddyWorldModel";
import type { Palette } from "./types";
import {
  drawAmbientLayers,
  drawCelestial,
  drawHorizonHaze,
  drawObservatoryStructures,
  drawSkyGradient,
  drawStarField,
  drawWeatherAtmosphere,
  shouldDrawStarField,
} from "./buddyWorldDrawAtmosphere";
import {
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
  drawCloudShadows,
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
import { drawBuddyWorldActor } from "./buddyWorldDrawActor";
import {
  finiteOrZero,
  safeDimension,
  safeFrame,
  type BuddyWorldActor,
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
  actor?: BuddyWorldActor | null;
}

interface CameraDrift {
  pan: number;
  rise: number;
}

function cameraDrift(frame: number, reducedMotion: boolean): CameraDrift {
  if (reducedMotion) return { pan: 0, rise: 0 };
  const f = safeFrame(frame);
  return {
    pan: finiteOrZero(Math.sin(f / 620) * 3.1 + Math.sin(f / 167) * 0.5),
    rise: finiteOrZero(Math.sin(f / 840 + 2.1) * 1.05),
  };
}

function withParallaxBand(
  ctx: CanvasRenderingContext2D,
  dx: number,
  dy: number,
  draw: () => void,
): void {
  const safeDx = finiteOrZero(dx);
  const safeDy = finiteOrZero(dy);
  if (Math.abs(safeDx) < 0.01 && Math.abs(safeDy) < 0.01) {
    draw();
    return;
  }
  ctx.save();
  ctx.translate(safeDx, safeDy);
  draw();
  ctx.restore();
}

export function drawBuddyWorld(args: DrawBuddyWorldArgs): void {
  const width = safeDimension(args.width, 720);
  const height = safeDimension(args.height, args.compact ? 190 : 260);
  const drawArgs: DrawBuddyWorldBaseArgs = {
    ctx: args.ctx,
    world: args.world,
    palette: args.palette,
    tokenPalette: args.tokenPalette,
    frame: safeFrame(args.frame),
    width,
    height,
    compact: args.compact,
    reducedMotion: args.reducedMotion,
  };

  const { ctx } = args;
  ctx.globalAlpha = 1;
  ctx.globalCompositeOperation = "source-over";
  ctx.clearRect(0, 0, width, height);
  ctx.beginPath();
  ctx.globalAlpha = 1;
  ctx.imageSmoothingEnabled = false;

  const camera = cameraDrift(drawArgs.frame, args.reducedMotion);

  drawSkyGradient(drawArgs);
  withParallaxBand(ctx, -camera.pan * 0.85, camera.rise * 0.55, () => {
    if (shouldDrawStarField(args.world)) drawStarField(drawArgs);
    drawNightSkyDust(drawArgs);
    drawSkyIsland(drawArgs);
    drawObservatoryStructures(drawArgs);
    drawCelestial(drawArgs);
    drawGhibliClouds(drawArgs);
    drawAirship(drawArgs);
  });
  drawAmbientLayers(drawArgs);
  withParallaxBand(ctx, -camera.pan * 0.5, camera.rise * 0.25, () => {
    drawAlpineRidge(drawArgs);
    drawDistantHills(drawArgs);
    drawHorizonHaze(drawArgs);
  });
  withParallaxBand(ctx, -camera.pan * 0.2, 0, () => {
    drawGreatTree(drawArgs);
    drawMidgroundGarden(drawArgs);
    drawWorkshopZones(drawArgs);
  });
  drawKomorebi(drawArgs);
  drawWeatherAtmosphere(drawArgs);
  drawWorldObjects(drawArgs);
  drawGround(drawArgs);
  drawCloudShadows(drawArgs);
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
  drawCampfire(drawArgs);
  if (args.actor) drawBuddyWorldActor(drawArgs, args.actor);
  withParallaxBand(ctx, camera.pan * 0.32, 0, () => {
    drawForegroundCozyDetails(drawArgs);
    drawWindStreaks(drawArgs);
  });
}
