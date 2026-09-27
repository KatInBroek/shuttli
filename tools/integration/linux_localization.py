#!/usr/bin/env python3
"""Live locale switching and translated notifications on two dedicated desktops.

Changes UI preferences only, restores them on exit, and uses synthetic content.
The existing per-peer send permission and global/manual settings must be ready.
"""
import argparse
import json
from pathlib import Path
import subprocess
import time
from linux_hosts import Node
from linux_tray import DBUS, wait


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--config',required=True)
    p.add_argument('--report',required=True)
    p.add_argument('--allow-clipboard-overwrite',action='store_true',required=True)
    args=p.parse_args()
    nodes=[Node(s) for s in json.loads(Path(args.config).read_text())['nodes']]
    assert len(nodes)==2
    original={}
    policy={}
    catalogs={code:json.loads((Path(__file__).resolve().parents[2]/'crates/native-ui/locales'/f'{code}.json').read_text()) for code in ['en','nl','de','fr']}
    results=[]
    report=Path(args.report);report.parent.mkdir(parents=True,exist_ok=True)
    def rpc(n,code):return json.loads(n.run('/usr/bin/python3','-c',DBUS+code,n.data))
    def select(n,code):
        n.run('/usr/bin/python3','-c',"import sys;sys.path.insert(0,sys.argv[1]);from i18n import Translator;Translator(sys.argv[1]).select(sys.argv[2])",n.data,code)
    def record(name,fn):
        try:
            detail=fn();results.append(dict(test=name,status='PASS',detail=detail));print('PASS',name,flush=True)
        except Exception as error:
            results.append(dict(test=name,status='FAIL',detail=str(error)));raise
        finally:report.write_text(json.dumps(results,ensure_ascii=False,indent=2)+'\n')
    try:
        for n in nodes:
            original[n.label]=n.run('/usr/bin/python3','-c',"import pathlib,sys,json,base64;p=pathlib.Path(sys.argv[1])/'ui-preferences.json';print(json.dumps(base64.b64encode(p.read_bytes()).decode() if p.exists() else None))",n.data).decode()
            policy[n.label]=n.api('status')['status']
            assert policy[n.label]['settings']['send'] and policy[n.label]['settings']['receive']
            assert policy[n.label]['settings']['notifications'] and not policy[n.label]['settings']['automatic']
            rpc(n,"call('/Menu','com.canonical.dbusmenu','Event','(isvu)',(10,'clicked',GLib.Variant('i',0),0));print('null')")
        for code,catalog in catalogs.items():
            def switches():
                for n in nodes:
                    select(n,code)
                    wait(lambda:rpc(n,"print(json.dumps(call('/Menu','com.canonical.dbusmenu','GetProperty','(is)',(10,'label'))[0]))")==catalog['tray.open'])
                    counts = n.api('status')['status']['devices']
                    assert rpc(n,"print(json.dumps(prop('Title')))")==f"Shuttli · ↑ {counts['send']} · ↓ {counts['receive']}"
                    current=n.api('status')['status']
                    assert current['settings']==policy[n.label]['settings']
                    assert current['policy_revision']==policy[n.label]['policy_revision']
                return 'both real tray menus/titles changed; sync preferences and policy revision unchanged'
            record(code+' live language switch',switches)
            def notification():
                monitors=[]
                try:
                    for n in nodes:
                        monitors.append(subprocess.Popen(n.command('timeout','12s','dbus-monitor','--session',"type='method_call',interface='org.freedesktop.Notifications',member='Notify'"),stdout=subprocess.PIPE,stderr=subprocess.PIPE))
                    time.sleep(.8)
                    a,b=nodes
                    before=a.latest();digest=a.fixture();a.api('send');a.settled(before)
                    assert b.inspect()==digest
                    time.sleep(.8)
                    for monitor,key in zip(monitors,['notice.sent','notice.received']):
                        monitor.terminate();out,_=monitor.communicate(timeout=5)
                        assert catalog[key] in out.decode(), 'localized notification title not observed: '+key
                    return 'real desktop notification titles translated; successful remote OS readback'
                finally:
                    for monitor in monitors:
                        if monitor.poll() is None:monitor.terminate();monitor.communicate(timeout=5)
            record(code+' transfer notifications',notification)
    finally:
        for n in nodes:
            for child in n.children:
                child.terminate();child.wait(timeout=5)
            if n.label in original:
                n.run('/usr/bin/python3','-c',"import pathlib,sys,json,base64;p=pathlib.Path(sys.argv[1])/'ui-preferences.json';v=json.loads(sys.argv[2]);p.write_bytes(base64.b64decode(v)) if v is not None else p.unlink(missing_ok=True)",n.data,original[n.label])


if __name__=='__main__':main()
