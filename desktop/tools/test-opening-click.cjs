const test = require('node:test');
const assert = require('node:assert/strict');
const vm = require('node:vm');
const fs = require('node:fs');
const path = require('node:path');

test('the opening release is ignored; fresh pointer and keyboard clicks work', () => {
  const handlers = {};
  vm.runInNewContext(fs.readFileSync(path.join(__dirname, '../crates/tbs-app/src/screens/hosted/opening-click.js'), 'utf8'), {
    window: { addEventListener: (name, handler) => { handlers[name] = handler; } },
  });
  const click = detail => {
    const event = { detail, cancelled: false, stopped: false,
      preventDefault() { this.cancelled = true; },
      stopImmediatePropagation() { this.stopped = true; } };
    handlers.click(event);
    return event;
  };
  assert.equal(click(1).stopped, true);
  handlers.pointerdown();
  assert.equal(click(1).cancelled, false);
  assert.equal(click(0).cancelled, false);
  handlers.pointerdown();
  handlers.pointercancel();
  assert.equal(click(1).cancelled, true);
});
