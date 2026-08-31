export function createPermanentFailure(onFirst = () => {}) {
  let error = null;
  return {
    get error() { return error; },
    latch(value) {
      if (error) return { error, first: false };
      error = value instanceof Error ? value : new Error(String(value));
      onFirst(error);
      return { error, first: true };
    },
  };
}

export function createRequestGate() {
  let generation = 0;
  return {
    next() {
      generation += 1;
      return generation;
    },
    isCurrent(candidate) {
      return candidate === generation;
    },
    invalidate() {
      generation += 1;
    },
  };
}

export function createLiveBatch(flush, delay = 300, schedule = setTimeout, cancel = clearTimeout) {
  const pending = new Set();
  let timer = null;
  return {
    add(value) {
      pending.add(value);
      if (timer !== null) return;
      timer = schedule(() => {
        timer = null;
        const values = [...pending];
        pending.clear();
        flush(values);
      }, delay);
    },
    clear() {
      if (timer !== null) cancel(timer);
      timer = null;
      pending.clear();
    },
  };
}

export function expireLiveIds(expirations, now) {
  const expired = [];
  for (const [id, expiresAt] of expirations) {
    if (expiresAt > now) continue;
    expirations.delete(id);
    expired.push(id);
  }
  return expired;
}

export function pruneLiveIds(expirations, events) {
  const visible = new Set(events.map(event => event.id));
  const removed = [];
  for (const id of expirations.keys()) {
    if (visible.has(id)) continue;
    expirations.delete(id);
    removed.push(id);
  }
  return removed;
}

export function intersectEventIds(ids, events) {
  const returned = new Set(events.map(event => event.id));
  return [...new Set(ids)].filter(id => returned.has(id));
}
