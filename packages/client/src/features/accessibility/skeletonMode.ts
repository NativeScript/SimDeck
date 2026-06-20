export type AccessibilitySkeletonMode = "auto" | "always" | "off";

export const ACCESSIBILITY_SKELETON_MODES: AccessibilitySkeletonMode[] = [
  "auto",
  "always",
  "off",
];

export function isAccessibilitySkeletonMode(
  value: unknown,
): value is AccessibilitySkeletonMode {
  return value === "auto" || value === "always" || value === "off";
}
