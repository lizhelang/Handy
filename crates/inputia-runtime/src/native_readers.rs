//! 同进程原生解码器的实际存活证明；正文仍被持有时不能按网络读者超时结算。
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock, Weak};

fn readers() -> &'static Mutex<HashMap<String, Weak<()>>> {
    static READERS: OnceLock<Mutex<HashMap<String, Weak<()>>>> = OnceLock::new();
    READERS.get_or_init(Default::default)
}

/// 必须先建立此 pin、再登记耐久读者、最后取得正文；在所有正文及原生 session 销毁后释放。
/// 不能克隆或从客户端构造身份，不持有词语或其它业务正文。
pub struct NativeReaderPin {
    id: String,
    _lifetime: Arc<()>,
}
impl NativeReaderPin {
    pub fn new() -> Result<Self, &'static str> {
        let mut bytes = [0u8; 24];
        getrandom::getrandom(&mut bytes).map_err(|_| "native_reader_entropy")?;
        let id = format!(
            "native-reader:{}",
            bytes.iter().map(|b| format!("{b:02x}")).collect::<String>()
        );
        let mut map = readers().lock().map_err(|_| "native_reader_unavailable")?;
        map.retain(|_, value| value.strong_count() != 0);
        if map.len() >= 64 || map.contains_key(&id) {
            return Err("native_reader_budget");
        }
        let lifetime = Arc::new(());
        map.insert(id.clone(), Arc::downgrade(&lifetime));
        Ok(Self {
            id,
            _lifetime: lifetime,
        })
    }
    pub fn id(&self) -> &str {
        &self.id
    }
}

/// 锁中毒时保守保留，不能把无法核验的原生读者算作已清理。
pub(crate) fn is_pinned(id: &str) -> bool {
    if !id.starts_with("native-reader:") {
        return false;
    }
    readers().lock().map_or(true, |map| {
        map.get(id).is_some_and(|value| value.strong_count() != 0)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pin_remains_live_until_actual_owner_drops_and_instances_do_not_share_ack() {
        let first = NativeReaderPin::new().unwrap();
        let second = NativeReaderPin::new().unwrap();
        let id = first.id().to_owned();
        assert_ne!(id, second.id());
        assert!(is_pinned(&id));
        assert!(!is_pinned("host-1"));
        drop(first);
        assert!(!is_pinned(&id));
        assert!(is_pinned(second.id()));
    }
}
