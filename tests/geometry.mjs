import assert from 'node:assert/strict';
import {test} from 'node:test';
import {placeSubtitle, placeWindowSubtitle, overlaps, regionOnScreen, subtitleHeight} from '../extension/geometry.js';

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
test('long subtitles expand, shrink and stay outside the capture on scaled monitors', () => {
    const monitor = {x: -1280, y: 20, width: 1280, height: 720};
    const region = {x: -1100, y: 420, width: 900, height: 180};
    assert.equal(subtitleHeight(region, monitor, 56), 56);
    assert.equal(subtitleHeight(region, monitor, 210), 210);
    assert.equal(subtitleHeight(region, monitor, 900), 376);
    const position = placeSubtitle(region, monitor, 900, subtitleHeight(region, monitor, 900));
    assert.ok(position);
    assert.equal(overlaps(position, region), false);
    assert.equal(subtitleHeight(monitor, monitor, 56), 0);
});
test('window captions use output monitor coordinates, grow upwards and stay inside it', () => {
    const monitor = {x: -1920, y: -200, width: 1920, height: 1080};
    const short = placeWindowSubtitle(monitor, 900, 40);
    const long = placeWindowSubtitle(monitor, 900, 200);
    assert.equal(short.y + short.height, 856);
    assert.equal(long.y + long.height, 856);
    assert.ok(long.y < short.y);
    const dragged = placeWindowSubtitle(monitor, 900, 80, {x: 3000, y: -5000});
    assert.equal(dragged.x + dragged.width, -16);
    assert.equal(dragged.y, -176);
    const huge = placeWindowSubtitle(monitor, 5000, 5000);
    assert.equal(huge.width, 1888);
    assert.equal(huge.height, 1032);
});
