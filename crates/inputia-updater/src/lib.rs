//! 配对程序的耐久安装事务。核心不执行外部命令，不修改用户数据库，也不提供默认通过的原生适配器。
//! 所有写入都由 begin/run/recover 显式触发；prepare 和 inspect 只读。
#![cfg(unix)]
pub mod archive;
mod engine;
mod filesystem;
pub mod guardian;
mod model;
pub mod native_code;
pub mod native_input_source;
pub mod native_quiescence;

pub use engine::{artifact_set_digest, Transaction, Updater};
pub use filesystem::{fingerprint, validate_archive_entries};
pub use model::*;
