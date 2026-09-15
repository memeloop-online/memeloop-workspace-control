import { useRef, useState } from "react";
import {
  Avatar,
  Button,
  Caption1,
  makeStyles,
  tokens,
} from "@fluentui/react-components";
import { ImageAddRegular } from "@fluentui/react-icons";

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

const useStyles = makeStyles({
  root: {
    display: "inline-flex",
    alignItems: "center",
    gap: tokens.spacingHorizontalM,
    minWidth: 0,
  },
  avatar: {
    flexShrink: 0,
  },
  controls: {
    display: "flex",
    flexWrap: "wrap",
    alignItems: "center",
    gap: tokens.spacingHorizontalS,
    minWidth: 0,
  },
  hiddenInput: {
    display: "none",
  },
  error: {
    flexBasis: "100%",
    color: tokens.colorPaletteRedForeground1,
  },
});

export function UserAvatar({ displayName, userId, avatarUrl, size = "small", onChange, disabled = false }: Props) {
  const { t } = useI18n();
  const classes = useStyles();
  const input = useRef<HTMLInputElement>(null);
  const [error, setError] = useState("");
  const editable = Boolean(onChange);

  function select(file: File | undefined) {
    if (!file || !onChange) return;
    if (!TYPES.has(file.type)) {
      setError(t("avatarTypeError"));
      return;
    }
    if (file.size > MAX_AVATAR_BYTES) {
      setError(t("avatarSizeError"));
      return;
    }
    const reader = new FileReader();
    reader.onload = () => {
      if (typeof reader.result === "string") {
        setError("");
        onChange(reader.result);
      }
    };
    reader.onerror = () => setError(t("avatarReadError"));
    reader.readAsDataURL(file);
  }

  return <span className={classes.root}>
    <Avatar
      className={classes.avatar}
      name={displayName || userId}
      image={avatarUrl ? { src: avatarUrl } : undefined}
      size={size === "large" ? 64 : 32}
      color="colorful"
      aria-label={displayName}
    />
    {editable && <span className={classes.controls}>
      <input
        ref={input}
        className={classes.hiddenInput}
        type="file"
        accept="image/png,image/jpeg,image/webp"
        disabled={disabled}
        onChange={(event) => {
          select(event.target.files?.[0]);
          event.target.value = "";
        }}
      />
      <Button
        appearance="secondary"
        icon={<ImageAddRegular />}
        disabled={disabled}
        onClick={() => input.current?.click()}
      >
        {t("uploadAvatar")}
      </Button>
      {avatarUrl && <Button
        appearance="subtle"
        disabled={disabled}
        onClick={() => {
          setError("");
          onChange?.(null);
        }}
      >
        {t("removeAvatar")}
      </Button>}
      {error && <Caption1 className={classes.error} role="alert">{error}</Caption1>}
    </span>}
  </span>;
}
