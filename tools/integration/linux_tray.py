#!/usr/bin/env python3
"""Exercise real Linux StatusNotifier menus, GTK lifecycle and idle resources.

Requires dedicated X11 test desktops, python3-gi, xdotool, and a running agent.
Uses the same private node config as linux_hosts.py. Restores direction switches.
Menu activation below uses the panel's actual D-Bus protocol; manual pointer
checks are recorded separately because panel placement is desktop specific.
"""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import time
from linux_hosts import Node

DBUS = '''
import hashlib,json,sys
from gi.repository import Gio,GLib
bus=Gio.bus_get_sync(Gio.BusType.SESSION,None)
name='org.shuttli.Tray.profile_'+hashlib.sha256((sys.argv[1]+'/control.sock').encode()).hexdigest()[:16]
def call(path,interface,method,signature,args,dest=None):
 return bus.call_sync(dest or name,path,interface,method,GLib.Variant(signature,args),None,Gio.DBusCallFlags.NONE,3000,None).unpack()
def prop(key):return call('/StatusNotifierItem','org.freedesktop.DBus.Properties','Get','(ss)',('org.kde.StatusNotifierItem',key))[0]
'''
PROCESSES = '''
import os,json
from pathlib import Path
rows=[]
for p in Path('/proc').iterdir():
 try:
  if not p.name.isdigit() or p.stat().st_uid!=os.getuid():continue
  args=(p/'cmdline').read_bytes().split(b'\\0')
  kind=''
  if args[1:2]==[b'daemon'] and args[0].endswith(b'/shuttli'):kind='daemon'
  elif len(args)>1 and args[1].endswith(b'/shuttli-tray.py'):kind='tray'
  elif len(args)>1 and args[1].endswith(b'/shuttli-ui.py'):kind='window'
  if not kind:continue
  status=dict(line.split(':',1) for line in (p/'status').read_text().splitlines())
  ticks=(p/'stat').read_text().split()
  rows.append(dict(kind=kind,pid=int(p.name),rss_kib=int(status['VmRSS'].split()[0]),ticks=int(ticks[13])+int(ticks[14]),gtk_loaded='libgtk-4' in (p/'maps').read_text()))
 except (FileNotFoundError,ProcessLookupError):pass
print(json.dumps(rows))
'''


