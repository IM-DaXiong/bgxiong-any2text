"""Exercise the packaged Any2Text WebView against a real local Bridge.

The source is supplied as a WebView drop event. Exports use fixed destinations
so the harness can read the exact files written by the desktop application.
"""

import argparse
import hashlib
import sqlite3
import uuid
import wave
import json
import os
import pathlib
import subprocess
import time
import urllib.request

from playwright.sync_api import expect, sync_playwright


def start(args):
    flags = f"--remote-debugging-port={args.debug_port}"
    env = dict(os.environ, BGXIONG_ANY2TEXT_DATA_ROOT=str(args.workspace or args.output / "workspace"),
               BGXIONG_BRIDGE_REGISTRY_DIR=str(args.registry_dir),
               WEBVIEW2_USER_DATA_FOLDER=str(args.output / "webview2"), WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=flags)
    startup = subprocess.STARTUPINFO()
    startup.dwFlags |= subprocess.STARTF_USESHOWWINDOW
    startup.wShowWindow = subprocess.SW_HIDE
    return subprocess.Popen([str(args.exe)], env=env, startupinfo=startup)


def attach(playwright, port):
    url = f"http://127.0.0.1:{port}"
    end = time.monotonic() + 30
    while time.monotonic() < end:
        try:
            with urllib.request.urlopen(url + "/json/version", timeout=1) as response:
                endpoint = json.load(response)["webSocketDebuggerUrl"]
            return playwright.chromium.connect_over_cdp(endpoint, timeout=3000)
        except OSError:
            time.sleep(0.15)
    raise TimeoutError("Packaged Any2Text WebView did not start")


def ipc(page, name, data=None):
    return page.evaluate("([name,data]) => window.__TAURI_INTERNALS__.invoke(name,data)", [name, data or {}])


def import_from_drop(page, sample):
    return import_batch_from_drop(page, [sample])[0]


def import_batch_from_drop(page, samples):
    before = {project["id"] for project in ipc(page, "projects")}
    page.evaluate("""() => {
        window.__importPhases = [];
        const observer = new MutationObserver(() => {
            const phase = document.querySelector('.import-progress span')?.textContent;
            if (phase && window.__importPhases.at(-1) !== phase) window.__importPhases.push(phase);
        });
        observer.observe(document.getElementById('root'), { subtree: true, childList: true, characterData: true });
    }""")
    ipc(page, "plugin:event|emit_to", {"target": {"kind": "Webview", "label": "main"},
        "event": "tauri://drag-drop", "payload": {"paths": [str(sample) for sample in samples], "position": {"x": 40, "y": 100}}})
    page.wait_for_function("window.__importPhases.includes('正在导入')", timeout=5000)
    if len(samples) > 1:
        expect(page.get_by_role("progressbar", name="批量导入进度")).to_have_attribute("max", str(len(samples)))
    expect(page.get_by_role("progressbar", name="导入进度")).to_have_count(0, timeout=310000)
    return [project for project in ipc(page, "projects") if project["id"] not in before]


def connect(page):
    if not page.get_by_role("region", name="服务连接").count():
        page.get_by_role("button", name="比格熊 · 未连接").click()
    page.get_by_role("button", name="连接服务", exact=True).click()
    expect(page.get_by_role("button", name="比格熊 · 已连接")).to_be_visible(timeout=20000)
    page.get_by_role("button", name="比格熊 · 已连接").click()


def check_layout(page):
    for width, height in [(1440,900), (1024,768), (736,800), (360,780)]:
        session = page.context.new_cdp_session(page)
        session.send("Emulation.setDeviceMetricsOverride", {"width":width, "height":height, "deviceScaleFactor":1, "mobile":False})
        for section in ("导入素材", "任务队列", "字幕校对", "整理成稿", "导出交付"):
            page.get_by_role("navigation").get_by_role("button", name=section).click()
            assert page.evaluate("document.documentElement.scrollWidth <= innerWidth + 1"), (width, section)
        session.detach()


def wait_result(page, timeout):
    ready = page.get_by_role("button", name="取回并保存")
    failed = page.get_by_text("执行失败", exact=False)
    expect(ready.or_(failed).first).to_be_visible(timeout=timeout)
    if not ready.count():
        raise AssertionError(page.get_by_role("region", name="任务").inner_text())
    ready.first.click()



