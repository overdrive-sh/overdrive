(() => {
  const roots = [...document.querySelectorAll('[data-intake-logo]')];
  const scrubber = document.querySelector('#scrubber');
  const progressFill = document.querySelector('#progress-fill');
  const progressReadout = document.querySelector('#progress-readout');
  const reducedMotion = window.matchMedia('(prefers-reduced-motion: reduce)');
  const clamp = value => Math.max(0, Math.min(1, value));
  let layouts = [];
  let progress = 0;
  let target = 0;
  let frame = 0;
  let previousTime = 0;

  function measure() {
    layouts = roots.map(root => {
      const rotor = root.querySelector('.impeller');
      const word = root.querySelector('.word');
      const letters = [...word.querySelectorAll('.letter')];
      const savedTransform = rotor.style.transform;
      rotor.style.transform = 'none';
      const rotorBox = rotor.getBoundingClientRect();
      const [originX, originY] = getComputedStyle(rotor).transformOrigin.split(' ').map(Number.parseFloat);
      const wordBox = word.getBoundingClientRect();
      rotor.style.transform = savedTransform;

      return {
        rotor,
        centerX: rotorBox.left + originX - wordBox.left,
        centerY: rotorBox.top + originY - wordBox.top,
        letters: letters.map(letter => ({
          element: letter,
          left: letter.offsetLeft,
          width: letter.offsetWidth,
          centerY: letter.offsetTop + letter.offsetHeight / 2,
        })),
      };
    });
  }

  function render(value) {
    const amount = reducedMotion.matches ? 0 : clamp(value);
    for (const layout of layouts) {
      const count = layout.letters.length;
      const sequence = amount * count;
      const consumed = Math.min(count, Math.floor(sequence));
      const local = sequence - consumed;
      const active = layout.letters[consumed];
      const packed = active ? active.left : 0;
      const next = layout.letters[consumed + 1];
      const advance = active ? (next ? next.left - active.left : active.width) : 0;
      const feed = local * local * (3 - 2 * local);

      layout.rotor.style.transform = `rotate(${amount * count * 360}deg)`;

      layout.letters.forEach((letter, index) => {
        const baseX = letter.left + letter.width / 2;
        let x = -packed - advance * feed;
        let y = 0;
        let scale = 1;
        let angle = 0;
        let opacity = 1;

        if (index < consumed) {
          x = layout.centerX - baseX;
          y = layout.centerY - letter.centerY;
          scale = 0;
          opacity = 0;
        } else if (index === consumed && local > 0) {
          // Only the leading letter is drawn inward. The rest advance intact.
          const startX = baseX - packed;
          const pull = Math.pow(local, 1.25);
          const radius = (startX - layout.centerX) * (1 - pull);
          const curl = local * local * 1.15;
          x = layout.centerX + radius * Math.cos(curl) - baseX;
          y = (layout.centerY - letter.centerY) * pull
            + radius * Math.sin(curl) * .55;
          scale = Math.pow(1 - local, 1.35);
          angle = curl * 180 / Math.PI;
          opacity = 1 - clamp((local - .82) / .18);
        }

        letter.element.style.transform = `translate(${x}px, ${y}px) rotate(${angle}deg) scale(${scale})`;
        letter.element.style.opacity = String(opacity);
      });
    }

    const count = layouts[0]?.letters.length ?? 9;
    const consumed = Math.min(count, Math.floor(amount * count));
    if (progressFill) progressFill.style.transform = `scaleX(${amount})`;
    if (progressReadout) progressReadout.textContent = `${consumed} / ${count}`;
    if (scrubber) {
      scrubber.value = String(Math.round(amount * 1000));
      scrubber.disabled = reducedMotion.matches;
      scrubber.setAttribute('aria-valuetext', `${consumed} of ${count} letters drawn into the impeller`);
    }
  }

  function scrollRange() {
    return Math.max(1, document.documentElement.scrollHeight - window.innerHeight);
  }

  function tick(time) {
    frame = 0;
    const elapsed = previousTime ? Math.min(64, time - previousTime) : 16;
    previousTime = time;
    progress += (target - progress) * (1 - Math.exp(-elapsed / 75));
    if (Math.abs(target - progress) < .00005) progress = target;
    render(progress);
    if (progress !== target) frame = requestAnimationFrame(tick);
    else previousTime = 0;
  }

  function onScroll() {
    target = reducedMotion.matches ? 0 : clamp(window.scrollY / scrollRange());
    if (!frame) frame = requestAnimationFrame(tick);
  }

  function refresh() {
    cancelAnimationFrame(frame);
    frame = 0;
    previousTime = 0;
    measure();
    target = reducedMotion.matches ? 0 : clamp(window.scrollY / scrollRange());
    progress = target;
    render(progress);
  }

  scrubber?.addEventListener('input', () => {
    window.scrollTo(0, Number(scrubber.value) / 1000 * scrollRange());
    onScroll();
  });
  window.addEventListener('scroll', onScroll, { passive: true });
  window.addEventListener('resize', refresh, { passive: true });
  window.addEventListener('pageshow', refresh);
  reducedMotion.addEventListener('change', refresh);
  document.fonts.ready.then(refresh);
  refresh();
})();
