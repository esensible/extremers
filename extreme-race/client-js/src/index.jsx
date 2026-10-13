import { phase, IDLE, COUNTDOWN, RACING } from './common.jsx';
import { Active } from './idle.jsx';
import { Race } from './race.jsx';
import { Sequence } from './sequence.jsx';
import { Switch, Match, Show } from 'solid-js';
import { confirm } from './confirm.jsx';
import { exit, connected } from './api.js';

import './touch.js';
import './style.css'

const [Confirm, doConfirm] = confirm();

export const Main = () => (
  <Switch fallback={<div><h1>Loading...</h1></div>}>
    <Match when={phase() === IDLE}>
      <Active />
    </Match>
    <Match when={phase() === COUNTDOWN}>
      <Sequence />
    </Match>
    <Match when={phase() === RACING}>
      <Race />
    </Match>
  </Switch>
);

const app = () => (
  <div class="container">
    <button class="exit-button" onClick={() => doConfirm(exit, 2)}></button>
    <Confirm />
    <Main />
    <Show when={!connected()}>
      <div class="connection-lost">Connection lost, reconnecting</div>
    </Show>
  </div>
);

document.body.appendChild(app());
