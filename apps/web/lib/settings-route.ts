export type SettingsSection = "account" | "general" | "models" | "personalization" | "keyboard" | "memory";

const routeNames: Record<SettingsSection, string> = {
  account: "Account",
  general: "General",
  models: "Models",
  personalization: "Personalization",
  keyboard: "Keyboard",
  memory: "Memory",
};

const sectionNames = new Map(Object.entries(routeNames).map(([section, route]) => [route.toLowerCase(), section as SettingsSection]));

export function settingsHash(section: SettingsSection): string {
  return `#settings/${routeNames[section]}`;
}

export function settingsSectionFromHash(hash: string): SettingsSection | null {
  const match = /^#settings\/([^/?#]+)$/i.exec(hash.trim());
  if (!match) return null;
  let route = match[1];
  try { route = decodeURIComponent(route); } catch { return null; }
  return sectionNames.get(route.toLowerCase()) ?? null;
}
