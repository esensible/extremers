import express from 'express';
import assert from 'assert';

const app = express();
const port = 8080;

// Middleware for parsing JSON bodies
app.use(express.json());

export const STATE_INIT = 0
export const STATE_IDLE = 1
export const STATE_SEQ = 2
export const STATE_RACE = 3

// Temporary storage for POSTed data
let state = {
    state: STATE_INIT,
};

let startTimeout = null;

function raceStart() {
    state = {
        state: STATE_RACE,
        startTime: state.startTime,
    }
    console.log("RACE!!");
    sync();
}

// Define the POST /event route
app.post('/event', (req, res) => {
    const now = new Date().getTime();
    console.log(req.body);
    switch (req.body.event) {
        case "setup/push_off":
            assert.strictEqual(state.state, STATE_INIT);
            state = {
                state: STATE_IDLE,
            }
            break;

        case "idle/seq":
            assert.strictEqual(state.state, STATE_IDLE);

            // assert.ok(Math.abs(now - req.body.timestamp < 6));
            state = {
                state: STATE_SEQ,
                startTime: req.body.timestamp + req.body.seconds * 1000,
            }
            const delta = state.startTime - now;
            console.log(`start in: ${delta}`);
            // startTimeout = setTimeout(raceStart, state.startTIme - now);
            startTimeout = setTimeout(raceStart, delta);
            break;

        case "seq/bump":
            clearTimeout(startTimeout);
            assert.strictEqual(state.state, STATE_SEQ);

            if (req.body.seconds == 0) {
                state.startTime -= (state.startTime - req.body.timestamp) % 60000;
            } else {
                state.startTime -= req.body.seconds * 1000;
            }

            if (state.startTime <= now + 500) {
                state = {
                    state: STATE_RACE,
                    startTime: state.startTime,
                }
            } else {
                const delta = state.startTime - now;
                console.log(`bumped to: ${delta}`);
                startTimeout = setTimeout(raceStart, delta);
            }
            break;
   
        case "race/finish":
            assert.strictEqual(state.state, STATE_RACE);
            state = {
                state: STATE_IDLE
            }
            break;

        case "line/stbd":
            break;
        case "line/port":
            break;
        default:
            console.log(`Unknown event: ${req.body.event}`);
            assert.ok(false);
            break;
           
    }
    sync();
    res.status(200).json(state);
});


let pendingResponses = [];

function sync() {
    pendingResponses.forEach((response) => {
        response.json(state);
    });
    pendingResponses = [];
}


app.get('/sync', (_, res) => {
    pendingResponses.push(res);
});


app.listen(port, () => {
  console.log(`Server running at http://localhost:${port}`);
});