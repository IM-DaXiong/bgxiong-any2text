"""Focused UI regression against built frontend with simulated Tauri IPC."""
import functools
import http.server
import pathlib
import threading

from playwright.sync_api import sync_playwright, expect

ROOT = pathlib.Path(__file__).resolve().parents[1]
MOCK = """
window.fixture = { online: true, attempts: 0, pending: 0, peak: 0, delay: 0 };
window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => {} };
window.__TAURI_INTERNALS__ = {
  metadata: {currentWindow: {label: 'main'}, currentWebview: {label: 'main'}},
  transformCallback: () => 1,
  unregisterCallback: () => {},
  invoke: async (cmd) => {
    const f = window.fixture;
    if (cmd === 'connect') {
      f.attempts++; f.pending++; f.peak = Math.max(f.peak, f.pending);
      try {
        await new Promise(resolve => setTimeout(resolve, f.delay));
        if (!f.online) throw '比格熊服务未启动';
        return {providerId:'bigbear', serviceId:'test', endpointInstance:'test',
          models:[], operations:[], pause:false, resume:false};
      } finally { f.pending--; }
    }
    if (cmd === 'provider_health' && !f.online) throw '比格熊服务未启动';
    if (cmd === 'provider_appearance') return 'dark';
    if (cmd === 'projects' || cmd === 'jobs') return [];
    if (cmd === 'launch_bigbear') { f.online = true; return true; }
    return null;
  }
};
"""


class QuietHandler(http.server.SimpleHTTPRequestHandler):
    def log_message(self, *_):
        pass


def main():
    server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), functools.partial(
        QuietHandler, directory=str(ROOT / 'ui' / 'dist')))
    threading.Thread(target=server.serve_forever, daemon=True).start()
    try:
        with sync_playwright() as p:
            browser = p.chromium.launch(channel='msedge', headless=True)
            page = browser.new_page(viewport={'width': 1280, 'height': 850})
            errors = []
            page.on('pageerror', lambda error: errors.append(str(error)))
            page.add_init_script(MOCK)
            page.goto(f'http://127.0.0.1:{server.server_port}')
            status = page.locator('.provider-anchor > button')
            expect(status).to_have_text('比格熊 · 已连接')
            assert page.evaluate('fixture.attempts') == 1
            for width in [1280, 768, 360]:
                page.set_viewport_size({'width': width, 'height': 850})
                before = page.locator('main').bounding_box()
                status.click()
                expect(page.get_by_role('region', name='服务连接')).to_be_visible()
                assert page.locator('main').bounding_box() == before
                panel = page.locator('#provider-panel').bounding_box()
                assert panel['x'] >= 0 and panel['x'] + panel['width'] <= width
                page.keyboard.press('Escape')
                expect(status).to_be_focused()
                expect(page.locator('#provider-panel')).to_have_count(0)
            page.set_viewport_size({'width': 1280, 'height': 850})
            status.click()
            page.get_by_role('heading', name='导入素材').click()
            expect(page.locator('#provider-panel')).to_have_count(0)
            status.click()
            page.get_by_role('button', name='断开连接', exact=True).click()
            expect(status).to_have_text('比格熊 · 未连接')
            attempts = page.evaluate('fixture.attempts')
            page.wait_for_timeout(5400)
            assert page.evaluate('fixture.attempts') == attempts
            page.get_by_role('button', name='连接服务', exact=True).click()
            expect(status).to_have_text('比格熊 · 已连接')
            page.evaluate('fixture.online = false')
            expect(status).not_to_have_text('比格熊 · 已连接', timeout=6000)
            page.evaluate('fixture.online = true')
            expect(status).to_have_text('比格熊 · 已连接', timeout=12000)
            page.add_init_script('fixture.online = false; fixture.delay = 200;')
            page.reload()
            expect(status).to_have_text('比格熊 · 未连接')
            status.click()
            expect(page.get_by_role('status')).to_contain_text('比格熊服务未启动')
            page.evaluate('fixture.online = true; fixture.delay = 5500')
            expect(status).to_have_text('比格熊 · 连接中…', timeout=7000)
            expect(page.get_by_role('button', name='连接中…', exact=True)).to_be_disabled()
            expect(status).to_have_text('比格熊 · 已连接', timeout=12000)
            assert page.evaluate('fixture.peak') == 1
            page.evaluate('fixture.delay = 0')
            page.get_by_role('button', name='断开连接', exact=True).click()
            page.get_by_role('button', name='启动比格熊', exact=True).click()
            expect(status).to_have_text('比格熊 · 已连接', timeout=7000)
            assert not errors, errors
            print('PASS: startup, retry, recovery, manual disconnect/reconnect, launch, single flight, overlay at 3 widths, Escape and outside dismissal')
            browser.close()
    finally:
        server.shutdown()


if __name__ == '__main__':
    main()
