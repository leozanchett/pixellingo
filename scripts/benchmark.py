#!/usr/bin/env python3
"""Read /proc only. Records CPU/RSS for explicitly supplied processes, no screens."""
import argparse
import csv
import json
import os
from pathlib import Path
import statistics
import time

p = argparse.ArgumentParser(description=__doc__)
p.add_argument('--pid', type=int, nargs='+', required=True, help='Service and (optionally) GNOME Shell PIDs')
p.add_argument('--seconds', type=int, default=900)
p.add_argument('--output', type=Path, required=True)
args = p.parse_args()
ticks = os.sysconf('SC_CLK_TCK')
cores = os.cpu_count() or 1

def read(pid):
    fields = Path(f'/proc/{pid}/stat').read_text().rsplit(')', 1)[1].split()
    cpu = (int(fields[11]) + int(fields[12])) / ticks
    rss = int(fields[21]) * os.sysconf('SC_PAGE_SIZE') / 1024 / 1024
    return cpu, rss

args.output.parent.mkdir(parents=True, exist_ok=True)
rows = []
previous = {pid: read(pid) for pid in args.pid}
start = last = time.monotonic()
with args.output.open('w') as output:
    writer = csv.DictWriter(output, fieldnames=['seconds', 'pid', 'cpu_one_core_pct', 'cpu_machine_pct', 'rss_mib'])
    writer.writeheader()
    while time.monotonic() - start < args.seconds:
        time.sleep(1)
        now = time.monotonic()
        for pid in args.pid:
            try:
                cpu, rss = read(pid)
            except FileNotFoundError:
                raise SystemExit(f'Processo {pid} encerrou; medição incompleta salva em {args.output}')
            percent = (cpu - previous[pid][0]) / (now - last) * 100
            row = dict(seconds=round(now-start, 3), pid=pid, cpu_one_core_pct=round(percent, 3),
                       cpu_machine_pct=round(percent/cores, 3), rss_mib=round(rss, 3))
            writer.writerow(row); rows.append(row)
            previous[pid] = (cpu, rss)
        output.flush(); last = now
summary = {}
for pid in args.pid:
    subset = [row for row in rows if row['pid'] == pid]
    summary[pid] = dict(samples=len(subset), cpu_machine_mean_pct=statistics.mean(row['cpu_machine_pct'] for row in subset),
                        rss_max_mib=max(row['rss_mib'] for row in subset))
args.output.with_suffix('.json').write_text(json.dumps(summary, indent=2) + '\n')
print(json.dumps(summary, indent=2))
