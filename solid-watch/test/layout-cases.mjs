// Layout-parity cases: a tree + viewport; the expected frames come from
// yoga-layout with the stub measurer (test/make-layout-fixtures.mjs).
// Tree node: { type, props?, children? } or a string (raw text).
const box = (props = {}, children = []) => ({ type: "view", props, children });
const text = (props, ...children) => ({ type: "text", props, children });
const W = { width: 198, height: 242 };

export const cases = [
  { name: "column-stretch", viewport: W, tree: box({}, [box({ height: 20 }), box({ height: 30 }), box({ height: 10, width: 50 })]) },
  { name: "row-space-between", viewport: W, tree: box({ flexDirection: "row", justifyContent: "space-between" }, [box({ width: 40, height: 20 }), box({ width: 30, height: 30 }), box({ width: 20, height: 10 })]) },
  { name: "row-space-evenly-center", viewport: W, tree: box({ flexDirection: "row", justifyContent: "space-evenly", alignItems: "center", height: 60 }, [box({ width: 40, height: 20 }), box({ width: 30, height: 50 })]) },
  { name: "padding-margin-gap", viewport: W, tree: box({ padding: 8, gap: 6 }, [box({ height: 20, margin: 4 }), box({ height: 20, marginHorizontal: 10, marginTop: 3 }), box({ paddingVertical: 5, paddingLeft: 7 }, [box({ height: 10 })])]) },
  { name: "percent", viewport: W, tree: box({ flexDirection: "row", height: 40 }, [box({ width: "50%" }), box({ width: "25%", height: "50%" }), box({ flexGrow: 1, marginLeft: "5%" })]) },
  { name: "absolute", viewport: W, tree: box({ height: 100 }, [box({ position: "absolute", top: 5, right: 10, width: 30, height: 20 }), box({ position: "absolute", left: "10%", bottom: 0, right: 0, height: 15 }), box({ height: 40 })]) },
  { name: "flex-grow", viewport: W, tree: box({ flexDirection: "row", height: 30 }, [box({ flexGrow: 1 }), box({ flexGrow: 2 }), box({ width: 18 })]) },
  { name: "flex-shrink", viewport: { width: 176, height: 215 }, tree: box({ flexDirection: "row", height: 30 }, [box({ width: 120, flexShrink: 1 }), box({ width: 120, flexShrink: 3 }), box({ width: 20 })]) },
  { name: "flex", viewport: W, tree: box({ height: 200 }, [box({ flex: 1 }), box({ flex: 3 }), box({ height: 20 })]) },
  { name: "wrap", viewport: { width: 176, height: 215 }, tree: box({ flexDirection: "row", flexWrap: "wrap", gap: 4, alignContent: "flex-start" }, [1, 2, 3, 4, 5].map(() => box({ width: 60, height: 20 }))) },
  { name: "aspect-ratio", viewport: W, tree: box({ alignItems: "flex-start" }, [box({ width: 80, aspectRatio: 2 }), box({ height: 30, aspectRatio: 0.5 })]) },
  { name: "display-none", viewport: W, tree: box({}, [box({ height: 20 }), box({ height: 50, display: "none" }), box({ height: 20 })]) },
  { name: "margin-auto", viewport: W, tree: box({ flexDirection: "row", height: 40 }, [box({ width: 40, marginLeft: "auto" }), box({ width: 20, marginVertical: "auto", height: 10 })]) },
  { name: "min-max", viewport: W, tree: box({ flexDirection: "row", height: 40 }, [box({ flexGrow: 1, maxWidth: 50 }), box({ flexGrow: 1, minWidth: 120 }), box({ width: 10, minHeight: 60 })]) },
  { name: "border-content-box", viewport: W, tree: box({}, [box({ borderWidth: 3, padding: 2, height: 30 }), box({ borderWidth: 3, padding: 2, height: 30, width: 50, boxSizing: "content-box" })]) },
  { name: "text-wrap", viewport: W, tree: box({ width: 120, padding: 2 }, [text({ fontSize: 10 }, "The quick brown fox jumps over the lazy dog"), text({ fontSize: 10, numberOfLines: 2 }, "The quick brown fox jumps over the lazy dog"), text({ fontSize: 10, numberOfLines: 1 }, "The quick brown fox jumps")]) },
  { name: "text-newlines", viewport: W, tree: box({ alignItems: "flex-start" }, [text({ fontSize: 12 }, "one\ntwo\nthree")]) },
  { name: "text-shrink", viewport: { width: 176, height: 215 }, tree: box({ padding: 4 }, [text({ fontSize: 40, minScale: 0.5 }, "12345678"), text({ fontSize: 40, minScale: 0.5 }, "1234567890123"), text({ fontSize: 40, minScale: 0.5 }, "12"), text({ fontSize: 30, minScale: 0.8, lineHeight: 30 }, "123456789")]) },
  { name: "text-spans", viewport: W, tree: box({ alignItems: "center" }, [text({ fontSize: 20 }, "Speed ", text({ fontSize: 12 }, "kn"), " now"), text({ fontSize: 14 }, text({ fontWeight: "bold" }, "a"), text({ lineHeight: 30 }, "b"))]) },
  { name: "text-baseline-row", viewport: W, tree: box({ flexDirection: "row", alignItems: "baseline", gap: 2 }, [text({ fontSize: 22 }, "+0.5"), text({ fontSize: 12 }, "kn")]) },
  { name: "text-row-shrink", viewport: W, tree: box({ flexDirection: "row", padding: 4, gap: 4, alignItems: "center" }, [text({ fontSize: 20, minScale: 0.5, flexShrink: 1 }, "Count 123456789012 items"), box({ flexGrow: 1, height: 10 })]) },
  { name: "empty-text", viewport: W, tree: box({ alignItems: "flex-start" }, [text({ fontSize: 20 }, ""), text({ fontSize: 20 }), box({ height: 5 })]) },
  { name: "scroll-content", viewport: { width: 176, height: 215 }, tree: box({ height: 215 }, [{ type: "scroll", props: { flexGrow: 1, paddingBottom: 6 }, children: [1, 2, 3, 4, 5, 6].map(() => box({ height: 50, marginBottom: 4 })) }]) },
  { name: "button-haptic", viewport: W, tree: box({ gap: 4 }, [{ type: "button", props: { height: 32, alignItems: "center", justifyContent: "center", onPress: true }, children: [text({}, "Go")] }, text({ fontSize: 54, minScale: 0.5, tabular: true }, "01:05"), { type: "haptic", props: { pattern: "click", trigger: 1 } }]) },
];
