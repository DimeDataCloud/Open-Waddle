import data from "./waddle.sprites.json";

export type Frame = readonly string[];
export type FrameName = keyof typeof data.frames;

export const SPRITE_W = data.width;
export const SPRITE_H = data.height;
export const FRAMES: Record<FrameName, Frame> = data.frames;
