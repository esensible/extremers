// As the Kindle race page's idle.jsx: speed, the wall clock, the line
// buttons and the start-sequence buttons (each behind the confirm widget).
import { speed, time, LineButtons, raceConfirm } from "./common.jsx";
import { postEvent, timestamp } from "./api.js";
import { Big, Pad, Row, Buttons } from "./style.jsx";

const [Confirm, doConfirm] = raceConfirm();

function seqClick(seconds) {
  return () => {
    // the tap, in device time (api.js timestamp()), before the confirm
    const clickTime = timestamp();
    doConfirm(() => { postEvent({ tag: "BumpSeq", value: { seconds: seconds, timestamp: clickTime } }); });
  };
}

// props: the screen ({ width, height }), for the confirm button
export const Active = (props) => (
  <view flexGrow={1}>
    <Big>{speed().toFixed(1)}</Big>
    <Big>{time()}</Big>
    <Buttons>
      <LineButtons />
      <Row>
        <Pad onPress={seqClick(600)} label="10" />
        <Pad onPress={seqClick(300)} label="5" />
        <Pad onPress={seqClick(240)} label="4" />
        <Pad onPress={seqClick(60)} label="1" />
      </Row>
    </Buttons>
    <Confirm width={props.width} height={props.height} />
  </view>
);
