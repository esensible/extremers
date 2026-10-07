import { state, STATE_ACTIVE, STATE_SEQ, STATE_RACE } from './common.jsx';
import { Active } from './idle.jsx';
import { Race } from './race.jsx';
import { Sequence } from './sequence.jsx';
import { Switch, Match, Show } from 'solid-js';
import { confirm } from './confirm.jsx';
import { selectEngine, connected } from './api.js';

import './touch.js';
import './style.css'

const [Confirm, doConfirm] = confirm();

export const Main = () => (
  <Switch fallback={<div><h1>Loading...</h1></div>}>
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
);

const app = () => (
  <div class="container">
    <button class="exit-button" onClick={() => doConfirm(() => selectEngine("Selector"), 2)}></button>
    <Confirm />
    <Main />
    <Show when={!connected()}>
      <div class="connection-lost">Connection lost, reconnecting</div>
    </Show>
  </div>
);

document.body.appendChild(app());
