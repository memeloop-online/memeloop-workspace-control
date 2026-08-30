interface Props {
  className?: string;
  size?: number;
}

const ICON_PATH = "/memeloop-workspace-control-icon.png";

export function BrandIcon({ className = "", size = 38 }: Props) {
  return (
    <img
      alt=""
      aria-hidden="true"
      className={`brand-icon-image ${className}`.trim()}
      decoding="async"
      height={size}
      src={ICON_PATH}
      width={size}
    />
  );
}
