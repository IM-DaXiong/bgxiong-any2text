"""Verify the real Any2Text Bridge using a caller-owned registry and local media."""

import argparse
import contextlib
import hashlib
import http.client
import json
import os
import pathlib
import secrets
import subprocess
import threading
import time
import urllib.parse
import uuid


ROOT = pathlib.Path(__file__).resolve().parents[1]


class Bridge:
    def __init__(self, registry, key=None, schema=None):
        self.registry = registry
        self.key = key or secrets.token_hex(32)
        self.schema = schema or ROOT / "contracts/v1/service-provider.schema.json"
        self.connect()

    def connect(self):
        entry = json.loads(self.registry.read_text(encoding="utf-8-sig"))
        address = urllib.parse.urlsplit(entry["baseUrl"])
        assert address.scheme == "http" and address.hostname == "127.0.0.1" and address.path == "/v1/"
        assert "any2textV1" in entry["transportFeatures"]
        self.port = address.port
        self.token = entry["installToken"]
        session = self.request("POST", "session", {"clientKey": self.key})
        assert session["endpointInstance"] == entry["instanceId"]
        self.token = session["sessionToken"]
        self.capabilities = self.request("GET", "capabilities")
        assert self.capabilities["schemaSha256"] == sha256(self.schema)
        assert self.capabilities["endpointInstance"] == entry["instanceId"]

    def request(self, method, path, payload=None, *, status=200, raw=False):
        headers = {"Authorization": "Bearer " + self.token}
        if payload is not None:
            headers["Content-Type"] = "application/json"
            payload = json.dumps(payload, ensure_ascii=False, separators=(",", ":")).encode()
        with contextlib.closing(http.client.HTTPConnection("127.0.0.1", self.port, timeout=15)) as conn:
            conn.request(method, "/v1/any2text/" + path, payload, headers)
            response = conn.getresponse()
            body = response.read()
            if response.status != status:
                raise AssertionError(f"{method} {path}: expected {status}, got {response.status}: {body[:250]!r}")
        return body if raw else json.loads(body) if body else None

    def upload(self, media, status=200):
        fingerprint = sha256(media)
        with media.open("rb") as stream, contextlib.closing(http.client.HTTPConnection("127.0.0.1", self.port, timeout=120)) as conn:
            conn.request("PUT", f"/v1/any2text/inputs/{fingerprint}", stream, {
                "Authorization": "Bearer " + self.token,
                "Content-Length": str(media.stat().st_size),
            })
            response = conn.getresponse()
            raw = response.read()
            assert response.status == status, raw[:250]
        result = json.loads(raw)
        if status == 200:
            assert result["fingerprint"] == fingerprint
        return result

    def control(self, job, action):
        return self.request("POST", f"jobs/{job['taskId']}/control", {"attemptId": job["attemptId"], "action": action})

    def wait(self, job, predicate, deadline, trace=None):
        start = time.monotonic()
        while time.monotonic() < deadline:
            job = self.request("GET", f"jobs/{job['taskId']}")
            if trace is not None:
                trace.append({"seconds": round(time.monotonic() - start, 3), "status": job["status"], "committedMs": job["committedMs"]})
            if predicate(job):
                return job
            if job["status"] in ("failed", "cancelled", "resultReady"):
                raise AssertionError(f"Unexpected terminal status: {job}")
            time.sleep(0.15)
        raise TimeoutError(f"Job timed out: {job}")


