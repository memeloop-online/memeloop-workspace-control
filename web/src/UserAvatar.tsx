import type { CSSProperties } from "react";
import { avatarHue, initials } from "./userIdentity";

interface Props {
  displayName: string;
  userId: string;
  avatarUrl?: string | null;
  size?: "small" | "large";
}

export function UserAvatar({ displayName, userId, avatarUrl, size = "small" }: Props) {
  const style = { "--avatar-hue": avatarHue(userId) } as CSSProperties;
  return <span className={`user-avatar ${size}`} style={style} aria-label={displayName}>
    {avatarUrl ? <img src={avatarUrl} alt="" referrerPolicy="no-referrer" /> : <span aria-hidden="true">{initials(displayName)}</span>}
  </span>;
}
