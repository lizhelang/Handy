import hashlib,json,math
from pathlib import Path
P=Path(__file__).resolve().parent
rows=[json.loads(s) for s in (P/'results.jsonl').read_text().splitlines()]
mapping=json.loads((P/'blind-mapping.json').read_text())
rating_data=json.loads((P/'blind-ratings.json').read_text())
ratings=rating_data['items'] if isinstance(rating_data,dict) and 'items' in rating_data else rating_data['ratings'] if isinstance(rating_data,dict) and 'ratings' in rating_data else rating_data
if isinstance(ratings,dict): ratings=[{'blind_id':key,**v} for key,v in ratings.items()]
by_blind={r['blind_id']:r for r in ratings}
assert len(by_blind)==48
by_key={ (r['profile'],r['id']):r for r in rows }
for m in mapping:
 row=by_key[m['profile'],m['id']]; rating=by_blind[m['blind_id']]
 assert int(rating['score']) in range(4)
 row['blind_id']=m['blind_id']; row['rating']=rating
T=json.loads((P/'timing-summary.json').read_text())
summary={'review_mode':'independent root AI-agent reviewer, shuffled blind IDs; one pass plus one documented language-judgment typo correction; no rule changes','useful_score':3,'heldout_sha256':(P/'heldout.sha256').read_text().strip(),'protocol_sha256':(P/'protocol.sha256').read_text().strip(),'ratings_sha256':hashlib.sha256((P/'blind-ratings.json').read_bytes()).hexdigest(),'profiles':{}}
for cfg in ['A','B']:
 summary['profiles'][cfg]={}
 for phase in ['heldout','challenge']:
  rr=[r for r in rows if r['profile']==cfg and r['phase']==phase]
  summary['profiles'][cfg][phase]={'n':len(rr),'useful':sum(r['rating']['score']==3 for r in rr),'score_at_least_2_diagnostic_only':sum(r['rating']['score']>=2 for r in rr),'nonempty':sum(r['nonempty'] for r in rr),'score_distribution':{str(i):sum(r['rating']['score']==i for r in rr) for i in range(4)},'timing':T['by_phase'][phase][cfg]['complete_candidate']}
 a=summary['profiles'][cfg]
 a['quantity_gate_passed']=a['heldout']['useful']>=16 and a['challenge']['useful']>=3
 summary['profiles'][cfg]['language_error_examples_from_unchanged_rating_reasons']=({'A':['c19'],'B':['c06','c16','c17','c19']})[cfg]
 a['gate_passed']=a['quantity_gate_passed'] and not a['language_error_examples_from_unchanged_rating_reasons']
(P/'summary.json').write_text(json.dumps(summary,ensure_ascii=False,indent=2)+'\n')
(P/'rated-results.json').write_text(json.dumps(rows,ensure_ascii=False,indent=2)+'\n')
lines=['# Inputia 最后一次本地续词锁定验证','', '日期：2026-10-14。只使用独立生成的公开合成句；未读取私人输入、学习数据或聊天内容。本轮未修改产品代码、未安装模型到产品、未下载新资源。','',
'结论：'+('达到事先数量门槛的方案仍须按逐项语法缺陷检查后决定接入。' if any(x['gate_passed'] for x in summary['profiles'].values()) else '两种方案均未达到事先冻结的接入门槛，停止本轮模型探索；不接入默认推荐。响应快不代表续词质量可用。'),'',
'## 冻结方法','',
'题目由未查看模型输出的独立 agent 生成，先冻结 20 条普通样例和 4 条长上下文/同尾异题挑战，再运行；普通上下文为 13–22 字，挑战为 154–180 字。本轮没有看到输出后改题、改模板或重抽样。A 是纯原文 raw 补全；B 是已由 4 条开发样例固定的 few-shot 模板，仅同步把输出字数上限由 8 改为 12。旧 20 条不参与本轮统计。','',
'两方案统一 1–12 汉字、最多 32 生成 token，允许标点终止；不允许起手标点造成空结果。固定 temperature 0.2、top-k 20、top-p 0.8、seed 42，其余参数见 protocol.json。每条生成一个候选，A/B 顺序交错。两方案各用同一条旧开发句预热一次，不用于评分或调参。','',
'输出打乱为 48 个匿名编号，由另一位独立 agent 按事先 0–3 档规则逐条评分，评分时不提供配置或时间。最高档 3（自然、贴合、合理具体）计“有用”；2 档可接但笼统，仅作诊断统计，不计入过关。门槛为普通 ≥16/20、挑战 ≥3/4，且无明显基础语法错误。允许自然词片段，不要求补全整句。','',
'## 结果','', '|方案|普通有用|挑战有用|普通非空覆盖|普通完整候选 p50 / p95 / max|挑战 p50 / p95 / max|', '|---|---:|---:|---:|---|---|']
for cfg in ['A','B']:
 s=summary['profiles'][cfg]; a=s['heldout'];b=s['challenge']
 fmt=lambda d:' / '.join(f"{d[k]:.1f}" for k in ['p50_ms','p95_ms','max_ms'])+' ms'
 lines.append(f"|{cfg}|{a['useful']}/20|{b['useful']}/4|{a['nonempty']}/20|{fmt(a['timing'])}|{fmt(b['timing'])}|")
