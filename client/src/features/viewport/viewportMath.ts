import type { ChromeProfile } from "../../api/types";
import {
  BEZEL_PADDING,
  DEVICE_SCREEN_WIDTH,
  FIT_MARGIN,
  MAX_ZOOM_MULTIPLIER,
} from "../../shared/constants";
import type { Point, ScreenRect, Size } from "./types";

export function shellSize(
  deviceNaturalSize: Size | null,
  chromeProfile: ChromeProfile | null,
): Size {
  if (chromeProfile?.totalWidth && chromeProfile?.totalHeight) {
    return {
      width: chromeProfile.totalWidth,
      height: chromeProfile.totalHeight,
    };
  }

  const aspectRatio = deviceNaturalSize
    ? deviceNaturalSize.height / deviceNaturalSize.width
    : 2.16;
  const screenHeight = DEVICE_SCREEN_WIDTH * aspectRatio;
  return {
    width: DEVICE_SCREEN_WIDTH + BEZEL_PADDING * 2,
    height: screenHeight + BEZEL_PADDING * 2,
  };
}

export function computeChromeScreenRect(
  chromeProfile: ChromeProfile | null,
  deviceNaturalSize: Size | null,
): ScreenRect | null {
  if (!chromeProfile) {
    return null;
  }

  const profileAspect = chromeProfile.screenWidth / chromeProfile.screenHeight;
  const deviceAspect = deviceNaturalSize
    ? deviceNaturalSize.width / deviceNaturalSize.height
    : profileAspect;
  if (!deviceAspect || !Number.isFinite(deviceAspect)) {
    return null;
  }

  let width = chromeProfile.screenWidth;
  let height = width / deviceAspect;
  let x = chromeProfile.screenX;
  let y = chromeProfile.screenY;

  if (height > chromeProfile.screenHeight) {
    height = chromeProfile.screenHeight;
    width = height * deviceAspect;
    x += (chromeProfile.screenWidth - width) / 2;
  } else {
    y += (chromeProfile.screenHeight - height) / 2;
  }

  return { x, y, width, height };
}

function computeScale(
  canvasSize: Size | null,
  deviceNaturalSize: Size | null,
  marginX: number,
  marginY: number,
  chromeProfile: ChromeProfile | null,
  reservedBottomInset = 0,
): number {
  if (!canvasSize) {
    return 1;
  }
  const totalSize = shellSize(deviceNaturalSize, chromeProfile);
  const availableWidth = Math.max(canvasSize.width - marginX * 2, 0);
  const availableHeight = Math.max(
    canvasSize.height - marginY * 2 - reservedBottomInset,
    0,
  );
  return Math.min(
    availableWidth / totalSize.width,
    availableHeight / totalSize.height,
  );
}

export function computeFitScale(
  canvasSize: Size | null,
  deviceNaturalSize: Size | null,
  chromeProfile: ChromeProfile | null,
  reservedBottomInset = 0,
): number {
  return computeScale(
    canvasSize,
    deviceNaturalSize,
    FIT_MARGIN,
    FIT_MARGIN,
    chromeProfile,
    reservedBottomInset,
  );
}

export function computeCenterScale(
  canvasSize: Size | null,
  deviceNaturalSize: Size | null,
  chromeProfile: ChromeProfile | null,
  reservedBottomInset = 0,
): number {
  if (!canvasSize) {
    return 1;
  }

  const horizontalMargin = Math.max(
    40,
    Math.min(96, Math.round(canvasSize.width * 0.12)),
  );
  const verticalMargin = Math.max(
    28,
    Math.min(72, Math.round(canvasSize.height * 0.08)),
  );

  return Math.min(
    1,
    computeScale(
      canvasSize,
      deviceNaturalSize,
      horizontalMargin,
      verticalMargin,
      chromeProfile,
      reservedBottomInset,
    ),
  );
}

export function clampZoom(scale: number, fitScale: number): number {
  const minZoom = Math.min(1, fitScale);
  const maxZoom = Math.max(1, fitScale) * MAX_ZOOM_MULTIPLIER;
  return Math.min(Math.max(scale, minZoom), maxZoom);
}

export function clampPan(
  nextPan: Point,
  scale: number,
  canvasSize: Size | null,
  deviceNaturalSize: Size | null,
  chromeProfile: ChromeProfile | null,
): Point {
  if (!canvasSize) {
    return nextPan;
  }

  const deviceSize = shellSize(deviceNaturalSize, chromeProfile);
  const scaledWidth = deviceSize.width * scale;
  const scaledHeight = deviceSize.height * scale;
  const limitX = Math.max((scaledWidth - canvasSize.width) / 2 + 32, 0);
  const limitY = Math.max((scaledHeight - canvasSize.height) / 2 + 32, 0);

  return {
    x: Math.min(Math.max(nextPan.x, -limitX), limitX),
    y: Math.min(Math.max(nextPan.y, -limitY), limitY),
  };
}
