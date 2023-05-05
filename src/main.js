import { state, STATE_INIT, STATE_IDLE, STATE_SEQ, STATE_RACE } from './common.js';
import { Idle } from './st_idle.js';
import { Race } from './st_race.js';
import { Setup, start as startSetup } from './st_setup.js';
import { Sequence } from './st_sequence.js';

function Main() {
  switch (state()) {
    case STATE_INIT:
      return <Setup/>;
    case STATE_IDLE:
      return <Idle/>;
    case STATE_SEQ:
      return <Sequence/>;
    case STATE_RACE:
      return <Race/>;
    default:
      return <div><h1>Unknown state</h1></div>;
  }
}

document.body.appendChild(Main());
startSetup();

