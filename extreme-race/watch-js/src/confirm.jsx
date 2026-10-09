// The Kindle's confirm widget (client-common/confirm.js) drawn on the watch:
// [Confirm, doConfirm, cancel]. As on the Kindle, the button goes to a
// random place when it appears and on each tap, so each tap of a multi-tap
// confirm is a fresh aim; on the watch that place is near the centre, clear
// of the exit square and of the other buttons, the button shrinking where
// the room between them is short (client-common/confirm-place.js).
//
//   confirm({ exit, buttons })
//     exit       the exit square's side, as a share of the screen height
//     buttons()  the height of the button rows at the bottom of the screen
//                now (0 for none)
//   <Confirm width={…} height={…} />   the screen, in points
import { createMemo, createSignal } from "solid-js";
import { confirmLogic } from "../../../client-common/confirm.js";
import { seededRandom, confirmDraw, confirmRect, CONFIRM_MIDDLE } from "../../../client-common/confirm-place.js";

// One random source for every confirm in the bundle, seeded from the clock.
const random = seededRandom(Date.now() % 4294967296);

export const confirm = ({ exit, buttons }) => {
  const [shown, setShown] = createSignal(false);
  // the latest placement; mapped into the free band of the moment below
  const [draw, setDraw] = createSignal(CONFIRM_MIDDLE);
  const c = confirmLogic({
    place: () => confirmDraw(random),
    show: () => setShown(true),
    hide: () => setShown(false),
    move: setDraw,
  });

  const Confirm = (props) => {
    const rect = createMemo(() =>
      confirmRect({ width: props.width, height: props.height, top: props.height * exit, bottom: props.height - buttons() }, draw()));
    return (
      <button
        position="absolute"
        left={rect().left}
        top={rect().top}
        width={rect().size}
        height={rect().size}
        borderRadius={rect().size / 2}
        background="accent"
        display={shown() ? "flex" : "none"}
        onPress={c.tap}
      />
    );
  };

  return [Confirm, c.confirm, c.cancel];
};
