import { createSignal } from 'silkjs';
import './style.css'

const [className, setClassName] = createSignal('');

function toggleClassName() {
  setClassName((className() === '' ? 'highlight' : ''));
}

const app = () => (
    <div>
      <h1 class={className}>Hello, SolidJS!</h1>
      <button onClick={toggleClassName}>Toggle Heading Class</button>
    </div>
  );

document.body.appendChild(app());