import json,urllib.request,sys
B='https://api.bencher.dev/v0/projects/minigraf'
def get(p):
    return json.load(urllib.request.urlopen(urllib.request.Request(B+p,headers={'User-Agent':'ac8'})))
out={}
for lane,tb in (('jvm','binding-jvm-ubuntu24-v1'),('wasi','binding-wasi-ubuntu24-v1')):
    ths=[]
    page=1
    while True:
        d=get(f'/thresholds?branch=v3&testbed={tb}&per_page=100&page={page}')
        ths+=d
        if len(d)<100: break
        page+=1
    rows=[]
    for t in ths:
        m=t.get('model') or {}
        rows.append((t['measure']['name'],m.get('test'),m.get('upper_boundary'),m.get('max_sample_size'),t['testbed']['name'],t['branch']['name']))
    print(lane,tb,len(ths)); 
    for r in set(rows): print(' ',r)
    out[lane]=ths
json.dump(out,open('/tmp/ac8-thresholds.json','w'))
