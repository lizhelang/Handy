//! 原生端使用与开发工具相同的固定 schema，额外校验逐库和回滚关系。
use crate::{canonical, safe_relative, schema, ReleaseError, Result};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

fn require(value: bool) -> Result<()> {
    if value {
        Ok(())
    } else {
        Err(ReleaseError::InvalidDocument)
    }
}
fn array(value: &Value) -> Result<&Vec<Value>> {
    value.as_array().ok_or(ReleaseError::InvalidDocument)
}
fn text(value: &Value) -> Result<&str> {
    value.as_str().ok_or(ReleaseError::InvalidDocument)
}
fn integer(value: &Value) -> Result<u64> {
    value.as_u64().ok_or(ReleaseError::InvalidDocument)
}
fn indexed<'a>(value: &'a Value, key: &str) -> Result<BTreeMap<&'a str, &'a Value>> {
    let mut result = BTreeMap::new();
    for item in array(value)? {
        require(result.insert(text(&item[key])?, item).is_none())?;
    }
    Ok(result)
}
fn contains_all(left: &Value, right: &Value) -> Result<bool> {
    let left = array(left)?;
    Ok(array(right)?.iter().all(|item| left.contains(item)))
}
pub(crate) fn os_version(value: &str) -> Result<[u32; 3]> {
    let parts: Vec<_> = value.split('.').collect();
    if parts.is_empty() || parts.len() > 3 {
        return Err(ReleaseError::InvalidDocument);
    }
    let mut version = [0; 3];
    for (index, part) in parts.iter().enumerate() {
        require(!part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))?;
        version[index] = part.parse().map_err(|_| ReleaseError::InvalidDocument)?;
    }
    Ok(version)
}

fn stores<'a>(value: &'a Value, product: &Value) -> Result<BTreeMap<&'a str, &'a Value>> {
    let stores = indexed(value, "id")?;
    let expected: BTreeSet<_> = array(&product["compatibility"]["required_stores"])?
        .iter()
        .map(text)
        .collect::<Result<_>>()?;
    require(stores.keys().copied().collect::<BTreeSet<_>>() == expected)?;
    for store in stores.values() {
        let read = &store["readable_schema_range"];
        let write = &store["writable_schema_range"];
        require(
            integer(&read["min"])? <= integer(&write["min"])?
                && integer(&write["min"])? <= integer(&write["max"])?
                && integer(&write["max"])? <= integer(&read["max"])?,
        )?;
        require(
            array(&store["event_formats"]["readable_versions"])?
                .contains(&store["event_formats"]["writable_version"]),
        )?;
        for kind in ["privacy", "revision", "outbox"] {
            require(contains_all(
                &store[format!("{kind}_capabilities")],
                &product["compatibility"][format!("required_{kind}_capabilities")],
            )?)?;
        }
    }
    Ok(stores)
}

pub fn validate_manifest(value: &Value) -> Result<()> {
    let definition = canonical::parse(include_bytes!(
        "../../../release/schema/release-manifest.schema.json"
    ))?;
    schema::validate(value, &definition)?;
    let product: Value = toml::from_str(include_str!("../../../release/product.toml"))
        .map_err(|_| ReleaseError::InvalidDocument)?;
    if value["product_id"] != product["product_id"] {
        return Err(ReleaseError::WrongProduct);
    }
    let target = &value["target"];
    for tested in array(&target["tested_os"])? {
        require(os_version(text(tested)?)? >= os_version(text(&target["min_os"])?)?)?;
    }
    let components = indexed(&value["components"], "role")?;
    for role in ["control", "ime", "settings", "updater", "bootstrap"] {
        require(components.contains_key(role))?;
    }
    for expected in array(&product["components"])? {
        require(components[text(&expected["role"])?]["bundle_id"] == expected["bundle_id"])?;
    }
    let mut teams = BTreeSet::new();
    for component in components.values() {
        require(component["bundle_id"] == component["signing_requirement"]["bundle_id"])?;
        let cdhashes = array(&component["cdhashes"])?;
        // 当前公开目标只有单一 arm64 slice；Apple CDHash 固定为 20 字节。
        require(
            cdhashes.len() == 1
                && text(&cdhashes[0])?.len() == 40
                && text(&cdhashes[0])?
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
        )?;
        teams.insert(text(&component["signing_requirement"]["team_id"])?);
    }
    require(teams.len() == 1)?;
    let current = stores(&value["stores"], &product)?;
    let rollbacks = indexed(&value["rollback_targets"], "release_id")?;
    require(!rollbacks.contains_key(text(&value["release_id"])?))?;
    for rollback in rollbacks.values() {
        let previous = stores(&rollback["stores"], &product)?;
        for (id, state) in &current {
            let before = previous[id];
            for range in ["readable_schema_range", "writable_schema_range"] {
                require(
                    integer(&before[range]["min"])?
                        <= integer(&state["writable_schema_range"]["min"])?
                        && integer(&before[range]["max"])?
                            >= integer(&state["writable_schema_range"]["max"])?,
                )?;
            }
            require(
                array(&before["event_formats"]["readable_versions"])?
                    .contains(&state["event_formats"]["writable_version"]),
            )?;
            require(
                array(&state["event_formats"]["readable_versions"])?
                    .contains(&before["event_formats"]["writable_version"]),
            )?;
            for capabilities in [
                "privacy_capabilities",
                "revision_capabilities",
                "outbox_capabilities",
            ] {
                require(contains_all(&before[capabilities], &state[capabilities])?)?;
            }
        }
    }
    indexed(&value["resources"], "id")?;
    let artifacts = indexed(&value["distribution_artifacts"], "role")?;
    require(artifacts.contains_key("installer-dmg"))?;
    let mut paths = BTreeSet::new();
    for artifact in components
        .values()
        .copied()
        .chain(artifacts.values().copied())
        .chain(std::iter::once(&value["pair_manifest"]))
    {
        let path = text(&artifact["artifact"])?;
        safe_relative(path)?;
        require(paths.insert(path))?;
    }
    Ok(())
}

