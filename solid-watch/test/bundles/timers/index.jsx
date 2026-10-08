// Timers, the way a browser client of a device uses them: a clock offset
// kept from the timestamp in every state; a countdown to a device time
// driven by a setTimeout loop; a confirm widget (tap the button again,
// `count` times, within 5 s, else it goes away by itself); send(bytes) with
// the tap time stamped in device time.
import { createSignal, createEffect, onCleanup } from "solid-js";
import { createApp, send } from "solid-watch";

// Diagnostics to the host's log (the Swift host's onLog; the recording
// drivers have none, so golden logs don't change).
const note = (m) => {
  const h = globalThis.__host;
  if (h && h.log) h.log(m);
};

// Device clock minus ours, from every state (Date is the host's clock).
var timestampOffset = 0;
function timestamp() {
  return new Date().getTime() + timestampOffset;
}

const [start, setStart] = createSignal(null);
const [countdown, setCountdown] = createSignal("--:--");
const [sent, setSent] = createSignal(0);

const pad2 = (n) => ("0" + n).slice(-2);

// Counts down to the start, re-armed for each whole second.
createEffect(() => {
  const startTimestamp = start();
  if (startTimestamp === null) {
    setCountdown("--:--");
    return;
  }
  var timerId = null;
  onCleanup(() => {
    if (timerId !== null) clearTimeout(timerId);
  });
  function tick() {
    const remaining = startTimestamp - timestamp();
    if (remaining <= 0) {
      setCountdown("00:00");
      timerId = null;
      return;
    }
    const seconds = Math.floor(remaining / 1000);
    setCountdown(pad2(Math.floor(seconds / 60)) + ":" + pad2(seconds % 60));
    timerId = setTimeout(tick, remaining % 1000);
  }
  tick();
});

// The confirm widget: doConfirm(callback, count) shows a button; tapping it
// `count` times runs the callback; 5 s without a tap cancels.
const CONFIRM_TIMEOUT = 5000;
function confirm() {
  const [shown, setShown] = createSignal(false);
  var _count = -1;
  var _callback = null;
  var _removeTimeout = null;
  function confirmFn(callback, count) {
    _callback = callback;
    _count = typeof count === "number" ? count : 1;
    setShown(true);
    onClickHandler();
  }
  function cancelFn() {
    if (_count >= 0) note("confirm: timed out");
    if (_removeTimeout !== null) {
      clearTimeout(_removeTimeout);
      _removeTimeout = null;
    }
    setShown(false);
    _count = -1;
    _callback = null;
  }
  function onClickHandler() {
    if (_removeTimeout !== null) {
      clearTimeout(_removeTimeout);
      _removeTimeout = null;
    }
    if (_count > 0) {
      note("confirm: " + _count + " more tap(s)");
      _removeTimeout = setTimeout(cancelFn, CONFIRM_TIMEOUT);
    } else {
      note("confirm: confirmed");
      setShown(false);
      _callback();
    }
    _count -= 1;
  }
  const Confirm = () => (
    <button position="absolute" top="50%" left="50%" width={56} height={56} marginTop={-28} marginLeft={-28} borderRadius={28} background="#FF9F0A" display={shown() ? "flex" : "none"} onPress={onClickHandler} />
  );
  return [Confirm, confirmFn, cancelFn];
}

const [Confirm, doConfirm] = confirm();

// 6 bytes little-endian of a device time
function bytesOf(ms) {
  const out = [];
  for (let i = 0; i < 6; i++) {
    out.push(ms % 256);
    ms = Math.floor(ms / 256);
  }
  return out;
}

function sendClick() {
  const clickTime = timestamp();
  note("Send tapped at device time " + clickTime);
  doConfirm(() => {
    send([0x01].concat(bytesOf(clickTime)));
    setSent(sent() + 1);
  }, 2);
}

function App() {
  return (
    <view flexGrow={1}>
      <view gap={4} padding={4}>
        <text fontSize={40} tabular>{countdown()}</text>
        <text>sent {sent()}</text>
        <button onPress={sendClick} height={32} background="#333333" borderRadius={10} justifyContent="center" alignItems="center">
          <text>Send</text>
        </button>
      </view>
      <Confirm />
    </view>
  );
}

createApp(App, {
  onState(s) {
    timestampOffset = s.now - new Date().getTime();
    setStart(s.start);
  },
});
