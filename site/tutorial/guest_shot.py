#!/usr/bin/env python3
"""The guest page (remote/guest) as a guest sees it on a phone, for the tutorial (needs Playwright).

    python3 site/tutorial/guest_shot.py OUT_DIR
"""
import functools
import http.server
import os
import sys
import threading
from playwright.sync_api import sync_playwright
page_dir = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..", "remote", "guest")
out = sys.argv[1]
ua = "Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Mobile/15E148 Safari/604.1"
# Served over HTTP like the real page: a browser loads the audio worklet only from a web origin.
class Quiet(http.server.SimpleHTTPRequestHandler):
    def log_message(self, *args):
        pass


handler = functools.partial(Quiet, directory=os.path.abspath(page_dir))
server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), handler)
threading.Thread(target=server.serve_forever, daemon=True).start()
base = f"http://127.0.0.1:{server.server_address[1]}"
with sync_playwright() as p:
    # A stand-in microphone, allowed without asking, as after a guest clicked "Allow".
    b = p.chromium.launch(args=["--use-fake-ui-for-media-stream", "--use-fake-device-for-media-stream"])
    for lang, name in [("de", "Sven"), ("en", "Sam")]:
        ctx = b.new_context(viewport={"width": 390, "height": 760}, device_scale_factor=2, user_agent=ua, locale=lang, is_mobile=True, has_touch=True)
        ctx.grant_permissions(["microphone"])
        pg = ctx.new_page()
        pg.goto(f"{base}/index.html?lang={lang}#talkcraft-demo")
        pg.wait_for_timeout(800)
        pg.fill("#name", name)
        # Until the microphone list has the device (the browser takes a moment).
        pg.wait_for_function("document.querySelectorAll('#mic option').length > 0 && /fake/i.test(document.querySelector('#mic').textContent)", timeout=10000)
        # The stand-in device's technical name, as a guest's headset would be called.
        mic = "Headset-Mikrofon" if lang == "de" else "Headset microphone"
        pg.evaluate("m => document.querySelectorAll('#mic option').forEach(o => { if (/fake/i.test(o.textContent)) o.textContent = m; })", mic)
        pg.wait_for_timeout(200)
        pg.screenshot(path=f"{out}/guest-page-{lang}.png", full_page=False)
        print(f"guest-page-{lang}.png")
    b.close()
server.shutdown()
