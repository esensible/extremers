// Type check of js/jsx.d.ts (tsc -p test/types, jsxImportSource "solid-watch").
// The @ts-expect-error lines must fail to type-check, so a too-loose
// jsx.d.ts fails this too.

export const ok = (
  <view flexDirection="row" padding={4} width="50%" margin="auto" background="#FF0000" gap={2}>
    <text fontSize={20} fontWeight="600" tabular minScale={0.5} color="white">
      a <text fontWeight="bold">b</text>
    </text>
    <scroll flexGrow={1} />
    <button onPress={(p) => p} press="anything" confirm={{ title: "t", detail: "d", label: "l" }} disabled={false} />
    <countdown toMs={1000} format="elapsed" />
    <clock format="h:mm" />
    <haptic pattern="success" trigger={3} />
  </view>
);

// @ts-expect-error: not a flexDirection
export const bad1 = <view flexDirection="sideways" />;
// @ts-expect-error: unknown prop
export const bad2 = <view colour="red" />;
// @ts-expect-error: padding takes no "auto"
export const bad3 = <view padding="auto" />;
// @ts-expect-error: countdown has no children
export const bad4 = <countdown>x</countdown>;
