// Development-only tests of the exact WebDriver script with synthetic DOM inputs.
// No browser, account, cookie, network, or third-party JavaScript dependency.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import vm from 'node:vm';

const rust = readFileSync(new URL('../src/browser.rs', import.meta.url), 'utf8');
const body = rust.match(/const CHART_SCRIPT: &str = r#"([\s\S]*?)"#;/)?.[1];
assert.ok(body, 'production chart script must be found');
const script = new vm.Script(`(function () { ${body} }).apply(null, args)`);

function probe({ origin = 'https://create.roblox.com', pathname = '/dashboard/creations/experiences/77/analytics/retention', search = '', expected, frames = [], dialogs = [] } = {}) {
  const context = vm.createContext({
    args: ['Day 1 retention', 'probe', expected],
    location: { origin, pathname, search },
    URL,
    URLSearchParams,
    document: {
      querySelectorAll(selector) {
        if (selector === 'iframe') return frames;
        if (selector === '[role="dialog"]') return dialogs;
        return [];
      },
    },
    getComputedStyle: element => ({ visibility: element.visibility ?? 'visible' }),
  });
  return JSON.parse(JSON.stringify(script.runInContext(context, { timeout: 1000 })));
}

test('login navigation has a distinct signal rather than claiming a proven challenge', () => {
  const result = probe({ origin: 'https://www.roblox.com', pathname: '/login' });
  assert.equal(result.state, 'auth_required');
  assert.equal(result.signal, 'login_route');
  assert.equal(result.originCategory, 'roblox_web');
  assert.equal(result.routeCategory, 'login');
});

test('challenge route is classified without solving it', () => {
  const result = probe({ pathname: '/challenge' });
  assert.equal(result.state, 'auth_required');
  assert.equal(result.signal, 'challenge_route');
  assert.equal(result.originCategory, 'creator_dashboard');
  assert.equal(result.routeCategory, 'challenge');
});

test('unexpected origins are not reported as a confirmed login or challenge', () => {
  const result = probe({ origin: 'https://unexpected.invalid', pathname: '/private-synthetic-value' });
  assert.equal(result.state, 'auth_required');
  assert.equal(result.signal, 'unexpected_origin');
  assert.equal(result.originCategory, 'other');
  assert.equal(result.routeCategory, 'other');
  assert.ok(!JSON.stringify(result).includes('private-synthetic-value'));
  assert.ok(!JSON.stringify(result).includes('unexpected.invalid'));
});

function visibleElement(fields = {}) {
  return { getBoundingClientRect: () => ({ width: 20, height: 20 }), ...fields };
}

test('frame heuristic is distinguished from a login route and no frame URL is exposed', () => {
  const result = probe({ frames: [visibleElement({ src: 'https://captcha.invalid/?token=private-synthetic-value' })] });
  assert.equal(result.state, 'auth_required');
  assert.equal(result.signal, 'challenge_frame');
  assert.equal(result.originCategory, 'creator_dashboard');
  assert.equal(result.routeCategory, 'experience_analytics');
  assert.ok(!JSON.stringify(result).includes('private-synthetic-value'));
});

test('verification dialog heuristic is distinguished and no dialog text is exposed', () => {
  const result = probe({ dialogs: [visibleElement({ textContent: 'Security verification: private-synthetic-value' })] });
  assert.equal(result.state, 'auth_required');
  assert.equal(result.signal, 'verification_dialog');
  assert.ok(!JSON.stringify(result).includes('private-synthetic-value'));
});

test('no signal does not invent authentication success or click anything', () => {
  assert.deepEqual(probe(), { state: 'waiting' });
});

test('non-visible challenge iframe does not produce a visible challenge signal', () => {
  const result = probe({ frames: [visibleElement({ src: 'https://captcha.invalid/', visibility: 'hidden' })] });
  assert.deepEqual(result, { state: 'waiting' });
});

test('another universe after manual login is refused before reading charts', () => {
  const result = probe({ expected: 'https://create.roblox.com/dashboard/creations/experiences/88/analytics/retention' });
  assert.equal(result.state, 'wrong_target');
});

test('another group or duplicate group after manual login is refused', () => {
  const expected = 'https://create.roblox.com/dashboard/creations/experiences/77/analytics/retention?groupId=42';
  assert.equal(probe({ expected, search: '?groupId=43' }).state, 'wrong_target');
  assert.equal(probe({ expected, search: '?groupId=42&groupId=43' }).state, 'wrong_target');
  assert.deepEqual(probe({ expected, search: '?groupId=42' }), { state: 'waiting' });
});

test('canonical benchmark URL may omit the group hint while keeping the same universe', () => {
  const expected = 'https://create.roblox.com/dashboard/creations/experiences/77/analytics/retention?groupId=42';
  assert.deepEqual(probe({ expected, search: '?annotation=Benchmark' }), { state: 'waiting' });
});
