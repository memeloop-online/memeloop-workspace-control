import { makeStyles, mergeClasses, tokens } from "@fluentui/react-components";

interface Props {
  className?: string;
  size?: number;
}

const ICON_PATH = "/memeloop-workspace-control-icon.png";

const useStyles = makeStyles({
  image: {
    display: "block",
    objectFit: "cover",
    borderRadius: tokens.borderRadiusLarge,
    boxShadow: tokens.shadow8,
  },
});

export function BrandIcon({ className, size = 38 }: Props) {
  const styles = useStyles();
  return (
    <img
      alt=""
      aria-hidden="true"
      className={mergeClasses(styles.image, className)}
      decoding="async"
      height={size}
      src={ICON_PATH}
      width={size}
    />
  );
}
