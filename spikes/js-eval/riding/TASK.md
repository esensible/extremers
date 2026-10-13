# Riding

I'd like a bike computer screen for my rides.

It should show:

- **My speed right now**, in km/h.
- **How far I've ridden**, in kilometres, adding up the GPS track.
- **My ride time.** The clock starts when the GPS first gets a position, and it only runs while I'm riding: see pause below.
- **My average speed**, in km/h: the distance divided by the ride time (so paused time doesn't drag it down). Zero until I've got going.
- **How much I've climbed** in metres, from the GPS altitude. GPS altitude wobbles up and down by a metre or two even on the flat, and I don't want that wobble counted. The rule, using a **3 metre** threshold:
  - At the start I'm not climbing, and the first altitude is my low point.
  - While I'm not climbing, the low point is the lowest altitude so far. Once I'm 3 metres or more above it, I'm climbing: count the whole rise from the low point to here.
  - While I'm climbing, whenever I get higher than the highest point of this climb, count the extra. Dropping less than 3 metres below that high point doesn't end the climb, and climbing back up to the high point isn't counted again.
  - Once I'm 3 metres or more below the climb's high point, the climb is over, and where I am now is my new low point.
  - Going downhill never takes anything off. Some GPS updates come without an altitude; just skip those for climbing, and skip altitudes while I'm paused too.
- **Whether I'm paused.** There's one pause/resume button: press it to pause, press it again to carry on. While I'm paused (at a café, say) the ride time stops and nothing is added to my distance or climbing. Everything starts at zero and not paused.

The exact names and units of what's shown are in `view.schema.json`; what the app receives is in `input.schema.json`.