def seed_subtitle_review(workspace):
    project_id, revision, request, attempt = (str(uuid.uuid4()) for _ in range(4))
    workspace.mkdir(parents=True, exist_ok=False)
    project_root = workspace / "projects" / project_id
    project_root.mkdir(parents=True)
    media = (workspace / "review.wav").resolve()
    sample_rate, seconds = 8000, 90
    with wave.open(str(media), "wb") as audio:
        audio.setparams((1, 2, sample_rate, 0, "NONE", "not compressed"))
        audio.writeframes(bytes(sample_rate * seconds * 2))
    fingerprint = hashlib.sha256(media.read_bytes()).hexdigest()
    segments = [dict(id=f"s{i}", startMs=i * 1000, endMs=(i+1) * 1000, text=f"第{i}段甲。乙。") for i in range(82)]
    transcript = dict(schemaVersion=1, sourceFingerprint=fingerprint, durationMs=seconds * 1000,
        timeOrigin="sourceMedia", timingQuality="segmentTimed", provenance={}, segments=segments,
        waveformPeaks=[0.2] * 40)
    payload = json.dumps(transcript, ensure_ascii=False).encode()
    (project_root / f"{revision}.json").write_bytes(payload)
    legacy_revision = str(uuid.uuid4())
    (project_root / f"{legacy_revision}.json").write_text(json.dumps({"s0":"第0段甲。乙。"}, ensure_ascii=False), encoding="utf-8")
    project = dict(id=project_id, name=media.name, mediaPath=str(media), providerId="bigbear",
        serviceId="test", input=dict(inputRef=fingerprint, fingerprint=fingerprint, durationMs=seconds * 1000,
            audioTracks=[dict(index=0,codec="pcm_s16le")]), original=revision, correction=legacy_revision)
    job = dict(projectId=project_id, providerId="bigbear", serviceId="test",
        request=dict(clientRequestId=request, operation=dict(kind="transcribe")), document=revision,
        remote=dict(taskId="test", attemptId=attempt, clientRequestId=request, status="resultReady", committedMs=seconds*1000,
            durationMs=seconds*1000, result=dict(bytes=len(payload), sha256=hashlib.sha256(payload).hexdigest())))
    with sqlite3.connect(workspace / "workspace.sqlite3") as db:
        db.executescript("CREATE TABLE projects (id TEXT PRIMARY KEY, body TEXT NOT NULL);"
            "CREATE TABLE jobs (id TEXT PRIMARY KEY, project TEXT NOT NULL REFERENCES projects(id), body TEXT NOT NULL);"
            "CREATE TABLE revisions (id TEXT PRIMARY KEY, project TEXT NOT NULL REFERENCES projects(id), kind TEXT NOT NULL, parent TEXT, source TEXT);")
        db.execute("INSERT INTO projects VALUES (?,?)", (project_id, json.dumps(project)))
        db.execute("INSERT INTO jobs VALUES (?,?,?)", (request, project_id, json.dumps(job)))
        db.execute("INSERT INTO revisions VALUES (?,?,?,?,?)", (revision, project_id, "original", None, None))
        db.execute("INSERT INTO revisions VALUES (?,?,?,?,?)", (legacy_revision, project_id, "correction", revision, revision))


