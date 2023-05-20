import { state, setState, STATE_INIT, STATE_IDLE, STATE_SEQ, STATE_RACE } from './common.jsx';
import { Idle } from './idle.jsx';
import { Race } from './race.jsx';
import { Setup } from './setup.jsx';
import { Sequence } from './sequence.jsx';

import './style.css'

export const Main = () => {
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
};

const app = () => (
  <div><Main/></div>
);

document.body.appendChild(app());
