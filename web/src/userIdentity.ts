export function initials(displayName: string): string {
  const words = displayName.trim().split(/\s+/u).filter(Boolean);
  if (words.length === 0) return "?";
  const segments = words.length === 1 ? Array.from(words[0]).slice(0, 2) : [Array.from(words[0])[0], Array.from(words.at(-1) ?? "")[0]];
  return segments.filter(Boolean).join("").toLocaleUpperCase();
}

export function avatarHue(userId: string): number {
  let hash = 0;
  for (const character of userId) hash = (hash * 31 + character.codePointAt(0)!) % 360;
  return hash;
}
