import type { ChromeProfile } from "../../api/types";
import {
  BEZEL_PADDING,
  DEVICE_SCREEN_WIDTH,
  FIT_MARGIN,
  MAX_ZOOM_MULTIPLIER,
  MIN_ZOOM_MULTIPLIER,
} from "../../shared/constants";
import type { Point, ScreenRect, Size } from "./types";

export function normalizeQuarterTurns(quarterTurns: number): number {
  return ((Math.trunc(quarterTurns) % 4) + 4) % 4;
}

function rotateSize(size: Size, quarterTurns: number): Size {
  return normalizeQuarterTurns(quarterTurns) % 2 === 0
    ? size
    : {
        height: size.width,
        width: size.height,
      };
}

export function shellSize(
  deviceNaturalSize: Size | null,
  chromeProfile: ChromeProfile | null,
  rotationQuarterTurns = 0,
): Size {
  const naturalSize =
    chromeProfile?.totalWidth && chromeProfile?.totalHeight
      ? {
          height: chromeProfile.totalHeight,
          width: chromeProfile.totalWidth,
        }
      : (() => {
          const aspectRatio = deviceNaturalSize
            ? deviceNaturalSize.height / deviceNaturalSize.width
            : 2.16;
          const screenHeight = DEVICE_SCREEN_WIDTH * aspectRatio;
          return {
            height: screenHeight + BEZEL_PADDING * 2,
            width: DEVICE_SCREEN_WIDTH + BEZEL_PADDING * 2,
          };
        })();

  return rotateSize(naturalSize, rotationQuarterTurns);
}

export function buildShellRotationTransform(
  deviceNaturalSize: Size | null,
  chromeProfile: ChromeProfile | null,
  rotationQuarterTurns: number,
): string {
  const naturalShellSize = shellSize(deviceNaturalSize, chromeProfile);

  switch (normalizeQuarterTurns(rotationQuarterTurns)) {
    case 1:
      return `translate(${naturalShellSize.height}px, 0px) rotate(90deg)`;
    case 2:
      return `translate(${naturalShellSize.width}px, ${naturalShellSize.height}px) rotate(180deg)`;
    case 3:
      return `translate(0px, ${naturalShellSize.width}px) rotate(270deg)`;
    default:
      return "none";
  }
}

export function screenAspectRatio(deviceNaturalSize: Size | null): string {
  return deviceNaturalSize
    ? `${deviceNaturalSize.width} / ${deviceNaturalSize.height}`
    : "9 / 19.5";
}

export function mapDisplayedPointToNaturalOrientation(
  point: Point,
  rotationQuarterTurns: number,
): Point {
  switch (normalizeQuarterTurns(rotationQuarterTurns)) {
    case 1:
      return { x: point.y, y: 1 - point.x };
    case 2:
      return { x: 1 - point.x, y: 1 - point.y };
    case 3:
      return { x: 1 - point.y, y: point.x };
    default:
      return point;
  }
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

  const aspectDelta = Math.abs(deviceAspect - profileAspect) / profileAspect;
  if (aspectDelta <= 0.01) {
    return {
      height: chromeProfile.screenHeight,
      width: chromeProfile.screenWidth,
      x: chromeProfile.screenX,
      y: chromeProfile.screenY,
    };
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

export function computeChromeScreenBorderRadius(
  chromeProfile: ChromeProfile | null,
  screenRect: ScreenRect | null,
): string | null {
  if (!chromeProfile || !screenRect) {
    return null;
  }

  const epsilon = 0.5;
  const leftTouches = Math.abs(screenRect.x - chromeProfile.screenX) <= epsilon;
  const topTouches = Math.abs(screenRect.y - chromeProfile.screenY) <= epsilon;
  const rightTouches =
    Math.abs(
      screenRect.x +
        screenRect.width -
        (chromeProfile.screenX + chromeProfile.screenWidth),
    ) <= epsilon;
  const bottomTouches =
    Math.abs(
      screenRect.y +
        screenRect.height -
        (chromeProfile.screenY + chromeProfile.screenHeight),
    ) <= epsilon;
  const radius = Math.max(
    0,
    Math.min(
      chromeProfile.cornerRadius,
      screenRect.width / 2,
      screenRect.height / 2,
    ),
  );
  const topLeft = topTouches && leftTouches ? radius : 0;
  const topRight = topTouches && rightTouches ? radius : 0;
  const bottomRight = bottomTouches && rightTouches ? radius : 0;
  const bottomLeft = bottomTouches && leftTouches ? radius : 0;

  return `${topLeft}px ${topRight}px ${bottomRight}px ${bottomLeft}px`;
}

function computeScale(
  canvasSize: Size | null,
  deviceNaturalSize: Size | null,
  marginX: number,
  marginY: number,
  chromeProfile: ChromeProfile | null,
  reservedBottomInset = 0,
  rotationQuarterTurns = 0,
): number {
  if (!canvasSize) {
    return 1;
  }
  const totalSize = shellSize(
    deviceNaturalSize,
    chromeProfile,
    rotationQuarterTurns,
  );
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
  rotationQuarterTurns = 0,
): number {
  return computeScale(
    canvasSize,
    deviceNaturalSize,
    FIT_MARGIN,
    FIT_MARGIN,
    chromeProfile,
    reservedBottomInset,
    rotationQuarterTurns,
  );
}

export function computeCenterScale(
  canvasSize: Size | null,
  deviceNaturalSize: Size | null,
  chromeProfile: ChromeProfile | null,
  reservedBottomInset = 0,
  rotationQuarterTurns = 0,
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
      rotationQuarterTurns,
    ),
  );
}

export function clampZoom(scale: number, fitScale: number): number {
  const minZoom = Math.min(1, fitScale) * MIN_ZOOM_MULTIPLIER;
  const maxZoom = Math.max(1, fitScale) * MAX_ZOOM_MULTIPLIER;
  return Math.min(Math.max(scale, minZoom), maxZoom);
}

export function clampPan(
  nextPan: Point,
  scale: number,
  canvasSize: Size | null,
  deviceNaturalSize: Size | null,
  chromeProfile: ChromeProfile | null,
  rotationQuarterTurns = 0,
): Point {
  if (!canvasSize) {
    return nextPan;
  }

  const deviceSize = shellSize(
    deviceNaturalSize,
    chromeProfile,
    rotationQuarterTurns,
  );
  const scaledWidth = deviceSize.width * scale;
  const scaledHeight = deviceSize.height * scale;
  const limitX = Math.max((scaledWidth - canvasSize.width) / 2 + 32, 0);
  const limitY = Math.max((scaledHeight - canvasSize.height) / 2 + 32, 0);

  return {
    x: Math.min(Math.max(nextPan.x, -limitX), limitX),
    y: Math.min(Math.max(nextPan.y, -limitY), limitY),
  };
}