def check_subtitle_review(page, args, checks):
    project = ipc(page, "projects")[0]
    before = ipc(page, "jobs")
    original = ipc(page, "document", {"projectId":project["id"], "original":True})
    page.get_by_role("navigation").get_by_role("button", name="字幕校对").click()
    editor = page.get_by_label("当前字幕")
    expect(editor).to_have_text("第0段甲。乙。")
    for term in ("第0段甲", "第81段甲"):
        search = page.get_by_role("searchbox", name="搜索字幕")
        search.fill(term)
        page.locator("[data-subtitle-row] button").click()
        editor.press("Home")
        for _ in range(5):
            editor.press("ArrowRight")
        page.get_by_role("button", name="在光标处拆段").click()
        search.clear()
    page.get_by_role("button", name="保存校订").click()
    expect(page.get_by_text("已保存 · 时间待复核", exact=True)).to_be_visible()
    saved = ipc(page, "document", {"projectId":project["id"], "original":False})
    assert len(saved["reviewGroups"]) == 2, saved["reviewGroups"]
    page.reload(wait_until="domcontentloaded")
    page.get_by_role("navigation").get_by_role("button", name="字幕校对").click()
    expect(page.get_by_role("button", name="复核当前段")).to_be_enabled()
    page.get_by_role("button", name="复核当前段").click()
    dialog = page.get_by_role("dialog")
    page.wait_for_function("document.querySelector('audio')?.readyState >= 2")
    dialog.get_by_role("button", name="循环试听").click()
    page.wait_for_function("!document.querySelector('audio').paused")
    page.wait_for_timeout(1200)
    assert page.locator("audio").evaluate("(audio) => audio.currentTime >= 0 && audio.currentTime <= 1.15")
    dialog.get_by_role("button", name="停止").click()
    assert page.locator("audio").evaluate("(audio) => audio.paused")
    checks.append("bounded local playback loops only within the first review group and stops")
    page.screenshot(path=str(args.output / "review-current.png"))
    dialog.get_by_role("button", name="认可此分界").click()
    dialog.get_by_role("button", name="确认时间").click()
    expect(dialog).to_have_count(0)
    page.get_by_role("button", name="复核全部").click()
    page.screenshot(path=str(args.output / "review-all.png"))
    dialog.get_by_role("button", name="确认时间").click()
    expect(dialog.get_by_role("alert")).to_be_visible()
    dialog.get_by_role("button", name="认可此分界").click()
    dialog.get_by_role("button", name="确认时间").click()
    expect(dialog).to_have_count(0)
    page.get_by_role("button", name="保存校订").click()
    expect(page.get_by_text("已保存", exact=True)).to_be_visible()
    exported = args.output / "review.srt"
    ipc(page, "export_document", {"projectId":project["id"], "original":False, "format":"srt", "path":str(exported), "range":None})
    assert " --> " in exported.read_text(encoding="utf-8")
    assert not ipc(page, "document", {"projectId":project["id"], "original":False})["reviewGroups"]
    assert ipc(page, "document", {"projectId":project["id"], "original":True}) == original
    assert ipc(page, "jobs") == before
    checks.append("structural corrections retained across restart, reviewed across pages and exported without a new job")
    check_layout(page)
    checks.append("all pages fit the four reference widths")
    page.get_by_role("navigation").get_by_role("button", name="任务队列").click()
    for name in ("导入素材", "任务队列", "字幕校对", "整理成稿", "导出交付"):
        expect(page.get_by_role("heading", name=name, exact=True)).to_have_count(0)
    page.get_by_role("button", name="清理任务").click()
    page.get_by_role("dialog").get_by_role("button", name="取消").click()
    assert ipc(page, "jobs") == before
    page.get_by_role("button", name="清理任务").click()
    page.get_by_role("dialog").get_by_role("button", name="确认清理").click()
    expect(page.get_by_role("button", name="清理任务")).to_have_count(0)
    assert ipc(page, "jobs") == []
    workspace = args.workspace or args.output / "workspace"
    assert not list((workspace / "projects" / project["id"]).glob("*.json"))
    assert pathlib.Path(project["mediaPath"]).is_file()
    assert exported.is_file()
    checks.append("queue deletion removes owned revisions, preserves source media and explicit exports, and duplicate headings are gone")


