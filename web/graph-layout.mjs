export const MIN_NODE_SPACING = 32;
export const MIN_INTERACTIVE_SCALE = 0.28;

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

export function layoutEvents(events) {
  const ordered = [...events].sort((left, right) => left.sequence - right.sequence);
  const correlations = [...new Set(ordered.map(event => event.correlation_id))];
  const laneCount = Math.max(1, Math.min(correlations.length, 20));
  const correlationLane = new Map(correlations.map((id, index) => [id, index % laneCount]));
  const width = Math.max(1200, (ordered.length - 1) * MIN_NODE_SPACING + 140);
  const height = Math.max(520, laneCount * 64 + 100);
  const positions = new Map();
  ordered.forEach((event, index) => {
    positions.set(event.id, {
      x: ordered.length === 1 ? width / 2 : 70 + index * MIN_NODE_SPACING,
      y: 70 + correlationLane.get(event.correlation_id) * 64 + (stableHash(event.id) % 13 - 6),
    });
  });
  return { ordered, positions, width, height };
}

export function fitTransform(world, viewport, minimumScale = MIN_INTERACTIVE_SCALE) {
  const rawScale = Math.min(viewport.width / world.width, viewport.height / world.height) * 0.9;
  const scale = Math.min(2, Math.max(minimumScale, rawScale));
  const fitsHorizontally = world.width * scale <= viewport.width;
  return {
    scale,
    x: fitsHorizontally ? (viewport.width - world.width * scale) / 2 : viewport.width - world.width * scale - 40,
    y: (viewport.height - world.height * scale) / 2,
  };
}

export function nearestNode(positions, point, maximumDistance) {
  let nearest = null;
  let nearestDistance = maximumDistance;
  for (const [id, position] of positions) {
    const distance = Math.hypot(position.x - point.x, position.y - point.y);
    if (distance < nearestDistance) {
      nearest = id;
      nearestDistance = distance;
    }
  }
  return nearest;
}

export function trimLine(source, target, sourceRadius, targetRadius) {
  const deltaX = target.x - source.x;
  const deltaY = target.y - source.y;
  const distance = Math.hypot(deltaX, deltaY);
  if (!distance) return null;
  const unitX = deltaX / distance;
  const unitY = deltaY / distance;
  const usableDistance = Math.max(0, distance - sourceRadius - targetRadius);
  return {
    source: {
      x: source.x + unitX * sourceRadius,
      y: source.y + unitY * sourceRadius,
    },
    target: {
      x: source.x + unitX * (sourceRadius + usableDistance),
      y: source.y + unitY * (sourceRadius + usableDistance),
    },
  };
}

function stableHash(value) {
  let result = 0;
  for (let index = 0; index < value.length; index += 1) {
    result = ((result << 5) - result + value.charCodeAt(index)) | 0;
  }
  return Math.abs(result);
}
