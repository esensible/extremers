// The Kindle's confirm widget (client-common/confirm.js) drawn on the watch:
// [Confirm, doConfirm, cancel]; the button sits at the centre of the
// screen (the Kindle's moves to a random spot).
import { createSignal } from "solid-js";
import { confirmLogic } from "../../../client-common/confirm.js";

const SIZE = 60; // pt

const centre = () => ({ left: "50%", top: "50%" });

export const confirm = () => {
  const [shown, setShown] = createSignal(false);
  const [pos, setPos] = createSignal(centre());
  const c = confirmLogic({
    place: centre,
    show: () => setShown(true),
    hide: () => setShown(false),
    move: setPos,
  });

  return [
    () => (
      <button
        position="absolute"
        left={pos().left}
        top={pos().top}
        marginLeft={-SIZE / 2}
        marginTop={-SIZE / 2}
        width={SIZE}
        height={SIZE}
        borderRadius={SIZE / 2}
        background="accent"
        display={shown() ? "flex" : "none"}
        onPress={c.tap}
      />
    ),
    c.confirm,
    c.cancel,
  ];
};
