import { state, STATE_INIT, STATE_IDLE, STATE_SEQ, STATE_RACE } from './common.jsx';
import { Idle } from './idle.jsx';
// import { Race } from './st_race.js';
import { Setup, start as startSetup } from './setup.jsx';
// import { Sequence } from './st_sequence.js';
import { Confirm } from './confirm.jsx'

import './style.css'

export const Main = () => {
  switch (state()) {
    case STATE_INIT:
      return <Setup/>;
    case STATE_IDLE:
      return <Idle/>;
    // case STATE_SEQ:
    //   return <Sequence/>;
    // case STATE_RACE:
    //   return <Race/>;
    default:
      return <div><h1>Unknown state</h1></div>;
  }
};

const app = () => (
  <div><Main/><Confirm/></div>
);

document.body.appendChild(app());
