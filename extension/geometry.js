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
export function subtitleHeight(region, monitor, naturalHeight) {
    // Match the 16px gap used by placeSubtitle and the 8px monitor inset.
    const above = region.y - monitor.y - 24;
    const below = monitor.y + monitor.height - region.y - region.height - 24;
    return Math.max(0, Math.min(Math.ceil(naturalHeight), Math.floor(Math.max(above, below))));
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

export function placeWindowSubtitle(monitor, width, naturalHeight, preferred = null) {
    // A window stream does not contain this Shell actor. Place independently of
    // its crop: the portal provides no on-screen position for window streams.
    const height = Math.min(Math.ceil(naturalHeight), monitor.height - 48);
    width = Math.min(width, monitor.width - 32);
    if (height <= 0 || width <= 0) return null;
    const left = monitor.x + 16;
    const top = monitor.y + 24;
    const right = monitor.x + monitor.width - width - 16;
    const bottom = monitor.y + monitor.height - height - 24;
    return {x: Math.max(left, Math.min(right, preferred?.x ?? monitor.x + (monitor.width - width) / 2)),
        y: Math.max(top, Math.min(bottom, preferred?.y ?? bottom)), width, height};
}
