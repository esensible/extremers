import { state, STATE_IDLE, STATE_ACTIVE, STATE_SEQ, STATE_RACE } from './common.jsx';
import { Active } from './idle.jsx';
import { Race } from './race.jsx';
import { Idle } from './setup.jsx';
import { Sequence } from './sequence.jsx';
import { Switch, Match } from 'solid-js';

import './style.css'

export const Main = () => (
  <Switch fallback={<div><h1>Loading...</h1></div>}>
    <Match when={state() === STATE_IDLE}>
      <Idle />
    </Match>
    <Match when={state() === STATE_ACTIVE}>
      <Active />
    </Match>
    <Match when={state() === STATE_SEQ}>
      <Sequence />
    </Match>
    <Match when={state() === STATE_RACE}>
      <Race />
    </Match>
  </Switch>  
  // switch (state()) {
  //   case STATE_IDLE:
  //     return <Idle/>;
  //   case STATE_ACTIVE:
  //     return <Active/>;
  //   case STATE_SEQ:
  //     return <Sequence/>;
  //   case STATE_RACE:
  //     return <Race/>;
  //   default:
  //     return <div><h1>Loading...</h1></div>;
  // }
);

const app = () => (
  <div><Main/></div>
);

document.body.appendChild(app());
