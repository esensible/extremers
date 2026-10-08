// The countdown's buzzes, as the native watch app had them (Haptics.swift,
// CountdownCues): each whole minute, 30 s, 10 s, each of the last five
// seconds, and the gun. The tick loops decide when (sequence.jsx: each
// second the countdown passes; race.jsx: the device saying the race has
// started); <Haptic /> is solid-watch's haptic node, which plays `pattern`
// whenever `trigger` changes.
import { createSignal } from "solid-js";

// A cue whose second passed longer ago than this (the countdown was just
// shown, or bumped) is not played late.
export const CUE_LATE_MS = 250;
// Re-anchoring (a new state, a bump) can bring a second that just buzzed
// back: nothing this soon after the last buzz (the closest cues are 1 s apart).
const REFIRE_GUARD_MS = 600;
// A gun from the device within this long of the local one is the same gun.
const GUN_GUARD_MS = 60000;

const [cue, setCue] = createSignal({ pattern: "click", n: 0 });
var lastBuzz = -Infinity;
var lastGun = -Infinity;

function buzz(pattern) {
  const now = new Date().getTime();
  lastBuzz = now;
  if (pattern === "success") lastGun = now;
  // a setter function: no read of cue() inside the caller's effect
  setCue((c) => ({ pattern: pattern, n: c.n + 1 }));
}

// The pattern for `s` whole seconds before the gun, if that second buzzes.
export function cuePattern(s) {
  if (s === 0) return "success";
  if (s >= 1 && s <= 5) return "click";
  if (s === 10 || s === 30) return "directionUp";
  if (s > 0 && s % 60 === 0) return "notification";
  return null;
}

// The countdown just passed `s` seconds before the gun, `lateMs` ago.
export function countdownCue(s, lateMs) {
  const pattern = cuePattern(s);
  if (pattern === null || lateMs >= CUE_LATE_MS) return;
  if (new Date().getTime() - lastBuzz < REFIRE_GUARD_MS) return;
  buzz(pattern);
}

// The device says the race started, `sinceMs` after the gun. Its state can
// beat the countdown's own gun (the countdown stops when the phase
// changes): buzz the gun unless it already went.
export function raceStarted(sinceMs) {
  if (sinceMs >= 5000) return;
  if (new Date().getTime() - lastGun < GUN_GUARD_MS) return;
  buzz("success");
}

export const Haptic = () => <haptic pattern={cue().pattern} trigger={cue().n} />;
