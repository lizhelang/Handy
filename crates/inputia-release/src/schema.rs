//! 只执行仓库内已固定 schema 的小型子集；不下载/解释外部 schema 或引用。
use crate::{canonical, ReleaseError, Result};
use serde_json::Value;
use std::collections::BTreeSet;

pub(crate) fn validate(value: &Value, schema: &Value) -> Result<()> {
    visit(value, schema, schema)
}
fn reject_unless(ok: bool) -> Result<()> {
    if ok {
        Ok(())
    } else {
        Err(ReleaseError::InvalidDocument)
    }
}

fn visit(value: &Value, schema: &Value, document: &Value) -> Result<()> {
    let object = schema.as_object().ok_or(ReleaseError::InvalidDocument)?;
    const ALLOWED: &[&str] = &[
        "$schema",
        "$id",
        "$defs",
        "$ref",
        "type",
        "const",
        "enum",
        "properties",
        "required",
        "additionalProperties",
        "items",
        "minItems",
        "maxItems",
        "uniqueItems",
        "minimum",
        "maximum",
        "minLength",
        "maxLength",
        "pattern",
        "description",
        "title",
    ];
    reject_unless(object.keys().all(|key| ALLOWED.contains(&key.as_str())))?;
    if let Some(types) = schema["type"].as_array() {
        let matches = |name: &str| match name {
            "object" => value.is_object(),
            "array" => value.is_array(),
            "string" => value.is_string(),
            "integer" => value.is_i64() || value.is_u64(),
            "boolean" => value.is_boolean(),
            "null" => value.is_null(),
            _ => false,
        };
        reject_unless(types.iter().all(|v| {
            v.as_str().is_some_and(|v| {
                ["object", "array", "string", "integer", "boolean", "null"].contains(&v)
            })
        }))?;
        let selected: Vec<_> = types
            .iter()
            .filter(|v| v.as_str().is_some_and(matches))
            .collect();
        reject_unless(selected.len() == 1)?;
        let mut selected_schema = schema.clone();
        selected_schema["type"] = selected[0].clone();
        return visit(value, &selected_schema, document);
    }
    if let Some(reference) = schema["$ref"].as_str() {
        let name = reference
            .strip_prefix("#/$defs/")
            .ok_or(ReleaseError::InvalidDocument)?;
        return visit(
            value,
            document["$defs"]
                .get(name)
                .ok_or(ReleaseError::InvalidDocument)?,
            document,
        );
    }
    if let Some(expected) = schema.get("const") {
        reject_unless(value == expected)?;
    }
    if let Some(values) = schema.get("enum") {
        reject_unless(
            values
                .as_array()
                .is_some_and(|values| values.contains(value)),
        )?;
    }
    match schema["type"].as_str() {
        Some("object") => {
            let values = value.as_object().ok_or(ReleaseError::InvalidDocument)?;
            let properties = schema["properties"].as_object();
            if let Some(required) = schema["required"].as_array() {
                for key in required {
                    reject_unless(key.as_str().is_some_and(|key| values.contains_key(key)))?;
                }
            }
            for (key, value) in values {
                if let Some(rule) = properties.and_then(|props| props.get(key)) {
                    visit(value, rule, document)?;
                } else {
                    reject_unless(schema["additionalProperties"] != false)?;
                }
            }
        }
        Some("array") => {
            let values = value.as_array().ok_or(ReleaseError::InvalidDocument)?;
            reject_unless(
                values.len() as u64 >= schema["minItems"].as_u64().unwrap_or(0)
                    && values.len() as u64 <= schema["maxItems"].as_u64().unwrap_or(1024),
            )?;
            if schema["uniqueItems"] == true {
                let mut unique = BTreeSet::new();
                for value in values {
                    reject_unless(unique.insert(canonical::encode(value)?))?;
                }
            }
            for value in values {
                visit(value, &schema["items"], document)?;
            }
        }
        Some("integer") => {
            let number = value.as_i64().ok_or(ReleaseError::InvalidDocument)?;
            reject_unless(
                number >= schema["minimum"].as_i64().unwrap_or(-9_007_199_254_740_991)
                    && number <= schema["maximum"].as_i64().unwrap_or(9_007_199_254_740_991),
            )?;
        }
        Some("string") => {
            let text = value.as_str().ok_or(ReleaseError::InvalidDocument)?;
            let count = text.chars().count() as u64;
            reject_unless(
                count >= schema["minLength"].as_u64().unwrap_or(0)
                    && count <= schema["maxLength"].as_u64().unwrap_or(1024),
            )?;
            if let Some(pattern) = schema["pattern"].as_str() {
                let pattern = regex::Regex::new(&format!("^(?:{pattern})$"))
                    .map_err(|_| ReleaseError::InvalidDocument)?;
                reject_unless(pattern.is_match(text))?;
            }
        }
        Some("boolean") => reject_unless(value.is_boolean())?,
        Some("null") => reject_unless(value.is_null())?,
        None => {}
        _ => return Err(ReleaseError::InvalidDocument),
    }
    Ok(())
}
