(() => {
  const control = document.querySelector('.hover-logo');
  const rotor = control.querySelector('.impeller');
  const reducedMotion = window.matchMedia('(prefers-reduced-motion: reduce)');
  const topSpeed = 1800; // Degrees per second; approach the limit gradually.
  const spoolTime = 2.8;
  const coastTime = .85;
  let hovering = false;
  let pressing = false;
  let keyboardFocused = false;
  let speed = 0;
  let angle = 0;
  let frame = 0;
  let previousTime = null;

  const powered = () => hovering || pressing || keyboardFocused;

  function tick(time) {
    frame = 0;
    const elapsed = previousTime === null ? 0 : Math.min((time - previousTime) / 1000, .064);
    previousTime = time;
    const target = powered() ? topSpeed : 0;
    const duration = powered() ? spoolTime : coastTime;
    const decay = Math.exp(-elapsed / duration);

    // Integrate the changing speed so entering/leaving never resets the angle.
    angle = (angle + target * elapsed + (speed - target) * duration * (1 - decay)) % 360;
    speed = target + (speed - target) * decay;
    rotor.style.transform = `rotate(${angle}deg)`;

    if (powered() || speed >= 1) {
      frame = requestAnimationFrame(tick);
    } else {
      speed = 0;
      previousTime = null;
    }
  }

  function wake() {
    if (reducedMotion.matches || frame) return;
    previousTime = null;
    frame = requestAnimationFrame(tick);
  }

  control.addEventListener('pointerenter', event => {
    if (event.pointerType === 'touch') return;
    hovering = true;
    wake();
  });
  control.addEventListener('pointerleave', event => {
    if (event.pointerType === 'touch') return;
    hovering = false;
    wake();
  });
  control.addEventListener('pointerdown', event => {
    if (event.pointerType !== 'touch') return;
    pressing = true;
    control.setPointerCapture(event.pointerId);
    wake();
  });
  function release() {
    pressing = false;
    wake();
  }
  control.addEventListener('pointerup', release);
  control.addEventListener('pointercancel', release);
  control.addEventListener('lostpointercapture', release);
  control.addEventListener('focus', () => {
    keyboardFocused = control.matches(':focus-visible');
    wake();
  });
  control.addEventListener('blur', () => {
    keyboardFocused = false;
    wake();
  });
  reducedMotion.addEventListener('change', () => {
    cancelAnimationFrame(frame);
    frame = 0;
    previousTime = null;
    speed = 0;
    if (powered()) wake();
  });
})();
