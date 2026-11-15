// A mouse release opening the native child must not activate its first page.
// Keyboard activation has detail=0 and remains available immediately.
(() => {
  let pressedHere = false;
  window.addEventListener('pointerdown', () => { pressedHere = true; }, true);
  window.addEventListener('click', event => {
    if (event.detail > 0 && !pressedHere) {
      event.preventDefault();
      event.stopImmediatePropagation();
    }
    pressedHere = false;
  }, true);
  window.addEventListener('pointercancel', () => { pressedHere = false; }, true);
})();
