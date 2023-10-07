import { state, setState, STATE_IDLE, STATE_ACTIVE, STATE_SEQ, STATE_RACE } from './common.jsx';
import { Active } from './idle.jsx';
import { Race } from './race.jsx';
import { Idle } from './setup.jsx';
import { Sequence } from './sequence.jsx';

import './style.css'

export const Main = () => {
  switch (state()) {
    case STATE_IDLE:
      return <Idle/>;
    case STATE_ACTIVE:
      return <Active/>;
    case STATE_SEQ:
      return <Sequence/>;
    case STATE_RACE:
      return <Race/>;
    default:
      return <div><h1>Loading...</h1></div>;
  }
};

const app = () => (
  <div><Main/></div>
);

document.body.appendChild(app());
