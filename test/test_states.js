import { Main } from '../src/index.jsx'
import { setAll } from 'silkjs';

describe('Render all states', () => {
    const main = Main();

    for (let i = 4; i >= 0; i--) {
        it(`state ${i} should render`, () => {
            setAll({
                    state: i,
                },
                true
            )
            
        })
    }
})