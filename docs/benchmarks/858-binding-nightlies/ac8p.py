import json,urllib.request,urllib.parse
B='https://api.bencher.dev/v0/projects/minigraf'
def get(p): return json.load(urllib.request.urlopen(urllib.request.Request(B+p,headers={'User-Agent':'ac8'})))
meas=[m for m in get('/measures') if m['slug']=='latency'][0]['uuid']
br=get('/branches/v3')['uuid']
res={}
for lane,tb in (('jvm','binding-jvm-ubuntu24-v1'),('wasi','binding-wasi-ubuntu24-v1')):
    tbu=get('/testbeds/'+tb)['uuid']
    bms=[];p=1
    while True:
        d=get(f'/benchmarks?search=binding/{lane}&per_page=100&page={p}'); bms+=d
        if len(d)<100: break
        p+=1
    bms=[b for b in bms if b['name'].startswith(f'binding/{lane}/')]
    print(lane,len(bms),'benchmarks')
    for b in sorted(bms,key=lambda b:b['name']):
        q=urllib.parse.urlencode({'branches':br,'testbeds':tbu,'benchmarks':b['uuid'],'measures':meas,'start_time':'1793000000000'})
        d=get('/perf?'+q)
        for r in d['results']:
            pts=r['metrics']
            lim=[x.get('boundary') for x in pts]
            last=pts[-1]
            res[b['name']]=dict(n=len(pts),value=last['metric']['value'],base=(last.get('boundary') or {}).get('baseline'),upper=(last.get('boundary') or {}).get('upper_limit'),reports=sorted({x['report'] for x in pts}))
            print(f"{b['name']:55} n={len(pts)} last={last['metric']['value']:.0f} base={(last.get('boundary') or {}).get('baseline')} upper={(last.get('boundary') or {}).get('upper_limit')}")
json.dump(res,open('/tmp/ac8-perf.json','w'))
