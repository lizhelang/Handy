import { type OSType } from "../lib/utils/keyboard";
import { getSafeOsType } from "../lib/tauriPlatform";

/**
 * Get the current OS type for keyboard handling.
 * This is a simple wrapper - type() is synchronous.
 */
export function useOsType(): OSType {
  return getSafeOsType();
}
