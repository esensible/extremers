// The Kindle race page's style.css, as solid-watch props: the big numbers
// (.speed, .heading, .countdown, .time), and the buttons (button,
// button.refresh: a set line end drawn without fill, .corner-label).

// .speed / .heading / .countdown / .time
export const Big = (props) => (
  <text fontSize={46} fontWeight="600" tabular textAlign="center" minScale={0.5} alignSelf="stretch">
    {props.children}
  </text>
);

// A row button. `refresh`: no fill, an outline. `corner`: the label small
// in that bottom corner (.corner-label), as when both line ends are set.
export const Pad = (props) => (
  <button
    onPress={props.onPress}
    flexGrow={1}
    flexBasis={0}
    alignSelf="stretch"
    borderRadius={8}
    background={props.refresh ? null : "button"}
    borderWidth={props.refresh ? 1 : 0}
    borderColor="button"
    padding={props.corner ? 3 : 0}
    alignItems={props.corner === "left" ? "flex-start" : props.corner === "right" ? "flex-end" : "center"}
    justifyContent={props.corner ? "flex-end" : "center"}
  >
    <text fontSize={props.corner ? 11 : 17} fontWeight="600" numberOfLines={1} minScale={0.6}>
      {props.label}
    </text>
  </button>
);

// A row of Pads (.row)
export const Row = (props) => (
  <view flexDirection="row" gap={4} height={40} alignSelf="stretch">
    {props.children}
  </view>
);

// The buttons at the bottom (.buttons)
export const Buttons = (props) => (
  <view marginTop="auto" gap={4} alignSelf="stretch">
    {props.children}
  </view>
);