pub fn validate_attestation(value: &Value) -> Result<()> {
    let definition = canonical::parse(include_bytes!(
        "../../../release/schema/release-attestation.schema.json"
    ))?;
    schema::validate(value, &definition)?;
    indexed(&value["rollback_reports"], "release_id")?;
    crate::utc(text(&value["issued_at"])?)?;
    Ok(())
}

/// 只校验逐库合同；运行中真实 schema、事件格式和迁移结果仍由更新器读取。
pub fn validate_transition(current: &Value, target: &Value) -> Result<()> {
    validate_manifest(current)?;
    validate_manifest(target)?;
    let target_stores = indexed(&target["stores"], "id")?;
    for before in array(&current["stores"])? {
        let after = target_stores[text(&before["id"])?];
        for range in ["readable_schema_range", "writable_schema_range"] {
            if integer(&after[range]["min"])? > integer(&before["writable_schema_range"]["min"])?
                || integer(&after[range]["max"])?
                    < integer(&before["writable_schema_range"]["max"])?
            {
                return Err(ReleaseError::Incompatible);
            }
        }
        if !array(&after["event_formats"]["readable_versions"])?
            .contains(&before["event_formats"]["writable_version"])
        {
            return Err(ReleaseError::Incompatible);
        }
        for key in [
            "privacy_capabilities",
            "revision_capabilities",
            "outbox_capabilities",
        ] {
            if !contains_all(&after[key], &before[key])? {
                return Err(ReleaseError::Incompatible);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn python_fixture_and_semantic_rejections_match() {
        let value = canonical::parse(include_bytes!("../tests/fixtures/manifest.json")).unwrap();
        validate_manifest(&value).unwrap();
        validate_transition(&value, &value).unwrap();
        for (pointer, replacement) in [
            ("/build", serde_json::json!(true)),
            (
                "/components/0/signing_requirement/bundle_id",
                serde_json::json!("bad"),
            ),
            (
                "/components/3/bundle_id",
                serde_json::json!("com.other.updater"),
            ),
            (
                "/components/4/bundle_id",
                serde_json::json!("com.other.bootstrap"),
            ),
            (
                "/components/0/cdhashes",
                serde_json::json!(["b".repeat(64)]),
            ),
            (
                "/components/1/cdhashes",
                serde_json::json!(["b".repeat(40), "c".repeat(40)]),
            ),
            (
                "/distribution_artifacts/0/artifact",
                serde_json::json!("a/../bad"),
            ),
            ("/stores/0/writable_schema_range/max", serde_json::json!(3)),
            (
                "/rollback_targets/0/stores/0/privacy_capabilities",
                serde_json::json!([]),
            ),
        ] {
            let mut bad = value.clone();
            *bad.pointer_mut(pointer).unwrap() = replacement;
            assert!(validate_manifest(&bad).is_err(), "{pointer}");
        }
    }
}