def wait(fn, timeout=12):
    until = time.monotonic() + timeout
    while time.monotonic() < until:
        result = fn()
        if result:
            return result
        time.sleep(.25)
    raise TimeoutError('tray/window state did not converge')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--config', required=True)
    parser.add_argument('--report', required=True)
    parser.add_argument('--allow-clipboard-overwrite', action='store_true')
    args = parser.parse_args()
    report = Path(args.report)
    report.parent.mkdir(parents=True, exist_ok=True)
    results = []
    specs = json.loads(Path(args.config).read_text())['nodes']
    for index, spec in enumerate(specs):
        n = Node(spec)
        original = n.api('settings')['settings']
        def rpc(code):
            return json.loads(n.run('/usr/bin/python3', '-c', DBUS + code, n.data))
        def processes():
            return json.loads(n.run('/usr/bin/python3', '-c', PROCESSES))
        def event(ident):
            rpc(f"call('/Menu','com.canonical.dbusmenu','Event','(isvu)',({ident},'clicked',GLib.Variant('i',0),0));print('null')")
        def props():
            return rpc("layout=call('/Menu','com.canonical.dbusmenu','GetLayout','(iias)',(0,-1,[]));print(json.dumps({str(row[0]):row[1] for row in layout[1][2]}))")
        def record(name, fn):
            start = time.monotonic()
            try:
                detail = fn()
                results.append(dict(node=n.label, test=name, status='PASS', seconds=round(time.monotonic()-start, 3), detail=detail))
                print('PASS', n.label, name, flush=True)
            except Exception as exc:
                results.append(dict(node=n.label, test=name, status='FAIL', detail=str(exc)))
                raise
            finally:
                report.write_text(json.dumps(results, indent=2)+'\n')
        def registered():
            value = rpc("items=call('/StatusNotifierWatcher','org.freedesktop.DBus.Properties','Get','(ss)',('org.kde.StatusNotifierWatcher','RegisteredStatusNotifierItems'),'org.kde.StatusNotifierWatcher')[0];print(json.dumps(sum(x.startswith(name+'/') for x in items)))")
            assert value == 1
            assert rpc("print(json.dumps([prop('ItemIsMenu'),prop('Menu'),prop('Status')]))") == [True, '/Menu', 'Active']
            return 'one registered item; panel renders native popup'
        def directions():
            hashes = []
            for send, receive in [(True, True), (True, False), (False, True), (False, False)]:
                n.api('request', json.dumps({'version': 2, 'action': {'command': 'set_directions', 'send': send, 'receive': receive}}))
                wait(lambda: (props()['20']['toggle-state'], props()['21']['toggle-state']) == (int(send), int(receive)))
                layout = props()
                assert layout['30']['enabled'] == send
                assert rpc("print(json.dumps(prop('Status')))") == 'Active'
                hashes.append(rpc("print(json.dumps(hashlib.sha256(bytes(prop('IconPixmap')[1][2])).hexdigest()))"))
            assert len(set(hashes)) == 4
            return 'all four icon pixmaps distinct; menu agrees; both-off stays visible'
        def menu_changes():
            before = n.api('settings')['settings']
            event(20)
            wait(lambda: n.api('settings')['settings']['send'])
            wait(lambda: props()['20']['toggle-state'] == 1)
            event(21)
            wait(lambda: n.api('settings')['settings']['receive'])
            after = n.api('settings')['settings']
            assert {k:v for k,v in before.items() if k not in ('send','receive')} == {k:v for k,v in after.items() if k not in ('send','receive')}
            return 'menu changes persist via public API; unrelated preferences preserved'
        def window_lifecycle():
            event(10)
            def windows():
                result = subprocess.run(n.command('xdotool','search','--onlyvisible','--name','^Shuttli$'), capture_output=True, timeout=10)
                if result.returncode not in (0, 1):
                    raise RuntimeError('window enumeration failed')
                return result.stdout.decode().split()
            ids = wait(windows)
            assert len(ids) == 1
            for _ in range(3):
                time.sleep(.6)
                event(10)
            assert windows() == ids
            assert len([p for p in processes() if p['kind']=='window']) == 1
            # Real WM close, not process termination.
            n.run('xdotool','key','Escape')
            n.run('xdotool','windowactivate','--sync',ids[0],'key','--clearmodifiers','alt+F4')
            wait(lambda: not any(p['kind']=='window' for p in processes()))
            assert n.api('status')['type'] == 'status'
            registered()
            return 'repeated open reuses one GTK window; WM close frees window and retains agent/tray'
        def send_from_menu():
            assert len(specs) == 2, 'menu transfer requires exactly two dedicated nodes'
            target = Node(specs[1-index])
            assert target.api('settings')['settings']['receive'], 'target receiving must be enabled'
            n.api('set','automatic','off')
            previous = n.latest()
            digest = n.fixture()
            event(30)
            rows = n.settled(previous)
            assert len(rows) == 1
            assert target.inspect() == digest
            return 'menu Send clipboard now: terminal applied receipt and matching independent remote OS digest'
        def idle():
            before = processes()
            assert sorted(p['kind'] for p in before) == ['daemon','tray']
            assert not any(p['gtk_loaded'] for p in before)
            time.sleep(10)
            after = processes()
            assert [p['pid'] for p in before] == [p['pid'] for p in after]
            hz = int(n.run('getconf','CLK_TCK'))
            return {'sample_seconds': 10, 'processes': [{**{k:v for k,v in p.items() if k not in ('pid','ticks')}, 'cpu_percent_one_core': round((p['ticks']-old['ticks'])/hz/10*100,2)} for old,p in zip(before,after)]}
        try:
            record('registered-on-desktop-panel',registered)
            record('four-direction-icons-and-menu',directions)
            record('menu-direction-actions',menu_changes)
            if args.allow_clipboard_overwrite:
                record('menu-send-remote-os-readback',send_from_menu)
            record('single-window-close-to-tray',window_lifecycle)
            record('idle-processes-no-gtk',idle)
        finally:
            for child in n.children:
                child.terminate()
                child.wait(timeout=5)
            if args.allow_clipboard_overwrite:
                n.api('set','automatic','on' if original['automatic'] else 'off')
            n.api('request',json.dumps({'version':2,'action':{'command':'set_directions','send':original['send'],'receive':original['receive']}}))


if __name__ == '__main__':
    main()