lines += ['', 'p50 为中位数，p95 使用 nearest-rank（ceil(0.95n)）；挑战只有 4 条，因此 p95 就是最大值。以上是 loopback 请求发出至唯一完整候选结束的时间，未包含产品键盘入口、目标证明、准入、窗口更新；不能冒充产品端到端延迟。首个字的时间另见 timing-summary.json，它不是完整可选候选。','',
'48 条均非空且符合字符格式，无请求失败、超时或 token 截断，最多实际生成 7 token。因为 grammar 强制至少一个汉字，非空覆盖不代表模型知道何时应该不推荐，也不代表可用率。','',
f"新进程至服务可接请求 {T['readiness_ms']:.1f} ms，随后第一条 A 预热完整候选 {T['warmup'][0]['complete_candidate_ms']:.1f} ms；两者合计约 {(T['readiness_ms']+T['warmup'][0]['complete_candidate_ms']):.1f} ms。未清空操作系统文件缓存，不能称磁盘完全冷启动。峰值 RSS 约 {T['peak_rss_gib']:.2f} GiB。单请求总截止 5 秒、进程 watchdog 120 秒均未触发，因此本轮没有新增超时取消路径的实测证据。最终进程正常退出，已核验 owned PID 不存在、loopback 端口关闭。",'',
'## 原始候选与逐项匿名评分理由','', '评分来自单名独立 agent 按语言规则的主观判读，并非真人用户评分或大规模可用性测量。reviewer 另在未查看映射前指出一处明确语言判断笔误（blind-18），已记录 0→2 的更正；最高档数量不变，评分规则没有变。表中原文和输出保持原样；挑战完整原文在下方列出。','', '|编号|前文|A 候选 / 分数 / 理由|B 候选 / 分数 / 理由|','|---|---|---|---|']
escape=lambda s:str(s).replace('|','\\|').replace('\n',' ')
for phase in ['heldout','challenge']:
 ids=sorted({r['id'] for r in rows if r['phase']==phase})
 for ident in ids:
  a=by_key['A',ident];b=by_key['B',ident]
  context=a['context'] if phase=='heldout' else '长文见下方（'+a['context'][-18:]+'）'
  def cell(r):return f"{r['output']} / {r['rating']['score']} / {r['rating']['reason']}"
  lines.append('|'+ '|'.join(map(escape,[ident,context,cell(a),cell(b)]))+'|')
for r in sorted([r for r in rows if r['phase']=='challenge' and r['profile']=='A'],key=lambda r:r['id']):
 lines += ['',f"### {r['id']}",'',r['context']]
lines += ['', '明确失败例子包括 A 的“降低生产批次”，以及 B 的“我们得先把安排好日程”“使用中缓存”“重新重新测试”。这些问题与是否必须补全整句无关；本轮没有通过改变评分来放宽接错。', '', '## 使用边界','',
'本轮只验证官方 Qwen3-1.7B Q8_0 GGUF 与官方 llama.cpp b11429 的离线实验运行时。产品 helper 使用的 llama-cpp-2 版本与实验运行时不同，实验不能代替产品 helper 验收。测试仅有中文纯汉字候选、单个候选和一次采样，未覆盖中英混输、所有行业术语或真实用户工作流。未证实质量过关的模型不应仅凭毫秒级速度进入默认推荐。','',
'没有继续换模型或调参数。即使未来采用其他模型，也仍须异步预取，候选发布、固定编号、拒绝与遗忘过滤、隐私注销和最终字段证明独立成立；本轮没有实现这些生产准入。','',
'## 证据','',
f"- 冻结题目 SHA256：`{summary['heldout_sha256']}`。",
f"- 冻结协议 SHA256：`{summary['protocol_sha256']}`。",
f"- 匿名评分 SHA256：`{summary['ratings_sha256']}`。",
'- heldout.json / protocol.json：题目与完整模板、采样、约束。',
'- results.jsonl：全部原始输出、SSE 时间与结束原因。',
'- blind-review.json / blind-ratings.json / blind-mapping.json：匿名输入、独立评分和揭盲映射。',
'- rated-results.json / summary.json / timing-summary.json：逐项汇合结果与统计。',
'- runtime-results.json / server.log：进程参数、退出与运行日志。',
'- model-runtime-provenance.json：既有模型与官方运行时来源、版本和 SHA256；本轮未新下载。', '- reproduce.py：只使用已存在的本地资源复跑冻结协议，写入新的结果目录，不覆盖此次证据。']
(P/'report.md').write_text('\n'.join(lines)+'\n')
print(json.dumps(summary,ensure_ascii=False,indent=2))