def sha256(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def prepare_request(source, model):
    return {"clientRequestId": str(uuid.uuid4()), "operation": {
        "kind": "transcribe", "inputRef": source["inputRef"], "inputFingerprint": source["fingerprint"],
        "modelConfigId": model, "language": "zh", "timing": "segmentTimed", "audioTrack": source["audioTracks"][0]["index"],
    }}


def verify_result(bridge, job, source, output, *, expect_speech=True):
    path = f"jobs/{job['taskId']}/result"
    data = bridge.request("GET", path, raw=True)
    receipt = job["result"]
    assert len(data) == receipt["bytes"] and hashlib.sha256(data).hexdigest() == receipt["sha256"]
    output.write_bytes(data)
    transcript = json.loads(data)
    assert transcript["schemaVersion"] == 1 and transcript["sourceFingerprint"] == source["fingerprint"]
    assert transcript["durationMs"] == source["durationMs"] and transcript["timeOrigin"] == "sourceMedia"
    segments = transcript["segments"]
    assert bool(segments) == expect_speech
    assert len({s["id"] for s in segments}) == len(segments)
    assert all(0 <= s["startMs"] < s["endMs"] <= source["durationMs"] and s["text"].strip() for s in segments)
    assert [s["startMs"] for s in segments] == sorted(s["startMs"] for s in segments)
    bridge.request("POST", path, {"sha256": receipt["sha256"]}, status=204)
    return transcript


def ffmpeg(*args):
    subprocess.run(["ffmpeg", "-nostdin", "-v", "error", "-n", *map(str, args)], check=True)


@contextlib.contextmanager
def record_resources(registry, host_data, output):
    if host_data is None:
        yield
        return
    import psutil

    stop = threading.Event()

    def sample():
        with output.open("w", encoding="utf-8") as log:
            while not stop.is_set():
                try:
                    lock = json.loads((registry.parent / "registry.lock").read_text(encoding="utf-8-sig"))
                    host = psutil.Process(lock["pid"])
                    processes = [host, *host.children(recursive=True)]
                    memory = [{"name": p.name(), "privateBytes": p.memory_info().private, "workingSetBytes": p.memory_info().rss} for p in processes if p.is_running()]
                    gpu = subprocess.check_output(["nvidia-smi", "--query-gpu=memory.used", "--format=csv,noheader,nounits"],
                                                  text=True, timeout=5, creationflags=subprocess.CREATE_NO_WINDOW)
                    size = sum(p.stat().st_size for p in (host_data / "any2text").rglob("*") if p.is_file())
                    row = {"seconds":time.time(),"processes":memory,"totalPrivateBytes":sum(p["privateBytes"] for p in memory),
                           "gpuUsedMiB":int(gpu.splitlines()[0]),"managedBytes":size}
                    print(json.dumps(row), file=log, flush=True)
                except (OSError, ValueError, KeyError, psutil.Error, subprocess.SubprocessError):
                    pass
                stop.wait(2)

    thread = threading.Thread(target=sample, daemon=True)
    thread.start()
    try:
        yield
    finally:
        stop.set()
        thread.join(8)


def run(args):
    args.output.mkdir(parents=True, exist_ok=False)
    bridge = Bridge(args.registry, schema=args.schema)
    model = next(m for m in bridge.capabilities["models"] if m["name"] == args.model and m["installed"] and m["ready"])
    assert "zh" in model["languages"] and "segmentTimed" in model["timings"]
    evidence = {"service": bridge.capabilities, "checks": []}
    evidence_path = args.output / "results.json"

    def record(name, **values):
        evidence["checks"].append({"check": name, **values})
        evidence_path.write_text(json.dumps(evidence, ensure_ascii=False, indent=2), encoding="utf-8")

    source = bridge.upload(args.sample)
    request = prepare_request(source, model["id"])
    with contextlib.closing(http.client.HTTPConnection("127.0.0.1", bridge.port, timeout=10)) as unobserved:
        unobserved.request("POST", "/v1/any2text/jobs", json.dumps(request).encode(), {
            "Authorization": "Bearer " + bridge.token,
            "Content-Type": "application/json",
        })
        deadline = time.monotonic() + 10
        while True:
            try:
                job = bridge.request("GET", f"jobs/by-request/{request['clientRequestId']}")
                break
            except AssertionError as error:
                if "got 404" not in str(error) or time.monotonic() >= deadline:
                    raise
                time.sleep(0.15)
    duplicate = bridge.request("POST", "jobs", request)
    assert duplicate["taskId"] == job["taskId"]
    assert bridge.request("GET", f"jobs/by-request/{request['clientRequestId']}")["taskId"] == job["taskId"]
    bridge.request("POST", "jobs", {**request, "operation": {**request["operation"], "audioTrack": 1000}}, status=409)
    outsider = Bridge(args.registry, schema=args.schema)
    outsider.request("GET", f"jobs/{job['taskId']}", status=404)
    outsider.request("GET", f"jobs/by-request/{request['clientRequestId']}", status=404)
    outsider.request("POST", f"jobs/{job['taskId']}/control", {"attemptId": job["attemptId"], "action": "cancel"}, status=404)
    bridge.connect()
    job = bridge.wait(job, lambda j: j["status"] == "resultReady", time.monotonic() + 90)
    transcript = verify_result(bridge, job, source, args.output / "wav.json")
    record("WAV, owner isolation, renewed session, idempotent submit and lookup", job=job, segments=len(transcript["segments"]))

    if args.formats:
        for extension, flags in [
            ("mp3", ["-i", args.sample, "-c:a", "libmp3lame"]),
            ("mp4", ["-f", "lavfi", "-i", "color=size=160x120:rate=25", "-i", args.sample, "-shortest", "-c:v", "libx264", "-c:a", "aac"]),
        ]:
            sample = args.output / f"speech.{extension}"
            ffmpeg(*flags, sample)
            source = bridge.upload(sample)
            job = bridge.request("POST", "jobs", prepare_request(source, model["id"]))
            job = bridge.wait(job, lambda j: j["status"] == "resultReady", time.monotonic() + 90)
            data = verify_result(bridge, job, source, args.output / f"{extension}.json")
            record(extension.upper(), job=job, segments=len(data["segments"]))

        damaged = args.output / "damaged.wav"
        damaged.write_bytes(b"RIFF\x04\x00\x00\x00WAVE")
        bridge.upload(damaged, status=400)
        no_audio = args.output / "no-audio.mp4"
        ffmpeg("-f", "lavfi", "-i", "color=size=160x120:rate=25", "-t", "2", "-c:v", "libx264", no_audio)
        no_audio_error = bridge.upload(no_audio, status=400)
        assert "音轨" in no_audio_error["message"]
        silent = args.output / "silence.wav"
        ffmpeg("-f", "lavfi", "-i", "anullsrc=r=16000:cl=mono", "-t", "2", "-c:a", "pcm_s16le", silent)
        source = bridge.upload(silent)
        unavailable = bridge.request("POST", "jobs", prepare_request(source, "any2text-nonexistent-model"), status=422)
        assert unavailable["code"] == "modelNotReady"
        silent_job = bridge.request("POST", "jobs", prepare_request(source, model["id"]))
        silent_job = bridge.wait(silent_job, lambda j: j["status"] == "resultReady", time.monotonic() + 90)
        verify_result(bridge, silent_job, source, args.output / "silence.json", expect_speech=False)
        record("damaged media, no audio track, unavailable model and silence distinguished")

        two_tracks = args.output / "two-tracks.mp4"
        ffmpeg("-f", "lavfi", "-i", "color=size=160x120:rate=25", "-f", "lavfi", "-i", "anullsrc=r=16000:cl=mono",
               "-i", args.sample, "-map", "0:v:0", "-map", "1:a:0", "-map", "2:a:0", "-shortest", "-c:v", "libx264", "-c:a", "aac", two_tracks)
        source = bridge.upload(two_tracks)
        assert len(source["audioTracks"]) == 2
        selected = prepare_request(source, model["id"])
        selected["operation"]["audioTrack"] = source["audioTracks"][1]["index"]
        selected_job = bridge.request("POST", "jobs", selected)
        selected_job = bridge.wait(selected_job, lambda j: j["status"] == "resultReady", time.monotonic() + 90)
        verify_result(bridge, selected_job, source, args.output / "selected-track.json")
        record("explicitly selected the second audio track")

    if args.long:
        with record_resources(args.registry, args.host_data, args.output / "resources.ndjson"):
            cell = args.output / "cell.wav"
            fixture = args.output / "fixture7200s.wav"
            ffmpeg("-i", args.sample, "-af", "apad=pad_dur=2", "-t", 10, "-ac", 1, "-ar", 16000, "-c:a", "pcm_s16le", cell)
            ffmpeg("-stream_loop", 719, "-i", cell, "-c:a", "pcm_s16le", fixture)
            assert fixture.stat().st_size > 230_000_000
            deadline = time.monotonic() + 1200
            source = bridge.upload(fixture)
            assert source["durationMs"] == 7_200_000
            job = bridge.request("POST", "jobs", prepare_request(source, model["id"]))
            trace = []
            job = bridge.wait(job, lambda j: j["committedMs"] > 0, min(deadline, time.monotonic() + 60), trace)
            bridge.control(job, "pause")
            paused = bridge.wait(job, lambda j: j["status"] == "paused", min(deadline, time.monotonic() + 30), trace)
            assert paused["committedMs"] >= job["committedMs"]
            time.sleep(0.5)
            assert bridge.request("GET", f"jobs/{job['taskId']}")["committedMs"] == paused["committedMs"]
            resumed = bridge.control(paused, "resume")
            assert resumed["attemptId"] == paused["attemptId"]
            job = bridge.wait(resumed, lambda j: j["committedMs"] > paused["committedMs"], deadline, trace)
            cancel_at = time.monotonic()
            bridge.control(job, "cancel")
            cancelled = bridge.wait(job, lambda j: j["status"] == "cancelled", min(deadline, cancel_at + 5), trace)
            cancel_seconds = time.monotonic() - cancel_at
            resumed = bridge.control(cancelled, "resume")
            assert resumed["attemptId"] != cancelled["attemptId"]
            assert resumed["committedMs"] >= cancelled["committedMs"]
            record("pause/cancel/resume", firstChunkSeconds=next(r["seconds"] for r in trace if r["committedMs"]),
                   cancellationSeconds=round(cancel_seconds, 3), paused=paused, cancelled=cancelled, resumed=resumed)
            if args.restart_command:
                before = bridge.wait(resumed, lambda j: j["committedMs"] > cancelled["committedMs"], deadline, trace)
                old_service = bridge.capabilities["serviceId"]
                old_instance = bridge.capabilities["endpointInstance"]
                checkpoint_dir = args.host_data / "any2text" / "tasks" / before["taskId"] / "checkpoints" if args.host_data else None
                original_checkpoints = {path: path.stat().st_mtime_ns for path in checkpoint_dir.glob("chunk-*.json")} if checkpoint_dir else {}
                restart_env = os.environ.copy()
                if args.checkpoint_gap:
                    restart_env["ANY2TEXT_TEST_CHECKPOINT_GAP"] = before["taskId"]
                subprocess.run(args.restart_command, check=True, env=restart_env)
                bridge.connect()
                assert bridge.capabilities["serviceId"] == old_service
                assert bridge.capabilities["endpointInstance"] != old_instance
                resumed = bridge.request("GET", f"jobs/{before['taskId']}")
                assert resumed["attemptId"] == before["attemptId"]
                if args.checkpoint_gap:
                    resumed = bridge.wait(resumed, lambda j: j["committedMs"] >= before["committedMs"], deadline, trace)
                assert resumed["committedMs"] >= before["committedMs"]
                record("cold restart restored the same attempt", before=before, recovered=resumed,
                       previousInstance=old_instance, newInstance=bridge.capabilities["endpointInstance"],
                       checkpointRegistrationGap=args.checkpoint_gap)
            job = bridge.wait(resumed, lambda j: j["status"] == "resultReady", deadline, trace)
            data = verify_result(bridge, job, source, args.output / "7200s.json")
            assert job["committedMs"] == 7_200_000
            assert data["segments"][-1]["endMs"] >= 7_190_000
            if args.restart_command and args.host_data:
                assert original_checkpoints and all(path.stat().st_mtime_ns == mtime for path, mtime in original_checkpoints.items())
                record("previous ASR chunks reused without writes", count=len(original_checkpoints))
            record("two-hour WAV result", job=job, segments=len(data["segments"]), lastEndMs=data["segments"][-1]["endMs"])
            (args.output / "progress.json").write_text(json.dumps(trace, indent=2), encoding="utf-8")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--registry", type=pathlib.Path, required=True)
    parser.add_argument("--sample", type=pathlib.Path, required=True)
    parser.add_argument("--output", type=pathlib.Path, required=True)
    parser.add_argument("--model", required=True)
    parser.add_argument("--schema", type=pathlib.Path)
    parser.add_argument("--formats", action="store_true")
    parser.add_argument("--long", action="store_true")
    parser.add_argument("--host-data", type=pathlib.Path)
    parser.add_argument("--checkpoint-gap", action="store_true", help="Request an isolated restart hook that rewinds the saved task cursor")
    parser.add_argument("--restart-command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    if args.checkpoint_gap and not (args.long and args.host_data and args.restart_command):
        parser.error("--checkpoint-gap requires --long, --host-data and a fault-injecting --restart-command")
    run(args)
