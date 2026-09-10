# Overdrive hover study

This standalone browser demo gives the Overdrive impeller a gentle spool-up on pointer hover, keyboard focus, or a held touch press. Move away or release to let it coast down. The controller also respects `prefers-reduced-motion` and leaves the mark still when motion is reduced.

Open [`index.html`](index.html) directly in a browser. The demo has no dependencies: `hover.js` owns the rotation, while the page keeps the mark as one focused button with a visible keyboard focus ring.

The impeller uses the existing relative PNG at `../logo-directions/impeller.png`; keep that path when copying the prototype. The source artwork is unchanged, including its transparent padding and measured rotation pivot.

`preview.png` shows the demo; `preview.gif` records the acceleration and coast-down.
