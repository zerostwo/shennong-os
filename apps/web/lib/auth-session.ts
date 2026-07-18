export type AuthSession = {
  authenticated: boolean;
  user_id: string;
  role: string;
  scopes: string[];
};

type JsonRecord = Record<string, unknown>;

function record(value: unknown): JsonRecord {
  return value !== null && typeof value === "object" && !Array.isArray(value)
    ? value as JsonRecord
    : {};
}

export function normalizeAuthSession(value: unknown): AuthSession {
  const root = record(value);
  const user = record(root.user);
  const userId = typeof root.user_id === "string"
    ? root.user_id
    : typeof root.id === "string"
      ? root.id
      : typeof user.id === "string"
        ? user.id
        : "";
  const role = typeof root.role === "string"
    ? root.role
    : typeof user.role === "string"
      ? user.role
      : "";
  const scopesValue = Array.isArray(root.scopes) ? root.scopes : user.scopes;
  const scopes = Array.isArray(scopesValue)
    ? scopesValue.filter((scope): scope is string => typeof scope === "string")
    : [];
  if (root.authenticated === false) {
    return { authenticated: false, user_id: "", role: "", scopes: [] };
  }
  return {
    authenticated: root.authenticated === true || Boolean(userId && role),
    user_id: userId,
    role,
    scopes,
  };
}
