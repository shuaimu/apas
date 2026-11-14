/**
 * Compact figures for tables of counts and money, such as the Overview's
 * usage table.
 */

const UNITS: Array<{ suffix: string; size: number }> = [
  { suffix: "k", size: 1e3 },
  { suffix: "M", size: 1e6 },
  { suffix: "B", size: 1e9 },
  { suffix: "T", size: 1e12 },
];

/**
 * A count with the largest unit that keeps it under 1,000: `512`, `7.26k`,
 * `25.7B`, `557.0B`. Three or four significant digits, so a total never reads
 * "557002.0M". An exact multiple of its unit drops the decimals (`15k`).
 *
 * Rounding is done before the unit is settled: 999,960 is `1.00M`, never
 * "1000.0k", and 9,996 is `10.0k`, never "10.00k".
 */
export function formatCompactCount(value: number): string {
  if (!Number.isFinite(value)) return "0";
  const sign = value < 0 ? "-" : "";
  const n = Math.abs(value);
  if (n < 1_000) return `${sign}${Math.round(n)}`;
  for (let index = 0; index < UNITS.length; index++) {
    const { suffix, size } = UNITS[index];
    const scaled = n / size;
    const last = index === UNITS.length - 1;
    if (Number.isInteger(scaled) && (scaled < 1_000 || last)) {
      return `${sign}${scaled}${suffix}`;
    }
    // Two decimals below 10, one from there on; a figure that rounds across
    // a boundary takes the next step's precision, or the next unit.
    let text = scaled.toFixed(scaled < 10 ? 2 : 1);
    if (scaled < 10 && Number(text) >= 10) text = scaled.toFixed(1);
    if (Number(text) < 1_000 || last) return `${sign}${text}${suffix}`;
  }
  return `${sign}${n}`;
}

/**
 * Dollars with cents and thousands separators: `$3,009.01`. An amount that is
 * not positive is a dash, because only some providers report cost and an
 * empty cell must not read as "$0 spent". A positive amount under one cent
 * says so instead of rounding to "$0.00".
 */
export function formatUsd(value: number): string {
  if (!(value > 0)) return "—";
  if (value < 0.01) return "<$0.01";
  return `$${value.toLocaleString("en-US", { minimumFractionDigits: 2, maximumFractionDigits: 2 })}`;
}
