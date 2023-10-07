import { Main } from '../src/index.jsx'
import { setAll } from 'silkjs';
// import { activeSignals } from 'silkjs';

// describe('Render all states', () => {
//     const main = Main();

//     for (let i = 4; i >= 0; i--) {
//         it(`state ${i} should render`, () => {
//             // activeSignals.length = 0;
//             setAll({
//                     state: i,
//                 },
//                 true
//             )
//             // log(`activeSignals: ${activeSignals}`)
//         })
//     }
// })

describe('Race state', () => {
    const main = Main();

    it(`Racing`, () => {
        // activeSignals.length = 0;
        setAll({
                state: 3,
                speed: "2.2",
                heading: "151"
            },
            true
        )
        // log(`activeSignals: ${activeSignals}`)
    });

})