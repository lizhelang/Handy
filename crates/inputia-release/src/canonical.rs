//! 与开发工具的 sort_keys/ensure_ascii=false/紧凑 JSON 一致；只支持精确整数。

use crate::{ReleaseError, Result};
use serde::de::{self, Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};
use serde_json::{Map, Number, Value};

pub const MAX_DOCUMENT_BYTES: usize = 4 * 1024 * 1024;
const MAX_EXACT_INTEGER: i64 = 9_007_199_254_740_991;

struct StrictValue(Value);
impl<'de> Deserialize<'de> for StrictValue {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        struct StrictVisitor;
        impl<'de> Visitor<'de> for StrictVisitor {
            type Value = StrictValue;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("bounded integer-only JSON without duplicate keys")
            }
            fn visit_bool<E: de::Error>(self, v: bool) -> std::result::Result<Self::Value, E> {
                Ok(StrictValue(Value::Bool(v)))
            }
            fn visit_unit<E: de::Error>(self) -> std::result::Result<Self::Value, E> {
                Ok(StrictValue(Value::Null))
            }
            fn visit_str<E: de::Error>(self, v: &str) -> std::result::Result<Self::Value, E> {
                Ok(StrictValue(Value::String(v.into())))
            }
            fn visit_string<E: de::Error>(self, v: String) -> std::result::Result<Self::Value, E> {
                Ok(StrictValue(Value::String(v)))
            }
            fn visit_i64<E: de::Error>(self, v: i64) -> std::result::Result<Self::Value, E> {
                if !(-MAX_EXACT_INTEGER..=MAX_EXACT_INTEGER).contains(&v) {
                    return Err(E::custom("integer out of range"));
                }
                Ok(StrictValue(Value::Number(Number::from(v))))
            }
            fn visit_u64<E: de::Error>(self, v: u64) -> std::result::Result<Self::Value, E> {
                if v > MAX_EXACT_INTEGER as u64 {
                    return Err(E::custom("integer out of range"));
                }
                Ok(StrictValue(Value::Number(Number::from(v))))
            }
            fn visit_seq<A: SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> std::result::Result<Self::Value, A::Error> {
                let mut result = Vec::new();
                while let Some(StrictValue(value)) = seq.next_element()? {
                    if result.len() >= 1024 {
                        return Err(de::Error::custom("array too large"));
                    }
                    result.push(value);
                }
                Ok(StrictValue(Value::Array(result)))
            }
            fn visit_map<A: MapAccess<'de>>(
                self,
                mut map: A,
            ) -> std::result::Result<Self::Value, A::Error> {
                let mut result = Map::new();
                while let Some(key) = map.next_key::<String>()? {
                    if result.len() >= 1024 || result.contains_key(&key) {
                        return Err(de::Error::custom("duplicate key or object too large"));
                    }
                    let StrictValue(value) = map.next_value()?;
                    result.insert(key, value);
                }
                Ok(StrictValue(Value::Object(result)))
            }
        }
        deserializer.deserialize_any(StrictVisitor)
    }
}

pub fn parse(bytes: &[u8]) -> Result<Value> {
    if bytes.is_empty() || bytes.len() > MAX_DOCUMENT_BYTES {
        return Err(ReleaseError::InvalidDocument);
    }
    serde_json::from_slice::<StrictValue>(bytes)
        .map(|value| value.0)
        .map_err(|_| ReleaseError::InvalidDocument)
}

pub fn encode(value: &Value) -> Result<Vec<u8>> {
    // 禁止调用方绕过有界整数合同；显式排序，不依赖依赖图的 preserve_order 特性。
    let bytes = serde_json::to_vec(value).map_err(|_| ReleaseError::InvalidDocument)?;
    let mut value = parse(&bytes)?;
    value.sort_all_objects();
    serde_json::to_vec(&value).map_err(|_| ReleaseError::InvalidDocument)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_duplicates_aliases_float_trailing_and_unsafe_integer() {
        for bytes in [
            br#"{"a":1,"a":2}"#.as_slice(),
            br#"{"a":1,"\u0061":2}"#,
            br#"{"nested":{"a":1,"a":2}}"#,
            b"1.0",
            b"1e0",
            b"-0",
            b"9007199254740992",
            b"{}{}",
            b"NaN",
        ] {
            assert!(parse(bytes).is_err(), "{:?}", bytes);
        }
    }
    #[test]
    fn canonical_matches_python_unicode_and_control_escaping() {
        let value = parse("{\"中\":\"文\\n\",\"a\":1,\"b\":[true,null,-2]}".as_bytes()).unwrap();
        assert_eq!(
            String::from_utf8(encode(&value).unwrap()).unwrap(),
            "{\"a\":1,\"b\":[true,null,-2],\"中\":\"文\\n\"}"
        );
        let nested = parse(br#"{"z":{"q":1,"a":[{"z":2,"a":3}]},"a":0}"#).unwrap();
        assert_eq!(
            encode(&nested).unwrap(),
            br#"{"a":0,"z":{"a":[{"a":3,"z":2}],"q":1}}"#
        );
    }
}
