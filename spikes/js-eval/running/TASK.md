# Running

I want a simple running screen for my watch.

When I start a run, the app should show me:

- **How long I've been running.** The clock starts when the GPS first gets a position.
- **How far I've run**, in metres, adding up the GPS track as I go.
- **My pace right now**, in seconds per kilometre, worked out over roughly the last 30 seconds so it doesn't jump around but still reacts when I speed up or slow down. Leave it blank until it can be worked out.
- **My kilometre splits:** every time I complete another kilometre (1 km, 2 km, 3 km, ...), how long that kilometre took. Show the last 5 completed kilometres, oldest first; before I've finished the first kilometre the list is empty.
- **Laps:** there's a lap button. The current lap time counts from the start of the run, or from the last time I pressed lap. When I press it, show me how long the lap I just finished took (blank until I've pressed lap once). Pressing lap doesn't change my kilometre splits.

The exact names and units of what's shown are in `view.schema.json`; what the app receives is in `input.schema.json`.
