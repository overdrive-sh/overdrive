# Overdrive flame speedometer

An original circular speedometer with sweeping, pointed flames in Overdrive orange
(`#ff5c28`). Transparent background, with dark separator shapes (`#141413`) on
the flame-facing side of the dial, and no font or library dependencies in the
SVG assets.

## Files

- `index.html` — open directly in a browser for the scroll preview, with Loop
  and Still controls and 24px / 48px samples. No server or build is needed.
- `overdrive-speedometer-animated.svg` — standalone looping SVG: six flame
  poses in 0.9 seconds, with a 2.7-second needle cycle.
- `overdrive-speedometer.svg` — static SVG for applications that strip animation.
- `overdrive-scroll.js` — optional controller for inline copies of the animated
  SVG. Scroll down to advance the needle and flames; scroll up to reverse them.
  The animation settles when scrolling stops.
- `preview.gif` — recorded animation preview on a dark background.
- `preview.png` — transparent still preview.

## Embed

For the looping asset:

```html
<img src="/brand/overdrive-speedometer-animated.svg"
     width="64" height="58" alt="Overdrive">
```

For scroll control, paste the **contents** of
`overdrive-speedometer-animated.svg` into your page, set its display width
(for example `width="48" height="44"`), and load:

```html
<script src="/brand/overdrive-scroll.js" defer></script>
```

The script finds inline `svg.od-logo` elements. An SVG inside `<img>` plays its
own loop and cannot observe the parent page's scrolling. The controller can
drive multiple inline copies, as demonstrated by the preview's header and
size samples. It uses the document's scroll position; custom scroll containers
would need their own binding.

The needle spans the page's scroll range. Flames cycle every 240 CSS pixels,
with a brief easing interval after input. Both the standalone SVG and the
controller respect `prefers-reduced-motion`; the static asset contains no
animation. Still mode displays the needle in its fixed brand pose.

To recolour the orange artwork, change the root SVG's `fill="#ff5c28"`; keep the
separator shapes at `#141413` unless the contrast treatment should also change.
All geometry is editable vector path data. The preview embeds the animated SVG
in `#logo-template` so it works even when opened from disk; update that
template if the asset changes.

## Visual reference

[Kong's navigation logo](https://konghq.com/) at `.MainNav_logo__klR_9` informed
the one-colour silhouette and discrete animation poses. The supplied flame and
speedometer images informed the longer flame tongues and circular dial. The
paths are newly drawn for Overdrive. Kong's gorilla runs a walk cycle
on scroll activity and flips on upward scrolling; this speedometer instead
scrubs its flame poses and needle with scroll distance. No Kong artwork is
included in these deliverables.
