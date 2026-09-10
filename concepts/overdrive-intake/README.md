# Overdrive intake

This browser prototype feeds the `overdrive` wordmark into the orange rotor one letter at a time as you scroll. Each leading letter curls inward, shrinks, and disappears; the remaining letters advance intact. Reverse scrolling brings the letters back out. The range input controls the same sequence.

Open `index.html` directly in a browser. `preview.gif` records the forward and reverse sequence; `preview.png` shows the page. The prototype has no dependencies and respects reduced-motion preferences.

To serve it over HTTP:

```sh
cd concepts
python3 -m http.server 8000
```

Then open `http://localhost:8000/overdrive-intake/`. The page references the existing logo asset at `../logo-directions/impeller.png`; keep that relative path when copying the prototype. `intake.js` drives both the header and enlarged logo, with a stable layout footprint.

The original artwork rotates around its measured visual centre (`49.497% 51.521%`), correcting the offset inside the PNG canvas. Letters converge on that same pivot.