def run(args):
    args.output.mkdir(parents=True, exist_ok=False)
    if args.subtitle_review and not args.workspace:
        seed_subtitle_review(args.output / "workspace")
    proc = start(args)
    checks = []
    errors = []
    try:
        with sync_playwright() as playwright:
            browser = attach(playwright, args.debug_port)
            page = browser.contexts[0].pages[0]
            page.on("pageerror", lambda error: errors.append(str(error)))
            expect(page.get_by_role("navigation").get_by_role("button", name="导入素材")).to_be_visible(timeout=12000)
            if args.subtitle_review:
                check_subtitle_review(page, args, checks)
            elif args.incompatible:
                page.get_by_role("button", name="连接比格熊").click()
                expect(page.get_by_role("alert")).to_contain_text("未提供 Any2Text v1 服务", timeout=10000)
                expect(page.get_by_role("button", name="开始转写")).to_be_disabled()
                checks.append("incompatible Bridge blocked transcribe")
            elif args.offline_existing:
                expect(page.get_by_role("button", name="比格熊 · 未连接")).to_be_visible()
                projects = ipc(page, "projects")
                project = next(project for project in projects if project["original"])
                page.get_by_role("combobox", name="项目").select_option(project["id"])
                page.get_by_role("navigation").get_by_role("button", name="导出交付").click()
                expect(page.locator("pre")).to_contain_text("本地校订验收。")
                original = ipc(page, "document", {"projectId":project["id"], "original":True})
                assert all("本地校订验收" not in segment["text"] for segment in original["segments"])
                exported = args.output / "offline.srt"
                ipc(page, "export_document", {"projectId":project["id"], "original":False, "format":"srt", "path":str(exported), "range":None})
                assert "本地校订验收。" in exported.read_text(encoding="utf-8")
                candidate = next(c for c in ipc(page, "candidates", {"projectId":project["id"]}) if c["accepted"])
                revised = ipc(page, "save_candidate", {"id":candidate["id"], "expectedRevision":candidate["revision"],
                    "blocks":[block["text"] + "（校订）" for block in candidate["draft"]["blocks"]]})
                assert not revised["accepted"]
                try:
                    ipc(page, "preview_candidate", {"id":candidate["id"], "format":"md"})
                    raise AssertionError("An edited draft remained exportable without acceptance")
                except Exception as error:
                    if isinstance(error, AssertionError):
                        raise
                    assert "接受" in str(error)
                accepted = ipc(page, "accept_candidate", {"id":candidate["id"], "expectedRevision":revised["revision"]})
                assert accepted["accepted"]
                checks.append("offline restart: original, correction, export and draft acceptance")
                check_layout(page)
                checks.append("five populated pages fit all four reference widths")
                source = ipc(page, "document", {"projectId": project["id"], "original": False})
                ipc(page, "save_correction", {"projectId": project["id"], "expectedRevision": source["revision"],
                    "changes": {source["segments"][0]["id"]: "来源版本更新验收。"}})
                page.reload(wait_until="domcontentloaded")
                page.get_by_role("combobox", name="项目").select_option(project["id"])
                page.get_by_role("navigation").get_by_role("button", name="整理成稿").click()
                expect(page.get_by_label("候选稿").locator(f'option[value="{accepted["id"]}"]')).to_contain_text("来源待核对")
                try:
                    ipc(page, "preview_candidate", {"id": accepted["id"], "format":"md"})
                except Exception as error:
                    assert "当前来源版本" in str(error)
                else:
                    raise AssertionError("Draft remained exportable after changing its source")
                checks.append("changed source requires renewed draft review")
            else:
                connect(page)
                if args.resume_existing:
                    project = next(project for project in ipc(page, "projects") if pathlib.Path(project["mediaPath"]) == args.sample)
                    job = next(job for job in ipc(page, "jobs") if job["projectId"] == project["id"] and job["request"]["operation"]["kind"] == "transcribe")
                    job = ipc(page, "refresh_job", {"requestId": job["request"]["clientRequestId"]})
                    page.reload(wait_until="domcontentloaded")
                    connect(page)
                    page.get_by_role("combobox", name="项目").select_option(project["id"])
                    page.get_by_role("navigation").get_by_role("button", name="任务队列").click()
                    if job["remote"]["status"] in ("paused", "cancelled", "failed"):
                        resume = page.get_by_role("button", name="继续未完成部分")
                        resume.click()
                        expect(resume).to_be_hidden(timeout=10000)
                else:
                    project = import_from_drop(page, args.sample)
                    page.reload(wait_until="domcontentloaded")
                    connect(page)
                    expect(page.get_by_role("combobox", name="项目")).to_have_value(project["id"])
                    model = page.get_by_label("识别模型")
                    if args.model_id:
                        model.select_option(value=args.model_id)
                    else:
                        model.select_option(label=args.model)
                    page.get_by_role("button", name="开始转写").click()
                wait_result(page, args.transcribe_timeout * 1000)
                expect(page.get_by_role("heading", name="字幕校对")).to_be_visible(timeout=20000)
                expect(page.get_by_label("当前字幕")).not_to_have_text("")
                page.locator("audio").evaluate("audio => { if (audio.readyState < 1) return new Promise((resolve, reject) => { audio.addEventListener('loadedmetadata',resolve,{once:true}); audio.addEventListener('error',reject,{once:true}); setTimeout(() => reject(new Error('audio metadata timeout')),12000); }); }")
                original = ipc(page, "document", {"projectId": project["id"], "original": True})
                if args.long_document:
                    assert len(original["segments"]) > 160
                    expect(page.get_by_role("button", name="下一页")).to_be_enabled()
                    page.get_by_role("button", name="下一页").click()
                    expect(page.locator(".segments button")).to_have_count(80)
                    last = original["segments"][-1]
                    page.get_by_label("搜索字幕").fill(last["text"])
                    expect(page.locator(".segments button").first).to_be_visible()
                    for extension, filename in (("txt", "transcript.txt"), ("srt", "subtitles.srt")):
                        saved = args.output / filename
                        ipc(page, "export_document", {"projectId": project["id"], "original": True,
                            "format": extension, "path": str(saved), "range": None})
                        assert saved.read_text(encoding="utf-8").strip()
                    checks.append("long document paging and text search")
                if args.expect_waveform:
                    assert any(peak > 0 for peak in original["waveformPeaks"])
                    page.get_by_role("button", name="波形细节").click()
                    expect(page.get_by_role("button", name="波形细节")).to_have_attribute("aria-pressed", "true")
                page.get_by_label("搜索字幕").fill(original["segments"][0]["text"])
                page.locator(".segments button").first.click()
                expect(page.get_by_label("当前字幕")).to_have_text(original["segments"][0]["text"])
                page.get_by_label("当前字幕").fill("本地校订验收。")
                expect(page.get_by_role("button", name="保存校订")).to_be_enabled()
                page.get_by_role("button", name="保存校订").click()
                expect(page.get_by_role("status")).to_contain_text("校订已保存")
                revision_list = ipc(page, "revisions", {"projectId": project["id"]})
                job = next(job for job in ipc(page, "jobs") if job["projectId"] == project["id"] and job["document"] == original["revision"])
                ipc(page, "retrieve", {"requestId": job["request"]["clientRequestId"]})
                assert ipc(page, "revisions", {"projectId": project["id"]}) == revision_list
                checks.append("packaged UI transcribed, retrieved, played and revised real media")
                checks.append("duplicate result retrieval preserved revision history")

                if args.organize:
                    for draft_type, mode in [("口播稿", "忠实整理"), ("文学剧本", "创作改编")]:
                        page.get_by_role("navigation").get_by_role("button", name="整理成稿").click()
                        page.get_by_label("类型").select_option(label=draft_type)
                        page.get_by_label("方式").select_option(label=mode)
                        page.get_by_role("button", name="生成候选稿").click()
                        wait_result(page, 240000)
                        expect(page.get_by_role("heading", name="整理成稿")).to_be_visible(timeout=12000)
                        candidates = ipc(page, "candidates", {"projectId": project["id"]})
                        latest = candidates[0]
                        page.get_by_label("候选稿").select_option(latest["id"])
                        if mode == "创作改编":
                            source = ipc(page, "document", {"projectId": project["id"], "original": False})
                            marked = 0
                            for block in latest["draft"]["blocks"]:
                                references = set(block["sourceSegmentIds"])
                                text = "".join(char for char in block["text"] if char.isalnum())
                                quoted = "".join(char for segment in source["segments"] if segment["id"] in references
                                                 for char in segment["text"] if char.isalnum())
                                if not text or text not in quoted:
                                    assert block["addedOrInferred"] and block["text"].lstrip().startswith("【新增/推测】")
                                    marked += 1
                            assert marked > 0, "Creative fixture produced no additions to check"
                            checks.append(f"all {marked} creative blocks with additions carried review markers")
                        page.get_by_role("button", name="接受稿件").click()
                        expect(page.get_by_role("button", name="已接受")).to_be_visible()
                        result = args.output / (latest["draft"]["draftType"] + ".md")
                        ipc(page, "export_candidate", {"id": latest["id"], "format": "md", "path": str(result)})
                        assert result.read_text(encoding="utf-8").strip()
                        txt = result.with_suffix(".txt")
                        ipc(page, "export_candidate", {"id": latest["id"], "format": "txt", "path": str(txt)})
                        assert txt.read_text(encoding="utf-8").strip()
                        checks.append(f"real local text model generated and exported {draft_type}")

                page.get_by_role("button", name="比格熊 · 已连接").click()
                page.get_by_role("button", name="断开连接").click()
                expect(page.get_by_role("button", name="比格熊 · 未连接")).to_be_visible()
                page.get_by_role("navigation").get_by_role("button", name="导出交付").click()
                expect(page.locator("pre")).to_contain_text("本地校订验收。")
                for extension in ("srt", "vtt", "txt", "json"):
                    saved = args.output / f"offline.{extension}"
                    ipc(page, "export_document", {"projectId":project["id"], "original":False, "format":extension, "path":str(saved), "range":None})
                    content = saved.read_text(encoding="utf-8")
                    assert "本地校订验收。" in content
                    if extension == "srt":
                        assert " --> " in content
                    elif extension == "vtt":
                        assert content.startswith("WEBVTT\n") and " --> " in content
                    elif extension == "json":
                        assert json.loads(content)["segments"][0]["text"] == "本地校订验收。"
                again = ipc(page, "document", {"projectId":project["id"], "original":True})
                assert again == original
                checks.append("offline export preserved the original and the correction")

                check_layout(page)
                checks.append("five pages fit all four reference widths")
                page.get_by_role("navigation").get_by_role("button", name="字幕校对").click()
                current = ipc(page, "document", {"projectId": project["id"], "original": False})
                pending = "并发校订冲突后的未保存内容。"
                page.get_by_label("当前字幕").fill(pending)
                ipc(page, "save_correction", {"projectId": project["id"], "expectedRevision": current["revision"],
                    "changes": {current["segments"][0]["id"]: "同一原文已在另一窗口更新。"}})
                page.get_by_role("button", name="保存校订").click()
                expect(page.get_by_role("alert")).to_contain_text("版本")
                expect(page.get_by_label("当前字幕")).to_have_text(pending)
                checks.append("a stale revision left the unsaved editor text intact")
            page.screenshot(path=str(args.output / "desktop.png"))
            if args.subtitle_review:
                page.get_by_role("group", name="窗口控制").locator('button[title="关闭"]').click()
                assert proc.wait(timeout=10) == 0
                checks.append("title-bar close exits the native window")
            assert not errors, errors
    finally:
        if proc.poll() is None:
            proc.terminate()
            proc.wait(timeout=15)
        (args.output / "results.json").write_text(json.dumps({"checks":checks,"pageErrors":errors}, ensure_ascii=False, indent=2), encoding="utf-8")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exe", type=lambda p: pathlib.Path(p).resolve(), required=True)
    parser.add_argument("--registry-dir", type=lambda p: pathlib.Path(p).resolve(), required=True)
    parser.add_argument("--sample", type=lambda p: pathlib.Path(p).resolve())
    parser.add_argument("--output", type=lambda p: pathlib.Path(p).resolve(), required=True)
    parser.add_argument("--workspace", type=lambda p: pathlib.Path(p).resolve())
    parser.add_argument("--debug-port", type=int, required=True)
    parser.add_argument("--model", default="Whisper-large-v3")
    parser.add_argument("--model-id")
    parser.add_argument("--organize", action="store_true")
    parser.add_argument("--expect-waveform", action="store_true")
    parser.add_argument("--long-document", action="store_true")
    parser.add_argument("--transcribe-timeout", type=int, default=100)
    parser.add_argument("--incompatible", action="store_true")
    parser.add_argument("--offline-existing", action="store_true")
    parser.add_argument("--resume-existing", action="store_true")
    parser.add_argument("--subtitle-review", action="store_true")
    run(parser.parse_args())
