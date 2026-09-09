/* Load after inline copies of overdrive-speedometer-animated.svg.
 * Scroll position drives the needle; scroll distance scrubs the flame cycle.
 * No dependencies. Native SVG/CSS handles the optional looping preview.
 */
(() => {
  const logos = [...document.querySelectorAll('svg.od-logo')];
  if (!logos.length) return;

  const preference = window.matchMedia('(prefers-reduced-motion: reduce)');
  const buttons = [...document.querySelectorAll('button[data-mode]')];
  const meter = document.querySelector('#rev-fill');
  const readout = document.querySelector('#revs');
  let mode = 'scroll';
  let frame = 0;
  let lastTime = 0;
  let position = Math.max(0, window.scrollY);
  let limit = 1;

  function measure() {
    limit = Math.max(1, document.documentElement.scrollHeight - window.innerHeight);
  }

  function paint() {
    const progress = Math.min(1, Math.max(0, position / limit));
    // One six-pose cycle per 240 CSS pixels; negative scroll reverses it.
    const phase = -(((position / 240) % 1) * .9);
    for (const logo of logos) {
      logo.style.setProperty('--od-angle', `${-54 + progress * 115}deg`);
      logo.style.setProperty('--od-phase', `${phase}s`);
    }
    if (meter) meter.style.transform = `scaleX(${progress})`;
    if (readout) readout.textContent = `${String(Math.round(progress * 100)).padStart(3, '0')}%`;
  }

  function tick(time) {
    frame = 0;
    if (mode !== 'scroll' || preference.matches) return;
    const target = Math.min(limit, Math.max(0, window.scrollY));
    const elapsed = lastTime ? Math.min(64, time - lastTime) : 16;
    lastTime = time;
    position += (target - position) * (1 - Math.exp(-elapsed / 65));
    if (Math.abs(target - position) < .1) position = target;
    paint();
    if (position !== target) frame = requestAnimationFrame(tick);
    else lastTime = 0;
  }

  function requestTick() {
    if (!frame && mode === 'scroll' && !preference.matches) {
      frame = requestAnimationFrame(tick);
    }
  }

  function selectMode(next) {
    mode = next;
    cancelAnimationFrame(frame);
    frame = 0;
    lastTime = 0;
    const effective = preference.matches ? 'still' : mode;
    for (const logo of logos) {
      logo.dataset.motion = effective;
      logo.style.removeProperty('--od-phase');
      logo.style.removeProperty('--od-angle');
    }
    for (const button of buttons) {
      button.setAttribute('aria-pressed', String(button.dataset.mode === mode));
    }
    if (effective === 'scroll') {
      measure();
      position = Math.min(limit, Math.max(0, window.scrollY));
      paint();
    } else {
      if (meter) meter.style.transform = effective === 'loop' ? 'scaleX(1)' : 'scaleX(0)';
      if (readout) readout.textContent = effective === 'loop' ? 'AUTO' : 'HOLD';
    }
  }

  for (const button of buttons) {
    button.addEventListener('click', () => selectMode(button.dataset.mode));
  }
  window.addEventListener('scroll', requestTick, { passive: true });
  window.addEventListener('resize', () => { measure(); requestTick(); }, { passive: true });
  window.addEventListener('pageshow', () => { measure(); requestTick(); });
  preference.addEventListener('change', () => selectMode(mode));
  measure();
  selectMode(mode);
})();
