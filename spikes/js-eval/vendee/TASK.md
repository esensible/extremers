# Vendée

I'm racing solo, non-stop around the world, and I want a simple navigation screen.

The course goes through these waypoints, in this order, starting from Les Sables d'Olonne (the start is not a waypoint; my first target is Finisterre):

| # | Name | Latitude | Longitude |
|---|---|---|---|
| 1 | Finisterre | 43.0 N | 10.0 W |
| 2 | Canaries | 28.0 N | 19.0 W |
| 3 | Equator Out | 0.0 | 27.0 W |
| 4 | Good Hope | 42.0 S | 18.5 E |
| 5 | Leeuwin | 45.0 S | 115.0 E |
| 6 | Campbell | 50.0 S | 170.0 E |
| 7 | Point Nemo | 49.0 S | 123.0 W |
| 8 | Cape Horn | 57.0 S | 67.3 W |
| 9 | Equator Home | 0.0 | 30.0 W |
| 10 | Finish | 46.47 N | 1.79 W |

The screen should show:

- **The name of my next waypoint**, exactly as written above. When I get within **10 nautical miles** of it, it counts as passed and the next one in the list becomes my target. (Once the finish is my target, it stays the target.)
- **Distance to the next waypoint**, in nautical miles, straight line (great circle) from where I am.
- **Distance to the finish**, in nautical miles, going the course: from where I am to the next waypoint, then waypoint to waypoint in order through the rest of the list, each leg a straight great-circle line, ending at the Finish.
- **My 24-hour run:** the distance I've actually sailed, along my track, over the last 24 hours, in nautical miles. Before I've been sailing for 24 hours, just show the distance sailed since my first position. My position can come every second or only every few minutes, and the app can't keep a huge history; being out by a few miles is fine.
- **My VMG towards the next waypoint:** how fast I'm actually closing on it, in knots, from the speed and course my GPS gives me. It's negative if I'm sailing away from it.

Until the GPS has given me a position, leave the distances blank, and leave the VMG blank until I have a position and a speed. The 24-hour run starts at zero.

A nautical mile is 1852 metres. The exact names and units of what's shown are in `view.schema.json`; what the app receives is in `input.schema.json`.
