# Ultra

I'm running a 56 km mountain ultra and I want to know if I'm going to make the cutoffs.

The aid stations, with their distance along the course and the cutoff (time after the race start by which I must get there):

| Aid station | Distance | Cutoff |
|---|---|---|
| Ridge Hut | 12.0 km | 2 h 30 min |
| River Crossing | 25.5 km | 5 h 00 min |
| Summit Camp | 41.0 km | 8 h 30 min |
| Finish | 56.0 km | 12 h 00 min |

I'll start the app on the start line, so the race starts when the GPS first gets a position.

The screen should show:

- **The next aid station's name**, exactly as written above. It starts at Ridge Hut.
- **How far I've gone along the course**, in km. That's the GPS distance (adding up the track), except that GPS is never quite right on trails, so see "checked in" below.
- **How far to the next aid station**, in km: its distance minus how far I've gone, never less than zero.
- **When I'll get there (ETA)**, as a time of day, from my pace over about the last 10 minutes and the distance I have left. My pace changes a lot on climbs, so don't use my average for the whole race. Leave it blank until there's a recent pace to go on.
- **Time in hand:** the next station's cutoff minus my ETA. Negative means I'm on course to miss it. Blank when the ETA is blank.

There's a **"checked in"** button. When I check in at an aid station, set how far I've gone to that station's distance exactly (whatever the GPS said), and make the following station the next one; from then on add the GPS distance on top of the station's distance. (After the Finish there's nothing next; just keep showing the Finish.)

The exact names and units of what's shown are in `view.schema.json`; what the app receives is in `input.schema.json`.
