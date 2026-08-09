/** Text normalization with street-name resolution. */
import { resolveStreetName } from "../geo/resolver";

export function normalize(raw: string): string {
  try {
    return resolveStreetName(raw);
  } catch (e) {
    return raw; // fallback: normalized text without vocabulary correction
  }
}
