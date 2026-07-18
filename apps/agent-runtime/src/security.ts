import { createHash, timingSafeEqual } from "node:crypto";
import type { JsonValue } from "./types.js";

const SENSITIVE_KEY = /(?:^|_)(?:api[_-]?key|authorization|cookie|credential|password|secret|storage[_-]?uri|token)(?:$|_)/i;
const BEARER = /\bBearer\s+[A-Za-z0-9._~+\/-]+=*/gi;
const MAX_DEPTH = 8;
const MAX_ARRAY_ITEMS = 100;
const MAX_OBJECT_KEYS = 100;
const MAX_STRING_LENGTH = 16_384;
const MAX_TOOL_ARGUMENT_DEPTH = 32;
const MAX_TOOL_ARGUMENT_BYTES = 1024 * 1024;

export function sha256(value: string | Uint8Array): string {
  return `sha256:${createHash("sha256").update(value).digest("hex")}`;
}

function normalized(value: unknown, depth = 0, key = ""): JsonValue {
  if (SENSITIVE_KEY.test(key)) return "[redacted]";
  if (depth > MAX_DEPTH) return "[truncated:depth]";
  if (value === null || typeof value === "boolean") return value;
  if (typeof value === "number") return Number.isFinite(value) ? value : String(value);
  if (typeof value === "string") {
    const clean = value.replaceAll("\0", "").replace(BEARER, "Bearer [redacted]");
    return clean.length <= MAX_STRING_LENGTH
      ? clean
      : `${clean.slice(0, MAX_STRING_LENGTH)}[truncated]`;
  }
  if (Array.isArray(value)) {
    const items = value.slice(0, MAX_ARRAY_ITEMS).map((item) => normalized(item, depth + 1));
    if (value.length > MAX_ARRAY_ITEMS) items.push(`[truncated:${value.length - MAX_ARRAY_ITEMS}-items]`);
    return items;
  }
  if (typeof value === "object") {
    const result: Record<string, JsonValue> = {};
    const entries = Object.entries(value as Record<string, unknown>)
      .sort(([left], [right]) => left.localeCompare(right))
      .slice(0, MAX_OBJECT_KEYS);
    for (const [childKey, childValue] of entries) {
      result[childKey] = normalized(childValue, depth + 1, childKey);
    }
    const total = Object.keys(value as object).length;
    if (total > MAX_OBJECT_KEYS) result._truncated = `${total - MAX_OBJECT_KEYS} keys`;
    return result;
  }
  return String(value ?? "");
}

export function sanitizeUntrusted(value: unknown): JsonValue {
  return normalized(value);
}

export function canonicalJson(value: unknown): string {
  return JSON.stringify(normalized(value));
}

export function promptSafeJson(value: unknown): string {
  return canonicalJson(value)
    .replaceAll("<", "\\u003c")
    .replaceAll(">", "\\u003e")
    .replaceAll("&", "\\u0026")
    .replaceAll("\u2028", "\\u2028")
    .replaceAll("\u2029", "\\u2029");
}

function exactJson(value: unknown, depth = 0): string {
  if (depth > MAX_TOOL_ARGUMENT_DEPTH) throw new Error("tool_arguments_too_deep");
  if (value === null || typeof value === "boolean" || typeof value === "string") return JSON.stringify(value);
  if (typeof value === "number") {
    if (!Number.isFinite(value)) throw new Error("tool_arguments_invalid_number");
    return JSON.stringify(value);
  }
  if (Array.isArray(value)) return `[${value.map((item) => exactJson(item, depth + 1)).join(",")}]`;
  if (typeof value === "object") {
    const prototype = Object.getPrototypeOf(value);
    if (prototype !== Object.prototype && prototype !== null) throw new Error("tool_arguments_invalid_object");
    return `{${Object.entries(value as Record<string, unknown>)
      .sort(([left], [right]) => left.localeCompare(right))
      .map(([key, child]) => `${JSON.stringify(key)}:${exactJson(child, depth + 1)}`)
      .join(",")}}`;
  }
  throw new Error("tool_arguments_not_json");
}

export function canonicalToolArguments(value: unknown): string {
  const encoded = exactJson(value);
  if (Buffer.byteLength(encoded) > MAX_TOOL_ARGUMENT_BYTES) throw new Error("tool_arguments_too_large");
  return encoded;
}

export function argumentsDigest(toolName: string, value: unknown): string {
  return createHash("sha256").update(`${toolName}\0${canonicalToolArguments(value)}`).digest("hex");
}

export function redactText(value: string, secrets: Array<string | undefined> = []): string {
  let clean = value.replaceAll("\0", "").replace(BEARER, "Bearer [redacted]");
  for (const secret of secrets) {
    if (secret && secret.length >= 8) clean = clean.replaceAll(secret, "[redacted]");
  }
  return clean.length <= 4096 ? clean : `${clean.slice(0, 4096)}[truncated]`;
}

export function timingSafeSecret(provided: string | undefined, expected: string): boolean {
  if (!provided?.startsWith("Bearer ") || expected.length < 32) return false;
  const left = createHash("sha256").update(provided.slice(7)).digest();
  const right = createHash("sha256").update(expected).digest();
  return timingSafeEqual(left, right);
}
