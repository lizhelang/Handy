"""最终锁定验证；只读取已冻结的公开数据与现有本地模型，不改产品。"""
import hashlib,http.client,json,random,re,resource,socket,subprocess,sys,threading,time
from pathlib import Path
import argparse
HERE=Path(__file__).resolve().parent
parser=argparse.ArgumentParser(description="只使用已有本地资源复跑冻结协议；不下载模型，不覆盖原结果。")
parser.add_argument('--model',type=Path,required=True)
parser.add_argument('--server',type=Path,required=True)
parser.add_argument('--output',type=Path,required=True)
args=parser.parse_args()
MODEL=args.model.resolve();SERVER=args.server.resolve()
assert MODEL.is_file() and SERVER.is_file(), '需要已有模型与 llama-server'
OUT=args.output.resolve();OUT.mkdir(parents=True,exist_ok=False)
config=json.loads((HERE/'protocol.json').read_text())
DATA=HERE/'heldout.json';source=DATA.read_bytes();data=json.loads(source)
digest=hashlib.sha256(source).hexdigest()
assert digest==config['heldout_sha256'], '冻结题目哈希不符'
assert hashlib.sha256((HERE/'protocol.json').read_bytes()).hexdigest()==(HERE/'protocol.sha256').read_text().strip(), '冻结协议哈希不符'
cases=data['cases'];challenges=data['challenge']
assert len(cases)==20 and len(challenges)==4
profiles=config['profiles'];sampling=config['sampling']
(OUT/'heldout.json').write_bytes(source)
(OUT/'heldout.sha256').write_text(digest+'\n')
for name in ['protocol.json','protocol.sha256']:
 (OUT/name).write_bytes((HERE/name).read_bytes())
with socket.socket() as s:s.bind(('127.0.0.1',0));port=s.getsockname()[1]
cmd=[str(SERVER),'-m',str(MODEL),'--host','127.0.0.1','--port',str(port),*config['server_args']]
log=(OUT/'server.log').open('w');p=subprocess.Popen(cmd,stdout=log,stderr=subprocess.STDOUT)
watch=threading.Timer(120,p.kill);watch.start();start=time.perf_counter();meta={'command':cmd,'pid':p.pid,'port':port,'heldout_sha256':digest,'cold_definition':'new process with --no-warmup; OS cache not purged'}

def completion(context,profile):
 cfg=profiles[profile];payload={**sampling,'prompt':cfg['template'].format(context=context),'grammar':cfg['grammar'],'stop':cfg['stop'],'stream':True,'cache_prompt':False}
 c=http.client.HTTPConnection('127.0.0.1',port,timeout=5);t=time.perf_counter();expired=threading.Event();timer=None
 row={'context':context,'profile':profile,'output':'','events':[],'first_content_ms':None,'error':None}
 try:
  c.request('POST','/completion',json.dumps(payload).encode(),{'Content-Type':'application/json'})
  wire=c.sock
  def timeout():
   expired.set()
   try:wire.shutdown(socket.SHUT_RDWR)
   except(OSError,AttributeError):pass
   c.close()
  timer=threading.Timer(max(.001,5-(time.perf_counter()-t)),timeout);timer.start()
  r=c.getresponse()
  if r.status!=200:raise RuntimeError((r.status,r.read().decode()))
  while line:=r.readline():
   if not line.startswith(b'data: '):continue
   d=json.loads(line[6:]);at=(time.perf_counter()-t)*1000
   if d.get('content'):
    if row['first_content_ms'] is None:row['first_content_ms']=at
    row['output']+=d['content'];row['events'].append({'at_ms':at,'content':d['content']})
   if d.get('stop'):row.update({k:d.get(k) for k in ['stop_type','tokens_predicted','tokens_evaluated','truncated','stopping_word','timings']})
  if expired.is_set():raise TimeoutError('total_request_deadline')
 except Exception as e:row['error']='total_request_deadline' if expired.is_set() else str(e)
 finally:
  if timer:timer.cancel()
  c.close()
 row['complete_candidate_ms']=(time.perf_counter()-t)*1000
 row['format_valid']=bool(re.fullmatch('[一-鿿]{1,12}',row['output']))
 row['nonempty']=bool(row['output'])
 return row
try:
 while time.perf_counter()-start<15:
  if p.poll() is not None:raise RuntimeError('runtime exited')
  try:
   c=http.client.HTTPConnection('127.0.0.1',port,timeout=.2);c.request('GET','/health');r=c.getresponse();r.read();c.close()
   if r.status==200:break
  except(OSError,TimeoutError):pass
  time.sleep(.02)
 else:raise RuntimeError('readiness deadline')
 meta['ready_ms']=(time.perf_counter()-start)*1000
 # 不属于评测集的固定预热；只原开发句一条，每个方案一次，不用于调参。
 warm=[dict(completion('明天记得给我',cfg),phase='warmup') for cfg in profiles]
 (OUT/'warmup.json').write_text(json.dumps(warm,ensure_ascii=False,indent=2)+'\n')
 with (OUT/'results.jsonl').open('w') as f:
  for phase,items in [('heldout',cases),('challenge',challenges)]:
   for index,item in enumerate(items):
    for cfg in (['A','B'] if index%2==0 else ['B','A']):
     row=dict(completion(item['context'],cfg),phase=phase,id=item['id'],category=item.get('category'))
     f.write(json.dumps(row,ensure_ascii=False)+'\n');f.flush()
     print(json.dumps({k:row[k] for k in ['phase','id','profile','context','output','complete_candidate_ms','error']},ensure_ascii=False),flush=True)
 meta['rss_kb']=int(subprocess.check_output(['ps','-o','rss=','-p',str(p.pid)],text=True).strip())
finally:
 p.terminate()
 try:p.wait(timeout=5)
 except subprocess.TimeoutExpired:p.kill();p.wait(timeout=3)
 watch.cancel();log.close();meta['exit_code']=p.returncode;meta['child_peak_rss_bytes_macos']=resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss
 (OUT/'runtime-results.json').write_text(json.dumps(meta,ensure_ascii=False,indent=2)+'\n')
rows=[json.loads(line) for line in (OUT/'results.jsonl').read_text().splitlines()]
random.Random(4017).shuffle(rows)
blind=[];mapping=[]
for i,r in enumerate(rows,1):
 key=f'blind-{i:02}'
 blind.append({'blind_id':key,'context':r['context'],'output':r['output']})
 mapping.append({'blind_id':key,'phase':r['phase'],'id':r['id'],'profile':r['profile']})
(OUT/'blind-review.json').write_text(json.dumps({'rubric':config['rubric'],'items':blind},ensure_ascii=False,indent=2)+'\n')
(OUT/'blind-mapping.json').write_text(json.dumps(mapping,ensure_ascii=False,indent=2)+'\n')
print('OWNED_SERVER_EXIT='+str(p.returncode),flush=True)
