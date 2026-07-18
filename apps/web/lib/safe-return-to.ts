const CONTROL_OR_BACKSLASH = /[\u0000-\u001f\u007f\\]/;
const BASE = "https://shennong.invalid";

function hasUnsafeForm(value: string): boolean {
  return !value.startsWith("/") || value.startsWith("//") || CONTROL_OR_BACKSLASH.test(value);
}

/**
 * Accept only an origin-relative application location. Repeated decoding closes
 * encoded network-path, backslash, and control-character bypasses while the
 * original encoded value is preserved for legitimate query parameters.
 */
export function safeInternalReturnTo(value: string | null | undefined, fallback = "/projects"): string {
  if (!value || hasUnsafeForm(value)) return fallback;

  let decoded = value;
  try {
    let stable = false;
    for (let pass = 0; pass < 8; pass += 1) {
      const next = decodeURIComponent(decoded);
      if (hasUnsafeForm(next)) return fallback;
      if (next === decoded) {
        stable = true;
        break;
      }
      decoded = next;
    }
    if (!stable) return fallback;
    const parsed = new URL(value, BASE);
    if (parsed.origin !== BASE || !parsed.pathname.startsWith("/")) return fallback;
  } catch {
    return fallback;
  }

  return value;
}
