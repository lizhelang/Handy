//! 本地知识文件索引；外部调用仅能读取明确开放的来源。
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};

type Result<T> = std::result::Result<T, String>;
fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn field<'a>(p: &'a Value, key: &str) -> Result<&'a str> {
    p.get(key)
        .and_then(Value::as_str)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| format!("缺少参数 {key}"))
}
fn supported(p: &Path) -> bool {
    matches!(
        p.extension()
            .and_then(|v| v.to_str())
            .unwrap_or("")
            .to_lowercase()
            .as_str(),
        "txt" | "md" | "markdown" | "csv" | "tsv" | "json" | "yaml" | "yml" | "rst" | "log"
    )
}
fn safe_directory(p: &Path) -> bool {
    fs::symlink_metadata(p).is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink())
}

/// 文件来源与分段引用索引。所有写入仅供应用内部调用。
pub struct KnowledgeStore {
    conn: Connection,
    root: PathBuf,
}
impl KnowledgeStore {
    pub fn open(root: &Path) -> Result<Self> {
        fs::create_dir_all(root.join("knowledge/files")).map_err(err)?;
        let canonical_root = fs::canonicalize(root).map_err(err)?;
        let root = canonical_root.as_path();
        let conn = Connection::open(root.join("knowledge/index.sqlite")).map_err(err)?;
        conn.busy_timeout(std::time::Duration::from_secs(5))
            .map_err(err)?;
        conn.execute_batch("CREATE TABLE IF NOT EXISTS kb_sources(id TEXT PRIMARY KEY,name TEXT NOT NULL,kind TEXT NOT NULL,path TEXT NOT NULL UNIQUE,enabled INTEGER NOT NULL DEFAULT 1,external_access INTEGER NOT NULL DEFAULT 0,status TEXT NOT NULL DEFAULT 'pending'); CREATE TABLE IF NOT EXISTS kb_chunks(id TEXT PRIMARY KEY,source_id TEXT NOT NULL,path TEXT NOT NULL,revision TEXT NOT NULL,title TEXT NOT NULL,text TEXT NOT NULL,locator TEXT NOT NULL); CREATE INDEX IF NOT EXISTS kb_chunks_source ON kb_chunks(source_id);").map_err(err)?;
        conn.execute("INSERT OR IGNORE INTO kb_sources(id,name,kind,path) VALUES('managed','知识文件','managed',?1)", [root.join("knowledge/files").to_string_lossy().as_ref()]).map_err(err)?;
        Ok(Self {
            conn,
            root: root.to_owned(),
        })
    }
    /// 打开已有知识索引，不创建或修改来源配置。
    pub fn open_readonly(root: &Path) -> Result<Self> {
        let conn = Connection::open_with_flags(
            root.join("knowledge/index.sqlite"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .map_err(err)?;
        conn.busy_timeout(std::time::Duration::from_secs(5))
            .map_err(err)?;
        Ok(Self {
            conn,
            root: root.to_owned(),
        })
    }
    /// 分发应用动作；external 模式强制只读并过滤未共享来源。
    pub fn dispatch(&self, action: &str, p: Value, external: bool) -> Result<Value> {
        if external && !matches!(action, "status" | "sources" | "search" | "read") {
            return Err("外部知识接口只允许读取".into());
        }
        if let Some(result) = super::knowledge_history::handle(&self.root, action, &p, external)? {
            return Ok(result);
        }
        let data = self.dispatch_files(action, &p, external)?;
        super::knowledge_history::augment(&self.root, action, &p, external, data)
    }
    fn dispatch_files(&self, action: &str, p: &Value, external: bool) -> Result<Value> {
        match action {
            "status" | "sources" => {
                let sources = self.sources(external)?;
                Ok(
                    json!({"protocol_version":1,"search_mode":"keyword","managed_path": if external {Value::Null} else {json!(self.managed()?)},"sources":sources,"supported_extensions":["txt","md","markdown","csv","tsv","json","yaml","yml","rst","log"]}),
                )
            }
            "add_directory" => {
                let raw = PathBuf::from(field(p, "path")?);
                if !safe_directory(&raw) {
                    return Err("请选择存在的普通文件夹，不支持符号链接".into());
                }
                let path = fs::canonicalize(raw).map_err(err)?;
                if path == fs::canonicalize(&self.root).map_err(err)?
                    || self.root.starts_with(&path)
                    || path.starts_with(self.root.join("knowledge"))
                {
                    return Err("不能索引应用数据目录".into());
                }
                let id = format!("folder-{}", hash(path.to_string_lossy().as_bytes()));
                self.conn.execute("INSERT OR IGNORE INTO kb_sources(id,name,kind,path) VALUES(?1,?2,'directory',?3)",params![id,path.file_name().unwrap_or_default().to_string_lossy(),path.to_string_lossy()]).map_err(err)?;
                self.sync()?;
                Ok(json!({"id":id}))
            }
            "import_files" => {
                let paths = p
                    .get("paths")
                    .and_then(Value::as_array)
                    .ok_or("缺少 paths")?;
                let dest = self.managed()?;
                if !safe_directory(&dest) || !no_symlink_ancestors(&dest) {
                    return Err("托管目录不可用或包含符号链接".into());
                }
                if paths.is_empty() || paths.len() > 100 {
                    return Err("每次请选择1至100个文件".into());
                }
                let mut validated = Vec::new();
                let mut total_bytes = 0usize;
                for raw in paths {
                    let src = PathBuf::from(raw.as_str().ok_or("文件路径必须是字符串")?);
                    let metadata = fs::symlink_metadata(&src).map_err(err)?;
                    if !metadata.is_file() || metadata.file_type().is_symlink() || !supported(&src)
                    {
                        return Err(format!(
                            "不支持的知识文件：{}（PDF/DOCX 尚未支持）",
                            src.display()
                        ));
                    }
                    let bytes = read_bounded(&src)?;
                    validate_text(&bytes)?;
                    total_bytes += bytes.len();
                    if total_bytes > 64 * 1024 * 1024 {
                        return Err("单次导入不能超过64 MiB".into());
                    }
                    let name = src.file_name().ok_or("文件名无效")?.to_owned();
                    validated.push((name, bytes));
                }
                let mut added = Vec::new();
                let mut errors = Vec::new();
                for (name, bytes) in validated {
                    let target = unique_path(&dest.join(&name));
                    let write = (|| -> Result<()> {
                        use std::io::Write;
                        let mut file = fs::OpenOptions::new()
                            .write(true)
                            .create_new(true)
                            .open(&target)
                            .map_err(err)?;
                        if let Err(error) = file.write_all(&bytes) {
                            drop(file);
                            let _ = fs::remove_file(&target);
                            return Err(err(error));
                        }
                        Ok(())
                    })();
                    match write {
                        Ok(()) => added.push(target),
                        Err(error) => errors.push(format!("{}: {error}", target.display())),
                    }
                }
                if let Err(error) = self.sync() {
                    errors.push(format!("文件已写入，索引同步失败：{error}"));
                }
                Ok(json!({"paths":added,"errors":errors,"partial":!errors.is_empty()}))
            }
            "save_note" => {
                let title = field(p, "title")?;
                let text = field(p, "text")?;
                let name: String = title
                    .chars()
                    .filter(|c| c.is_alphanumeric() || matches!(c, ' ' | '_' | '-'))
                    .take(100)
                    .collect();
                if name.trim().is_empty() {
                    return Err("笔记标题无效".into());
                }
                validate_text(text.as_bytes())?;
                let dest = self.managed()?;
                if !safe_directory(&dest) || !no_symlink_ancestors(&dest) {
                    return Err("托管目录不可用或包含符号链接".into());
                }
                let path = unique_path(&dest.join(format!("{}.md", name.trim())));
                {
                    use std::io::Write;
                    let mut file = fs::OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(&path)
                        .map_err(err)?;
                    if let Err(error) = file.write_all(text.as_bytes()) {
                        drop(file);
                        let _ = fs::remove_file(&path);
                        return Err(err(error));
                    }
                }
                self.sync()?;
                Ok(json!({"path":path}))
            }
            "set_managed_path" => {
                let path = PathBuf::from(field(p, "path")?);
                fs::create_dir_all(&path).map_err(err)?;
                if !safe_directory(&path) {
                    return Err("托管目录不能是符号链接".into());
                }
                let path = fs::canonicalize(path).map_err(err)?;
                if self.root.starts_with(&path) || path == self.root.join("knowledge") {
                    return Err("不能使用应用索引目录作为知识文件目录".into());
                }
                self.conn
                    .execute(
                        "UPDATE kb_sources SET path=?1,status='pending' WHERE id='managed'",
                        [path.to_string_lossy().as_ref()],
                    )
                    .map_err(err)?;
                self.sync()?;
                Ok(json!({"managed_path":path}))
            }
            "update_source" => {
                let id = field(p, "id")?;
                for key in ["enabled", "external_access"] {
                    if let Some(v) = p.get(key) {
                        let v = v.as_bool().ok_or("来源开关必须是布尔值")?;
                        self.conn
                            .execute(
                                &format!("UPDATE kb_sources SET {key}=?1 WHERE id=?2"),
                                params![v, id],
                            )
                            .map_err(err)?;
                    }
                }
                self.dispatch_files("status", &json!({}), false)
            }
            "remove_source" => {
                let id = field(p, "id")?;
                if id == "managed" {
                    return Err("托管来源可禁用，不能删除".into());
                }
                self.conn
                    .execute("DELETE FROM kb_chunks WHERE source_id=?1", [id])
                    .map_err(err)?;
                self.conn
                    .execute("DELETE FROM kb_sources WHERE id=?1", [id])
                    .map_err(err)?;
                Ok(json!({"removed":id}))
            }
            "sync" => self.sync(),
            "search" => self.search(p, external),
            "read" => self.read(p, external),
            _ => Err(format!("未知知识库操作：{action}")),
        }
    }
    fn managed(&self) -> Result<PathBuf> {
        self.conn
            .query_row("SELECT path FROM kb_sources WHERE id='managed'", [], |r| {
                r.get::<_, String>(0)
            })
            .map(PathBuf::from)
            .map_err(err)
    }
    fn sources(&self, external: bool) -> Result<Vec<Value>> {
        let mut stmt=self.conn.prepare("SELECT id,name,kind,path,enabled,external_access,status,(SELECT COUNT(*) FROM kb_chunks c WHERE c.source_id=s.id) FROM kb_sources s WHERE (?1=0 OR (external_access=1 AND enabled=1)) ORDER BY id").map_err(err)?;
        let rows=stmt.query_map([external],|r| {let path:String=r.get(3)?;let status:String=r.get(6)?; Ok(json!({"id":r.get::<_,String>(0)?,"name":r.get::<_,String>(1)?,"kind":r.get::<_,String>(2)?,"path":path,"enabled":r.get::<_,bool>(4)?,"external_access":r.get::<_,bool>(5)?,"status":if safe_directory(Path::new(&path)){status}else{"offline".into()},"item_count":r.get::<_,i64>(7)?}))}).map_err(err)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(err)
    }
    fn sync(&self) -> Result<Value> {
        let mut warnings = Vec::new();
        let mut updated = 0;
        for source in self.sources(false)? {
            if source["enabled"] != true {
                continue;
            }
            let id = source["id"].as_str().ok_or("来源 ID 无效")?;
            let path = PathBuf::from(source["path"].as_str().ok_or("来源路径无效")?);
            if !safe_directory(&path) {
                self.conn
                    .execute("UPDATE kb_sources SET status='offline' WHERE id=?1", [id])
                    .map_err(err)?;
                warnings.push(format!("{}：目录不可用", path.display()));
                continue;
            }
            let mut files = Vec::new();
            let mut scan_warnings = Vec::new();
            walk(
                &path,
                &self.root.join("knowledge"),
                &mut files,
                &mut scan_warnings,
                0,
                &mut 0,
            );
            self.conn.execute_batch("SAVEPOINT kb_sync").map_err(err)?;
            let result = (|| -> Result<()> {
                let existing: Vec<String> = {
                    let mut s = self
                        .conn
                        .prepare("SELECT DISTINCT path FROM kb_chunks WHERE source_id=?1")
                        .map_err(err)?;
                    let rows = s.query_map([id], |r| r.get(0)).map_err(err)?;
                    rows.collect::<std::result::Result<_, _>>().map_err(err)?
                };
                for old in existing {
                    if !files.iter().any(|p| p.to_string_lossy() == old) {
                        self.conn
                            .execute(
                                "DELETE FROM kb_chunks WHERE source_id=?1 AND path=?2",
                                params![id, old],
                            )
                            .map_err(err)?;
                    }
                }
                let mut scanned_bytes = 0u64;
                for file in files {
                    let size = fs::metadata(&file).map(|m| m.len()).unwrap_or(0);
                    if size <= 5 * 1024 * 1024 {
                        scanned_bytes += size;
                    }
                    if scanned_bytes > 64 * 1024 * 1024 {
                        scan_warnings.push("单来源扫描超过64 MiB预算，请拆分资料目录".into());
                        break;
                    }
                    let path = file.to_string_lossy();
                    let bytes = match read_bounded(&file) {
                        Ok(b) => b,
                        Err(e) => {
                            scan_warnings.push(format!("{}: {e}", file.display()));
                            self.conn
                                .execute(
                                    "DELETE FROM kb_chunks WHERE source_id=?1 AND path=?2",
                                    params![id, path],
                                )
                                .map_err(err)?;
                            continue;
                        }
                    };
                    let text = match validate_text(&bytes) {
                        Ok(t) => t,
                        Err(e) => {
                            scan_warnings.push(format!("{}: {e}", file.display()));
                            self.conn
                                .execute(
                                    "DELETE FROM kb_chunks WHERE source_id=?1 AND path=?2",
                                    params![id, path],
                                )
                                .map_err(err)?;
                            continue;
                        }
                    };
                    let revision = hash(&bytes);
                    let unchanged=self.conn.query_row("SELECT COUNT(*) FROM kb_chunks WHERE source_id=?1 AND path=?2 AND revision=?3",params![id,path,revision],|r|r.get::<_,i64>(0)).map_err(err)?>0;
                    if unchanged {
                        continue;
                    }
                    self.conn
                        .execute(
                            "DELETE FROM kb_chunks WHERE source_id=?1 AND path=?2",
                            params![id, path],
                        )
                        .map_err(err)?;
                    let title = file.file_name().unwrap_or_default().to_string_lossy();
                    for (index, (start, end, chunk)) in chunks(text).into_iter().enumerate() {
                        let chunk_id =
                            format!("file-{}", hash(format!("{id}\0{path}\0{index}").as_bytes()));
                        let locator = format!("{}#L{}-L{}", path, start, end);
                        self.conn.execute("INSERT INTO kb_chunks(id,source_id,path,revision,title,text,locator) VALUES(?1,?2,?3,?4,?5,?6,?7)",params![chunk_id,id,path,revision,title,chunk,locator]).map_err(err)?;
                    }
                    updated += 1;
                }
                self.conn
                    .execute(
                        "UPDATE kb_sources SET status=?1 WHERE id=?2",
                        params![
                            if scan_warnings.is_empty() {
                                "ready"
                            } else {
                                "warning"
                            },
                            id
                        ],
                    )
                    .map_err(err)?;
                Ok(())
            })();
            if let Err(e) = result {
                let _ = self
                    .conn
                    .execute_batch("ROLLBACK TO kb_sync; RELEASE kb_sync");
                return Err(e);
            }
            self.conn.execute_batch("RELEASE kb_sync").map_err(err)?;
            warnings.extend(scan_warnings);
        }
        Ok(json!({"updated":updated,"warnings":warnings}))
    }
    fn candidates(
        &self,
        external: bool,
        p: &Value,
        read: bool,
    ) -> Result<Vec<(Value, String, String)>> {
        let mut sql = "SELECT c.id,c.revision,c.source_id,c.title,c.text,c.locator,c.path,s.path FROM kb_chunks c JOIN kb_sources s ON c.source_id=s.id WHERE s.enabled=1 AND (?=0 OR s.external_access=1)".to_string();
        let mut args = vec![rusqlite::types::Value::Integer(i64::from(external))];
        if read {
            sql.push_str(" AND c.id=?");
            args.push(field(p, "id")?.to_owned().into());
        } else {
            if let Some(source) = p.get("source_id").and_then(Value::as_str) {
                sql.push_str(" AND c.source_id=?");
                args.push(source.to_owned().into());
            }
            for token in query_text(p)?.split_whitespace() {
                sql.push_str(" AND instr(lower(c.title || ' ' || c.text),lower(?))>0");
                args.push(token.to_owned().into());
            }
        }
        sql.push_str(if read {
            " ORDER BY c.id LIMIT 1"
        } else {
            " ORDER BY c.id LIMIT 1000"
        });
        let mut stmt = self.conn.prepare(&sql).map_err(err)?;
        let rows=stmt.query_map(rusqlite::params_from_iter(args),|r|Ok((json!({"id":r.get::<_,String>(0)?,"revision":r.get::<_,String>(1)?,"source_id":r.get::<_,String>(2)?,"title":r.get::<_,String>(3)?,"text":r.get::<_,String>(4)?,"locator":r.get::<_,String>(5)?,"kind":"file"}),r.get(6)?,r.get(7)?))).map_err(err)?;
        rows.collect::<std::result::Result<_, _>>().map_err(err)
    }
    fn search(&self, p: &Value, external: bool) -> Result<Value> {
        let query = query_text(p)?.trim().to_lowercase();
        if query.len() > 4096 || query.split_whitespace().count() > 32 {
            return Err("查询过长".into());
        }
        let limit = p
            .get("limit")
            .and_then(Value::as_u64)
            .unwrap_or(20)
            .clamp(1, 100) as usize;
        let mut items = Vec::new();
        let mut checked_files = std::collections::HashMap::new();
        let mut warnings = if external {
            vec!["外部接口读取最近一次索引；新增资料需由 Inputia 同步后可见".to_string()]
        } else {
            Vec::new()
        };
        let candidates = self.candidates(external, p, false)?;
        let mut has_more = candidates.len() == 1000;
        if has_more {
            warnings.push("匹配超过候选读取上限，请缩小查询或指定来源".into());
        }
        for (item, path, source_path) in candidates {
            if p.get("source_id")
                .and_then(Value::as_str)
                .is_some_and(|s| item["source_id"] != s)
            {
                continue;
            }
            let is_fresh = *checked_files
                .entry((path.clone(), item["revision"].to_string()))
                .or_insert_with(|| {
                    fresh(&path, &source_path, item["revision"].as_str().unwrap_or(""))
                });
            if !is_fresh {
                warnings.push("部分文件已更改或离线，请在 Inputia 同步知识库".to_string());
                continue;
            }
            items.push(item);
        }
        if let Some(intent) = p.get("_decision_intent").and_then(Value::as_str) {
            items.sort_by(|left, right| {
                intent_score(right, &query, intent)
                    .cmp(&intent_score(left, &query, intent))
                    .then_with(|| {
                        left["id"]
                            .as_str()
                            .unwrap_or("")
                            .cmp(right["id"].as_str().unwrap_or(""))
                    })
            });
        }
        if items.len() > limit {
            has_more = true;
            items.truncate(limit);
        }
        warnings.sort();
        warnings.dedup();
        Ok(json!({
            "items": items,
            "warnings": warnings,
            "has_more": has_more,
            "decision_intent": p.get("_decision_intent").cloned().unwrap_or(Value::Null)
        }))
    }
    fn read(&self, p: &Value, external: bool) -> Result<Value> {
        let id = field(p, "id")?;
        let revision = field(p, "revision")?;
        for (item, path, source_path) in self.candidates(external, p, true)? {
            if item["id"] == id {
                if item["revision"] != revision || !fresh(&path, &source_path, revision) {
                    return Err("引用已过期或来源离线，请同步后重新搜索".into());
                }
                return Ok(item);
            }
        }
        Err("引用不存在或未授权读取".into())
    }
}
fn intent_score(item: &Value, query: &str, intent: &str) -> u32 {
    let title = item["title"].as_str().unwrap_or("").to_lowercase();
    let text = item["text"].as_str().unwrap_or("").to_lowercase();
    let mut score = 0;
    if title.contains(query) {
        score += 100;
    }
    for token in query.split_whitespace() {
        if title.contains(token) {
            score += 12;
        } else if text.contains(token) {
            score += 3;
        }
    }
    let markers: &[&str] = match intent {
        "procedure" => &["如何", "步骤", "方法", "操作", "配置", "安装"],
        "definition" => &["定义", "是什么", "含义", "概念"],
        "comparison" => &["比较", "区别", "差异", "优缺点"],
        "history_lookup" => &["历史", "之前", "过去", "记录"],
        _ => &[],
    };
    score
        + markers
            .iter()
            .filter(|marker| title.contains(**marker))
            .count() as u32
            * 20
}
fn validate_text(bytes: &[u8]) -> Result<&str> {
    if bytes.len() > 5 * 1024 * 1024 {
        return Err("文本文件超过 5 MiB 限制".into());
    }
    if bytes.contains(&0) {
        return Err("不支持二进制内容".into());
    }
    std::str::from_utf8(bytes).map_err(|_| "文本需要 UTF-8 编码".into())
}
fn unique_path(path: &Path) -> PathBuf {
    if fs::symlink_metadata(path).is_err() {
        return path.to_owned();
    }
    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    let ext = path.extension().unwrap_or_default().to_string_lossy();
    for n in 1.. {
        let p = path.with_file_name(format!("{stem}-{n}.{ext}"));
        if fs::symlink_metadata(&p).is_err() {
            return p;
        }
    }
    unreachable!()
}
fn walk(
    path: &Path,
    excluded: &Path,
    files: &mut Vec<PathBuf>,
    warnings: &mut Vec<String>,
    depth: usize,
    visited: &mut usize,
) {
    if depth > 16 {
        warnings.push("扫描深度超过16层，已跳过".into());
        return;
    }
    let entries = match fs::read_dir(path) {
        Ok(e) => e,
        Err(e) => {
            warnings.push(format!("{}: {e}", path.display()));
            return;
        }
    };
    for entry in entries {
        *visited += 1;
        if *visited > 10000 {
            if warnings
                .last()
                .is_none_or(|v| v != "来源超过10000个目录项，扫描已截断")
            {
                warnings.push("来源超过10000个目录项，扫描已截断".into());
            }
            return;
        }
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                warnings.push(e.to_string());
                continue;
            }
        };
        let p = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.')
            || matches!(name.as_ref(), "node_modules" | "target")
            || p == excluded
        {
            continue;
        }
        let kind = match entry.file_type() {
            Ok(k) => k,
            Err(e) => {
                warnings.push(e.to_string());
                continue;
            }
        };
        if kind.is_symlink() {
            continue;
        }
        if kind.is_dir() {
            walk(&p, excluded, files, warnings, depth + 1, visited);
        } else if kind.is_file() {
            if supported(&p) {
                files.push(p);
            } else {
                warnings.push(format!("{}：不支持的文件格式", p.display()));
            }
        }
    }
    files.sort();
}
fn fresh(path: &str, source: &str, revision: &str) -> bool {
    let p = Path::new(path);
    let base = Path::new(source);
    if !safe_directory(base) || !no_symlink_ancestors(p) {
        return false;
    }
    let Ok(relative) = p.strip_prefix(base) else {
        return false;
    };
    let mut current = base.to_owned();
    for part in relative.components() {
        current.push(part);
        if fs::symlink_metadata(&current).map_or(true, |m| m.file_type().is_symlink()) {
            return false;
        }
    }
    read_bounded(p).is_ok_and(|b| hash(&b) == revision)
}
fn chunks(text: &str) -> Vec<(usize, usize, String)> {
    let mut result = Vec::new();
    let mut part = String::new();
    let mut count = 0usize;
    let mut start = 1usize;
    let mut line = 1usize;
    for c in text.chars() {
        part.push(c);
        count += 1;
        let end = line;
        if c == '\n' {
            line += 1;
        }
        if count >= 1600 {
            if !part.trim().is_empty() {
                result.push((start, end, std::mem::take(&mut part)));
            } else {
                part.clear();
            }
            count = 0;
            start = line;
        }
    }
    if !part.trim().is_empty() {
        result.push((start, line, part));
    }
    result
}

fn read_bounded(path: &Path) -> Result<Vec<u8>> {
    use std::io::Read;
    let metadata = fs::metadata(path).map_err(err)?;
    if !metadata.is_file() {
        return Err("知识内容必须是普通文件".into());
    }
    if metadata.len() > 5 * 1024 * 1024 {
        return Err("文本文件超过 5 MiB 限制".into());
    }
    let file = fs::File::open(path).map_err(err)?;
    let mut bytes = Vec::new();
    file.take(5 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(err)?;
    if bytes.len() > 5 * 1024 * 1024 {
        return Err("文本文件超过 5 MiB 限制".into());
    }
    Ok(bytes)
}

fn no_symlink_ancestors(path: &Path) -> bool {
    path.ancestors()
        .filter(|p| !p.as_os_str().is_empty())
        .all(|p| fs::symlink_metadata(p).is_ok_and(|m| !m.file_type().is_symlink()))
}

fn query_text(p: &Value) -> Result<&str> {
    p.get("query")
        .and_then(Value::as_str)
        .ok_or_else(|| "缺少字符串参数 query".into())
}
