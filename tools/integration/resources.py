#!/usr/bin/env python3
"""Sample explicit release daemon/helper PIDs; does not inspect clipboard data.

Reports RSS (not PSS/physical footprint) and CPU delta as percent of one core.
Remote probe timestamps use the local monotonic midpoint, so SSH round trips
are reported alongside the measurement. Run after transfers, with UI closed.
"""
import argparse
import json
import os
from pathlib import Path
import subprocess
import time

p = argparse.ArgumentParser(description=__doc__)
p.add_argument('--ssh', required=True)
p.add_argument('--linux-pid', type=int, required=True)
p.add_argument('--mac-agent-pid', type=int, required=True)
p.add_argument('--mac-helper-pid', type=int, required=True)
p.add_argument('--seconds', type=int, default=60)
p.add_argument('--report', required=True)
a = p.parse_args()
if a.seconds < 30:
    p.error('at least 30 seconds required for coarse OS CPU counters')

def linux():
    start = time.monotonic()
    raw = Path(f'/proc/{a.linux_pid}/stat').read_text()
    fields = raw[raw.rfind(')')+2:].split()
    result = {'agent': {'cpu': (int(fields[11])+int(fields[12]))/os.sysconf('SC_CLK_TCK'),
                        'rss': int(fields[21])*os.sysconf('SC_PAGE_SIZE')/1024}}
    end = time.monotonic()
    return (start+end)/2, end-start, result

def mac():
    start = time.monotonic()
    output = subprocess.check_output(['ssh', '-oBatchMode=yes', a.ssh,
        f'ps -p {a.mac_agent_pid},{a.mac_helper_pid} -o pid=,time=,rss='], text=True, timeout=20)
    end = time.monotonic()
    result = {}
    for line in output.splitlines():
        pid, cpu, rss = line.split()
        minutes, seconds = cpu.split(':')
        role = 'agent' if int(pid)==a.mac_agent_pid else 'pasteboard helper'
        result[role] = {'cpu': int(minutes)*60+float(seconds), 'rss': int(rss)}
    assert len(result)==2, 'both explicit Mac processes must remain alive'
    return (start+end)/2, end-start, result

before = {'linux': linux(), 'macos': mac()}
time.sleep(a.seconds)
after = {'linux': linux(), 'macos': mac()}
report = {'mode': 'release, Tailscale connected, no generated transfer workload, native UI closed',
          'scope': 'agent and persistent clipboard helper only; transient discovery/notification commands excluded',
          'method': 'RSS endpoint sample; CPU delta / per-platform monotonic probe-midpoint elapsed time; coarse OS counters',
          'platforms': {}}
for system, (start, probe_before, processes) in before.items():
    end, probe_after, final = after[system]
    elapsed = end-start
    items = []
    for role, prior in processes.items():
        current = final[role]
        items.append({'role': role, 'rss_mib': round(current['rss']/1024, 3),
                      'cpu_percent_one_core': round((current['cpu']-prior['cpu'])/elapsed*100, 3)})
    report['platforms'][system] = {'sample_seconds': round(elapsed, 3),
        'probe_duration_seconds': [round(probe_before, 3), round(probe_after, 3)], 'processes': items}
Path(a.report).write_text(json.dumps(report, indent=2)+'\n')
print(json.dumps(report, indent=2))
