import Foundation
import FoundationModels

@Generable struct UpToThree {
  @Guide(description: "零至三个可直接接在前文末尾的续词，每个一至六个汉字，无合适续词时为空数组。", .maximumCount(3))
  var candidates: [String]
}
@Generable struct One {
  @Guide(description: "可直接接在前文末尾的一个续词，一至六个汉字；没有合适续词则为空字符串。")
  var continuation: String
}
struct Sample: Sendable { let id: String; let context: String }
struct Result: Codable, Sendable {
  var id: String; var config: String; var context: String; var chars: Int
  var maxTokens: Int; var totalDeadlineMS: Int; var prewarmWindowMS: Int
  var prewarmCallMS: Double = 0; var firstSnapshotGenerationMS: Double?
  var firstCompleteCandidateGenerationMS: Double?; var finalGenerationMS: Double?
  var finalTotalMS: Double = 0; var status = "running"; var rawOutput = ""; var candidates: [String] = []; var error: String?
}
let instructions = """
你是中文输入法的光标续写模块，不是聊天助手、摘要器或咨询助手。
先默读完整前文，再给出能够直接拼接在前文最后一个字后面的短续词。把“前文+续词”连读，语法必须自然。续词不重复前文，不解释，不给标签或编号。每项一至六个汉字，最多三项，不要为了凑满而输出近义重复或不自然短语。
只能提出符合已有语境的可能接续，不要增添原文没有提供的日期、数字、质量结论、承诺或其他具体事实。不确定、句子已经结束且没有自然接续时，可以不给候选。
示例一：前文“水已经烧开了，我准备把面条”，可续“放进去”“下锅”。
示例二：前文“这份报告还没检查完，发出去之前应该”，可续“再核对”“先检查”。
示例三：前文“型号和数量都已确认。”，没有自然的短接续，返回空。
"""
let samples: [Sample] = [
 .init(id:"thanks",context:"这次麻烦你帮了大忙，真的非常谢谢"),
 .init(id:"i_want",context:"我想"),
 .init(id:"parcel",context:"快递放在楼下门卫那里了，我下班后顺路去"),
 .init(id:"novel",context:"这本小说刚读到最精彩的地方，我还想继续"),
 .init(id:"meal",context:"米饭已经蒸好了，汤还差一点火候，等汤煮好就可以"),
 .init(id:"empty_list",context:"这段代码在空列表时会报错，我准备先补上"),
 .init(id:"bend_sample",context:"折弯样件的角度偏大，工程师让我们先调整模具间隙，再"),
 .init(id:"same_tail_aquarium",context:"鱼缸的水有些浑浊，过滤器也停止工作了。为了找到问题，我们首先要检查"),
 .init(id:"same_tail_printer",context:"打印机连续卡纸，换了一包纸还是没有改善。为了找到问题，我们首先要检查"),
 .init(id:"sentence_end",context:"会议定在周四下午，所有参会人都已经收到通知。"),
 .init(id:"long_paragraph",context:"我们准备给社区图书室整理一批新收到的书。志愿者已经按文学、历史和自然科学分好类别，也核对了捐赠清单。整理过程中发现，有些书的封面磨损比较严重，还有少数书缺少页码。负责人希望先把这些问题处理好，再统一贴上新的分类标签，最后放到相应书架。现在清单已经核对完，修补材料也送到了，下一步应该先"),
 .init(id:"contact",context:"如果这份表格还有不清楚的地方，可以随时")
]
func ms(_ start:ContinuousClock.Instant)->Double { let d=start.duration(to:.now).components; return Double(d.seconds)*1000+Double(d.attoseconds)/1e15 }
func emit<T:Encodable>(_ value:T) { let e=JSONEncoder();e.outputFormatting=[.sortedKeys,.withoutEscapingSlashes];if let d=try? e.encode(value) { FileHandle.standardOutput.write(d);FileHandle.standardOutput.write(Data([10])) } }
func run(_ sample:Sample,config:String,deadlineMS:Int=5000,warmMS:Int=1000) async -> Result {
 let start=ContinuousClock.now
 let single=config == "single"
 let prompt="【前文】"+sample.context
 let session=LanguageModelSession(model:SystemLanguageModel.default,instructions:instructions+(single ? "本次只输出最可能的一个续词；可以为空字符串。" : ""))
 var r=Result(id:sample.id,config:config,context:sample.context,chars:sample.context.count,maxTokens:single ? 32:72,totalDeadlineMS:deadlineMS,prewarmWindowMS:warmMS)
 let prep=ContinuousClock.now
 if config == "prefix" { session.prewarm(promptPrefix:Prompt(prompt)) } else { session.prewarm() }
 r.prewarmCallMS=ms(prep)
 var generatedAt:ContinuousClock.Instant?
 do {
  if warmMS > 0 { try await Task.sleep(for:.milliseconds(warmMS)) }
  try Task.checkCancellation()
  let generation=ContinuousClock.now;generatedAt=generation
  let options=GenerationOptions(samplingMode:.greedy,maximumResponseTokens:r.maxTokens)
  if single {
   for try await s in session.streamResponse(to:prompt,generating:One.self,options:options) {
    try Task.checkCancellation()
    if r.firstSnapshotGenerationMS == nil { r.firstSnapshotGenerationMS=ms(generation) }
    r.rawOutput=s.content.continuation ?? ""
   }
   if !r.rawOutput.isEmpty { r.candidates=[r.rawOutput] }
  } else {
   for try await s in session.streamResponse(to:prompt,generating:UpToThree.self,options:options) {
    try Task.checkCancellation()
    if r.firstSnapshotGenerationMS == nil { r.firstSnapshotGenerationMS=ms(generation) }
    let values=s.content.candidates ?? []
    if values.count>1 && r.firstCompleteCandidateGenerationMS == nil { r.firstCompleteCandidateGenerationMS=ms(generation) }
    r.candidates=values;r.rawOutput=String(data:try JSONEncoder().encode(values),encoding:.utf8) ?? ""
   }
  }
  try Task.checkCancellation()
  r.finalGenerationMS=ms(generation)
  if r.firstCompleteCandidateGenerationMS == nil && !r.candidates.isEmpty { r.firstCompleteCandidateGenerationMS=r.finalGenerationMS }
  r.status=r.candidates.count <= (single ? 1:3) && Set(r.candidates).count==r.candidates.count && r.candidates.allSatisfy({ (1...6).contains($0.count) }) ? "completed_valid_shape":"completed_invalid_shape"
 } catch { r.status=Task.isCancelled ? "cancelled_at_deadline":"error";r.error=String(String(describing:error).prefix(1200)); if let generatedAt {r.finalGenerationMS=ms(generatedAt)} }
 r.finalTotalMS=ms(start);return r
}
@main struct Bench {
 static func main() async {
  let mode=CommandLine.arguments.dropFirst().first ?? "all"
  emit(["event":"environment","model":"SystemLanguageModel.default","availability":String(describing:SystemLanguageModel.default.availability),"route":"on_device","personal_data":"false","mode":mode])
  guard SystemLanguageModel.default.isAvailable else {return}
  if mode == "cancel" {
   let task=Task{await run(samples[2],config:"single",deadlineMS:100,warmMS:0)}
   let cancel=Task{try? await Task.sleep(for:.milliseconds(100));task.cancel()}
   let r=await task.value;cancel.cancel();emit(r);return
  }
  let configs=["three","single","prefix"]
  for (i,sample) in samples.enumerated() {
   for offset in 0..<3 {
    let config=configs[(i+offset)%3]
    emit(["event":"request_start","id":sample.id,"config":config])
    let task=Task{await run(sample,config:config)}
    let cancel=Task{do{try await Task.sleep(for:.seconds(5))}catch{return};emit(["event":"cancellation_requested","id":sample.id,"config":config]);task.cancel()}
    let r=await task.value;cancel.cancel();emit(r)
   }
  }
 }
}
