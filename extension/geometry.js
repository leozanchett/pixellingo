// Pure geometry, also tested with Node. All coordinates are GNOME logical pixels.
export function regionOnScreen(region, monitor, frameSize) {
    const [width, height] = frameSize;
    return {x: monitor.x + region.x / width * monitor.width,
        y: monitor.y + region.y / height * monitor.height,
        width: region.width / width * monitor.width,
        height: region.height / height * monitor.height};
}
export function overlaps(a, b) {
    return a.x < b.x + b.width && a.x + a.width > b.x
        && a.y < b.y + b.height && a.y + a.height > b.y;
}
export function placeSubtitle(region, monitor, width, height, preferred = null) {
    const safe = {x: monitor.x + 8, y: monitor.y + 8, width: monitor.width - 16, height: monitor.height - 16};
    const clamp = candidate => ({x: Math.max(safe.x, Math.min(safe.x + safe.width - width, candidate.x)),
        y: Math.max(safe.y, Math.min(safe.y + safe.height - height, candidate.y)), width, height});
    const protectedRegion = {x: region.x - 8, y: region.y - 8, width: region.width + 16, height: region.height + 16};
    const center = monitor.x + (monitor.width - width) / 2;
    const candidates = [preferred, {x: center, y: region.y + region.height + 16},
        {x: center, y: region.y - height - 16}].filter(Boolean).map(clamp);
    return candidates.find(candidate => candidate.width <= safe.width && candidate.height <= safe.height
        && !overlaps(candidate, protectedRegion)) ?? null;
}
