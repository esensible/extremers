// Touch guards for the Kindle's old WebKit browser.
//
// The viewport meta tag (see rollup.config.js) is the main defence against
// pinch and double-tap zoom. These are the belt and braces: block the
// WebKit gesture events and any multi-touch sequence (water droplets on the
// screen register as extra fingers), and the long-press text selection and
// context menu that an e-reader is keen to offer.

function block(event) {
  event.preventDefault();
}

function blockMultiTouch(event) {
  if (event.touches && event.touches.length > 1) {
    event.preventDefault();
  }
}

// `{ passive: false }` is needed for preventDefault to work in modern
// browsers; old WebKit reads the object as `capture = true`, which is fine.
const options = { passive: false };

['gesturestart', 'gesturechange', 'gestureend'].forEach((type) =>
  document.addEventListener(type, block, options)
);
document.addEventListener('touchstart', blockMultiTouch, options);
document.addEventListener('touchmove', blockMultiTouch, options);
document.addEventListener('contextmenu', block);
document.addEventListener('selectstart', block);
