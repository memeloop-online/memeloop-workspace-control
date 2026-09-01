import { useRef, useState, type CSSProperties } from "react";
import { avatarHue, initials } from "./userIdentity";
import { useI18n } from "./i18n";

interface Props {
  displayName: string;
  userId: string;
  avatarUrl?: string | null;
  size?: "small" | "large";
  onChange?: (value: string | null) => void;
  disabled?: boolean;
}

const MAX_AVATAR_BYTES = 512 * 1024;
const TYPES = new Set(["image/png", "image/jpeg", "image/webp"]);

export function UserAvatar({ displayName, userId, avatarUrl, size = "small", onChange, disabled = false }: Props) {
  const { t } = useI18n();
  const input = useRef<HTMLInputElement>(null);
  const [error, setError] = useState("");
  const editable = Boolean(onChange);
  const select = (file: File | undefined) => {
    if (!file || !onChange) return;
    if (!TYPES.has(file.type)) { setError(t("avatarTypeError")); return; }
    if (file.size > MAX_AVATAR_BYTES) { setError(t("avatarSizeError")); return; }
    const reader = new FileReader();
    reader.onload = () => { if (typeof reader.result === "string") { setError(""); onChange(reader.result); } };
    reader.onerror = () => setError(t("avatarReadError"));
    reader.readAsDataURL(file);
  };
  const style = { "--avatar-hue": avatarHue(userId) } as CSSProperties;
  return <span className={`user-avatar-shell${editable ? " editable" : ""}`}>
    <span className={`user-avatar ${size}`} style={style} aria-label={displayName}>
      {avatarUrl ? <img src={avatarUrl} alt="" referrerPolicy="no-referrer" /> : <span aria-hidden="true">{initials(displayName)}</span>}
    </span>
    {editable && <span className="avatar-controls"><input ref={input} type="file" accept="image/png,image/jpeg,image/webp" hidden disabled={disabled} onChange={(event) => { select(event.target.files?.[0]); event.target.value = ""; }} /><button type="button" className="button avatar-upload-button" disabled={disabled} onClick={() => input.current?.click()} aria-label={t("uploadAvatar")}>{t("uploadAvatar")}</button>{avatarUrl && <button type="button" className="button avatar-remove-button" disabled={disabled} onClick={() => { setError(""); onChange?.(null); }}>{t("removeAvatar")}</button>}{error && <span className="field-error" role="alert">{error}</span>}</span>}
  </span>;
}
