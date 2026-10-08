// The confirm widget's behaviour, shared by the Kindle pages (client-js) and
// the watch bundles (watch-js), which draw it with their own primitives:
//
//   doConfirm(callback, count)  shows the confirm button; `count` taps on it
//                               (default 1) run `callback`
//   cancel()                    hides it, forgetting the callback
//   tap()                       the confirm button's tap handler
//
// Each tap moves the button to `place()` (the Kindle: a random spot; the
// watch: the centre) and gives 5 s for the next one; 5 s without a tap
// cancels. Uses the standard setTimeout / clearTimeout. No imports, so it
// builds the same in every client.
//
//   show()      the confirm step starts: show the button
//   hide()      it ends (confirmed or cancelled)
//   move(pos)   where the button is now (whatever `place()` returned)

export const CONFIRM_TIMEOUT = 5000;

export function confirmLogic({ place, show, hide, move }) {
    var _count = -1;
    var _callback = null;
    var _removeTimeout = null;

    function confirmFn(callback, count) {
        _callback = callback;
        _count = typeof count === "number" ? count : 1;

        show();
        onClickHandler();
    }

    function cancelFn() {
        if (_removeTimeout !== null) {
            clearTimeout(_removeTimeout);
            _removeTimeout = null;
        }
        hide();
        _count = -1;
        _callback = null;
    }

    function onClickHandler() {
        if (_removeTimeout !== null) {
            clearTimeout(_removeTimeout);
            _removeTimeout = null;
        }

        if (_count > 0) {
            move(place());
            _removeTimeout = setTimeout(cancelFn, CONFIRM_TIMEOUT);
        } else {
            hide();
            _callback();
        }
        _count -= 1;
    }

    return { confirm: confirmFn, cancel: cancelFn, tap: onClickHandler };
}
