import { createSignal } from "./api.js" // maintains a map for RESTful updates

const CONFIRM_TIMEOUT = 5000;
const BORDER_SIZE = 15; // vw
const SCREEN_WIDTH = 100; // vw
// don't confirm over the buttons
const SCREEN_HEIGHT = 75; // vh

export const confirm = () => {
    const [class_, setClass] = createSignal("confirm");
    const [pos, setPos] = createSignal({});
    var _count = -1;
    var _callback = null;
    var _removeTimeout = null;

    function confirmFn(callback, count) {
        _callback = callback;
        _count = typeof count === "number" ? count: 1;

        setClass("active");    
        onClickHandler();
    }

    function cancelFn() {
        if (_removeTimeout !== null) {
            clearTimeout(_removeTimeout);
            _removeTimeout = null;
        }
        setClass("");
        _count = -1;
        _callback = null;
    }

    function onClickHandler() {
        if (_removeTimeout !== null) {
            clearTimeout(_removeTimeout);
            _removeTimeout = null;
        }
    
        if (_count > 0) {

            const leftPosition = Math.random() * (SCREEN_WIDTH - 2 * BORDER_SIZE) + BORDER_SIZE;
            const topPosition = Math.random() * (SCREEN_HEIGHT - 2 * BORDER_SIZE) + BORDER_SIZE; 
        
            setPos({
                left: leftPosition + 'vw',
                top: topPosition + 'vh',
            })
            _removeTimeout = setTimeout(cancelFn, CONFIRM_TIMEOUT);
        } else {
            setClass("");
            _callback();
        }      
        _count -= 1;
    }

    return [
            () => (
                <button class={`confirm ${class_()}`} style={pos()} onClick={onClickHandler}></button>
            ),
            confirmFn,
            cancelFn,
        ]
}


