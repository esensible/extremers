// Text formats, as watch-app/Extremers/Model/Formatting.swift.

/** U+2212, the typographic minus. */
export const MINUS = "−";

/** Knots with one decimal: "6.4". */
export const knots = (k) => k.toFixed(1);

/** One decimal with a sign: "+0.5", "−12.3", "0.0". */
export function signed(x) {
  // Swift's rounded(): half away from zero (Math.round rounds -0.5 up)
  const tenths = Math.sign(x) * Math.round(Math.abs(x * 10));
  if (tenths === 0) return "0.0";
  return (tenths > 0 ? "+" : MINUS) + (Math.abs(tenths) / 10).toFixed(1);
}
