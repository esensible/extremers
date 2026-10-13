// sim/nmea.js — what the host makes of one NMEA line: the GPS update of an
// RMC sentence, or null. A line-at-a-time mirror of common/src/nmea.rs
// (`next_message` + `GNRMC::update`), so the studio knows which update the
// host's `HostHandle.nmea(line)` applied (the host only says how many). The
// sim checks the two agree on every line (W-HOST otherwise): this is a
// mirror for the record, never the authority.

const isUint = (s) => /^\+?\d+$/.test(s);

/** Rust's `str::parse::<f64>()`: decimal or exponent forms, inf/infinity/nan; anything else null. */
function f64(s) {
  if (/^[+-]?(\d+\.?\d*|\.\d+)([eE][+-]?\d+)?$/.test(s)) return Number(s);
  const m = /^([+-]?)(inf|infinity|nan)$/i.exec(s);
  if (m) return /nan/i.test(m[2]) ? NaN : (m[1] === '-' ? -Infinity : Infinity);
  return null;
}

/** `hhmmss.sss` → ms since midnight, or null (nmea.rs parse_utc_time). */
function utcTime(t) {
  if (t.length < 6) return null;
  const [h, m, s] = [t.slice(0, 2), t.slice(2, 4), t.slice(4, 6)];
  if (![h, m, s].every(isUint)) return null;
  const hours = Number(h); const minutes = Number(m); const seconds = Number(s);
  if (hours > 23 || minutes > 59 || seconds > 60) return null;
  let ms = 0;
  if (t.length > 7) {
    let scale = 100;
    for (const c of t.slice(7, 10)) {
      if (c < '0' || c > '9') return null;
      ms += (c.charCodeAt(0) - 48) * scale;
      scale /= 10;
    }
  }
  return hours * 3600000 + minutes * 60000 + seconds * 1000 + ms;
}

/** `ddmm.mmmm` / `dddmm.mmmm` → degrees, or null (nmea.rs parse_angle). */
function angle(t, dd) {
  if (t.length < dd + 2) return null;
  const d = f64(t.slice(0, dd)); const m = f64(t.slice(dd));
  return d === null || m === null ? null : d + m / 60;
}

const MONTH_DAY = [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334];
/** `ddmmyy` → days since 1970-01-01, 2000-2099 (nmea.rs date_to_epoch). */
function date(t) {
  if (t.length !== 6) return null;
  const parts = [t.slice(0, 2), t.slice(2, 4), t.slice(4, 6)];
  if (!parts.every(isUint)) return null;
  const [day, month, year] = parts.map(Number);
  if (day < 1 || day > 31 || month < 1 || month > 12) return null;
  const inMonths = year % 4 === 0 && month > 2 ? MONTH_DAY[month - 1] + 1 : MONTH_DAY[month - 1];
  const leap = Math.floor((year + 2000 - 1 - 1972) / 4) + 1;
  return (year + 2000 - 1970) * 365 + leap + inMonths + day - 1;
}

const xor = (s) => { let x = 0; for (let i = 0; i < s.length; i++) x ^= s.charCodeAt(i); return x; };

/**
 * One line of NMEA text → `{ t?, lat?, lon?, speed?, heading? }` (as
 * HostHandle.gps takes it) when it is an RMC sentence with a valid checksum
 * and a valid fix; else null.
 */
export function rmcUpdate(line) {
  const text = String(line).replace(/\n$/, '');
  const at = text.lastIndexOf('$'); // a `$` starts a new sentence, even mid-line
  if (at < 0) return null;
  const s = text.slice(at);
  const star = s.indexOf('*');
  if (star < 0) return null; // a sentence without a checksum can't be trusted
  const body = s.slice(1, star);
  const tokens = body.split(',');
  if (tokens.length < 2 || (tokens[0] !== 'GPRMC' && tokens[0] !== 'GNRMC')) return null;
  const sum = s.slice(star + 1).replace(/\r+$/, '');
  if (sum.length !== 2 || !/^[0-9a-fA-F]{2}$/.test(sum) || parseInt(sum, 16) !== xor(body)) return null;
  const f = tokens.slice(1);
  const status = f[1];
  const mode = f[11];
  if (status !== 'A') return null;
  if (mode !== undefined && !['A', 'D', 'E'].includes(mode)) return null;
  const out = {};
  const time = f[0] !== undefined ? utcTime(f[0]) : null;
  const days = f[8] !== undefined ? date(f[8]) : null;
  if (time !== null && days !== null) out.t = time + days * 86400000;
  const lat = f[2] !== undefined ? angle(f[2], 2) : null;
  const lon = f[4] !== undefined ? angle(f[4], 3) : null;
  const ns = f[3] ? f[3][0] : undefined;
  const ew = f[5] ? f[5][0] : undefined;
  if (lat !== null && ns !== undefined && lon !== null && ew !== undefined) {
    out.lat = ns === 'S' ? -lat : lat;
    out.lon = ew === 'W' ? -lon : lon;
  }
  const speed = f[6] !== undefined ? f64(f[6]) : null;
  const heading = f[7] !== undefined ? f64(f[7]) : null;
  if (speed !== null && heading !== null) { out.speed = speed; out.heading = heading; }
  return out;
}

/** An RMC sentence for a fix (for tests and tracks): `$GPRMC,…*hh`. */
export function rmcSentence({ t, lat, lon, speed, heading }) {
  const d = new Date(t);
  const two = (n) => String(n).padStart(2, '0');
  const time = `${two(d.getUTCHours())}${two(d.getUTCMinutes())}${two(d.getUTCSeconds())}.${String(d.getUTCMilliseconds()).padStart(3, '0')}`;
  const dm = (v, dd) => { const a = Math.abs(v); const deg = Math.floor(a); return String(deg).padStart(dd, '0') + ((a - deg) * 60).toFixed(6).padStart(9, '0'); };
  const day = `${two(d.getUTCDate())}${two(d.getUTCMonth() + 1)}${two(d.getUTCFullYear() % 100)}`;
  const body = `GPRMC,${time},A,${dm(lat, 2)},${lat < 0 ? 'S' : 'N'},${dm(lon, 3)},${lon < 0 ? 'W' : 'E'},${speed ?? ''},${heading ?? ''},${day},,,A`;
  return `$${body}*${xor(body).toString(16).toUpperCase().padStart(2, '0')}`;
}
