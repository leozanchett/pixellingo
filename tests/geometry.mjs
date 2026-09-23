import assert from 'node:assert/strict';
import {test} from 'node:test';
import {placeSubtitle, overlaps, regionOnScreen} from '../extension/geometry.js';

test('fractional scale and a monitor with negative origin', () => {
    const region = regionOnScreen({x: 200, y: 1000, width: 1200, height: 300},
        {x: -1280, y: 0, width: 1280, height: 720}, [2560, 1440]);
    assert.deepEqual(region, {x: -1180, y: 500, width: 600, height: 150});
});
test('subtitle never overlaps the selected area, even when dragged into it', () => {
    const monitor = {x: 0, y: 0, width: 1920, height: 1080};
    const region = {x: 50, y: 700, width: 1800, height: 300};
    const position = placeSubtitle(region, monitor, 900, 92, {x: 100, y: 800});
    assert.ok(position);
    assert.equal(overlaps(position, region), false);
    assert.ok(position.y >= 0);
    assert.ok(position.x + position.width <= 1920);
});
test('no room means hide instead of reading our own subtitle', () => {
    const monitor = {x: 0, y: 0, width: 1280, height: 720};
    assert.equal(placeSubtitle(monitor, monitor, 900, 92), null);
});
test('valid user placement survives a new translation', () => {
    const monitor = {x: 0, y: 0, width: 1920, height: 1080};
    const region = {x: 50, y: 700, width: 1800, height: 300};
    assert.deepEqual(placeSubtitle(region, monitor, 900, 92, {x: 100, y: 40}),
        {x: 100, y: 40, width: 900, height: 92});
});
