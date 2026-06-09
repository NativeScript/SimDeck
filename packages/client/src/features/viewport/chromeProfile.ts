import type { ChromeProfile } from "../../api/types";

export function isUsableChromeProfile(
  profile: ChromeProfile | null,
): profile is ChromeProfile {
  if (!profile) {
    return false;
  }

  const requiredNumbers = [
    profile.totalWidth,
    profile.totalHeight,
    profile.screenX,
    profile.screenY,
    profile.screenWidth,
    profile.screenHeight,
    profile.cornerRadius,
  ];
  if (!requiredNumbers.every(Number.isFinite)) {
    return false;
  }
  if (
    profile.totalWidth <= 0 ||
    profile.totalHeight <= 0 ||
    profile.screenWidth < 8 ||
    profile.screenHeight < 8 ||
    profile.screenX < -0.5 ||
    profile.screenY < -0.5 ||
    profile.screenX + profile.screenWidth > profile.totalWidth + 0.5 ||
    profile.screenY + profile.screenHeight > profile.totalHeight + 0.5
  ) {
    return false;
  }

  if (
    profile.contentWidth != null ||
    profile.contentHeight != null ||
    profile.contentX != null ||
    profile.contentY != null
  ) {
    const contentNumbers = [
      profile.contentX,
      profile.contentY,
      profile.contentWidth,
      profile.contentHeight,
    ];
    if (
      !contentNumbers.every(
        (value) => typeof value === "number" && Number.isFinite(value),
      )
    ) {
      return false;
    }
    if (
      (profile.contentWidth ?? 0) < 8 ||
      (profile.contentHeight ?? 0) < 8 ||
      (profile.contentX ?? 0) < -0.5 ||
      (profile.contentY ?? 0) < -0.5 ||
      (profile.contentX ?? 0) + (profile.contentWidth ?? 0) >
        profile.totalWidth + 0.5 ||
      (profile.contentY ?? 0) + (profile.contentHeight ?? 0) >
        profile.totalHeight + 0.5
    ) {
      return false;
    }
  }

  return true;
}
