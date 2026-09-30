//! 更新查询使用新收据，并在网络等待结束后确认仍属于同一次安装上下文。
use inputia_settings::{
    installation::{self, LocatedInstallation, LocatorContext},
    maintenance,
};

pub fn inspect(
    context: &LocatorContext,
    startup: &LocatedInstallation,
) -> Result<LocatedInstallation, &'static str> {
    maintenance::ensure_normal_start(&context.home, context.uid).map_err(|_| "maintenance")?;
    let current = installation::load(context).map_err(|_| "installation_repair")?;
    let mut expected = startup.receipt.clone();
    expected.channel = current.receipt.channel.clone();
    if expected != current.receipt {
        return Err("installation_repair");
    }
    Ok(current)
}
pub fn revalidate(
    context: &LocatorContext,
    original: &LocatedInstallation,
) -> Result<(), &'static str> {
    maintenance::ensure_normal_start(&context.home, context.uid).map_err(|_| "maintenance")?;
    let current = installation::load(context).map_err(|_| "installation_repair")?;
    if &current != original {
        return Err("installation_changed");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use inputia_settings::installation::*;
    use std::{
        fs,
        os::unix::fs::{MetadataExt, PermissionsExt},
        path::Path,
    };
    fn write(path: &Path, bytes: &[u8]) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
    }
    fn fixture() -> (tempfile::TempDir, LocatorContext, LocatedInstallation) {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().canonicalize().unwrap();
        let context = LocatorContext {
            product_id: PRODUCT_ID.into(),
            release_id: "inputia-test-current".into(),
            uid: home.metadata().unwrap().uid(),
            home: home.clone(),
        };
        let receipt = InstallationReceipt {
            schema_version: 1,
            product_id: PRODUCT_ID.into(),
            installation_id: "22222222-2222-4222-8222-222222222222".into(),
            profile_id: "44444444-4444-4444-8444-444444444444".into(),
            uid: context.uid,
            scope: InstallationScope::User,
            data: DataLocation::Managed,
            components: ComponentPaths {
                control: home.join("Applications/Inputia.app"),
                ime: home.join("Library/Input Methods/InputiaUnifiedCandidate.app"),
                settings: home.join("Applications/Inputia 设置.app"),
            },
            release_id: context.release_id.clone(),
            channel: UpdateChannel::Candidate,
        };
        write(
            &context.receipt_path(),
            &serde_json::to_vec(&receipt).unwrap(),
        );
        let located = installation::load(&context).unwrap();
        (temp, context, located)
    }
    #[test]
    fn next_check_uses_new_channel_and_inflight_check_cannot_return_old_result() {
        let (_temp, context, startup) = fixture();
        let in_flight = inspect(&context, &startup).unwrap();
        let mut receipt = startup.receipt.clone();
        receipt.channel = UpdateChannel::Stable;
        write(
            &context.receipt_path(),
            &serde_json::to_vec(&receipt).unwrap(),
        );
        assert_eq!(
            revalidate(&context, &in_flight),
            Err("installation_changed")
        );
        assert_eq!(
            inspect(&context, &startup).unwrap().receipt.channel,
            UpdateChannel::Stable
        );
        receipt.profile_id = "55555555-5555-4555-8555-555555555555".into();
        write(
            &context.receipt_path(),
            &serde_json::to_vec(&receipt).unwrap(),
        );
        assert_eq!(inspect(&context, &startup), Err("installation_repair"));
    }
    #[test]
    fn maintenance_started_during_download_prevents_success_even_with_same_receipt() {
        let (_temp, context, startup) = fixture();
        let in_flight = inspect(&context, &startup).unwrap();
        let marker = maintenance::MaintenanceMarker {
            schema_version: 1,
            transaction_id: "11111111-1111-4111-8111-111111111111".into(),
            installation_id: startup.receipt.installation_id.clone(),
            old_release_id: Some(context.release_id.clone()),
            new_release_id: "inputia-test-next".into(),
            epoch: "33333333-3333-4333-8333-333333333333".into(),
            plan_sha256: "a".repeat(64),
        };
        write(
            &maintenance::marker_path(&context.home),
            &serde_json::to_vec(&marker).unwrap(),
        );
        assert_eq!(revalidate(&context, &in_flight), Err("maintenance"));
        assert_eq!(inspect(&context, &startup), Err("maintenance"));
    }
}
