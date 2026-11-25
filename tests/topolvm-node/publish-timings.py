import datetime
import json
import statistics
import sys
from pathlib import Path

evidence = Path(sys.argv[1])


def parse(timestamp):
    return datetime.datetime.fromisoformat(timestamp.strip().replace("Z", "+00:00"))


def seconds(start, finish):
    return (finish - start).total_seconds()


def sample(run):
    stamps = {}
    for line in (run / "publish.log").read_text().splitlines():
        time, payload = line.split(maxsplit=1)
        event = json.loads(payload)
        key = (event["volume_id"], event["msg"])
        assert key not in stamps, (run, key)
        stamps[key] = parse(time)
    data = {}
    for line in (run / "volume-ids.tsv").read_text().splitlines():
        role, _, handle = line.split("\t")
        start = stamps[handle, "NodePublishVolume called"]
        end = stamps[handle, "NodePublishVolume(fs) succeeded"]
        assert start < end, (run, role)
        data[role] = (start, end)
    assert len(data) == 2 and len(stamps) == 4, run
    starts, ends = zip(*data.values())
    return {
        "case": run.name,
        "publish_window_s": seconds(min(starts), max(ends)),
        "overlap_s": max(0, seconds(max(starts), min(ends))),
        "home_mount_s": seconds(*data["home"]),
        "scratch_mount_s": seconds(*data["scratch"]),
        "request_to_ready_s": seconds(
            parse((run / "request.txt").read_text()), parse((run / "ready.txt").read_text())
        ),
        "volumes_writable_and_reclaimed": True,
    }


samples = {variant: [sample(evidence / f"{variant}-{i}") for i in (1, 2, 3)]
           for variant in ("stock", "patched")}
medians = {variant: {k: statistics.median(s[k] for s in samples[variant])
                     for k in ("publish_window_s", "overlap_s", "request_to_ready_s")}
           for variant in samples}
result = {"samples": samples, "medians": medians, "same_node_disks_and_controller": True}
(evidence / "comparison.json").write_text(json.dumps(result, indent=2) + "\n")
assert all(s["overlap_s"] == 0 for s in samples["stock"]), result
assert sum(s["overlap_s"] > 0 for s in samples["patched"]) >= 2, result
