import { createDarkTheme, createLightTheme, type BrandVariants, type Theme } from "@fluentui/react-components";

const brand: BrandVariants = {
  10: "#001d2b",
  20: "#003448",
  30: "#004c63",
  40: "#00657d",
  50: "#007e96",
  60: "#0098ad",
  70: "#11a9ba",
  80: "#35baca",
  90: "#58cbd9",
  100: "#78dce7",
  110: "#94e8f0",
  120: "#aff1f6",
  130: "#c4f6fa",
  140: "#d6fafc",
  150: "#e5fcfd",
  160: "#f0feff",
};

export const lightTheme: Theme = createLightTheme(brand);
export const darkTheme: Theme = createDarkTheme(brand);
