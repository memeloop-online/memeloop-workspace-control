use k8s_openapi::api::core::v1::PersistentVolumeClaim;
use kube::Api;

use crate::workspaces::Workspace;

use super::{KubernetesCoordinator, ReconcileError};

impl KubernetesCoordinator {
    pub(super) async fn verify_home_volume_binding(
        &self,
        workspace: &Workspace,
    ) -> Result<(), ReconcileError> {
        self.builder.validate_home_volume_binding(workspace)?;
        let Some(binding) = &workspace.home_volume_binding else {
            return Ok(());
        };
        let invalid = |reason| ReconcileError::InvalidHomeVolumeBinding {
            claim_name: binding.claim_name.clone(),
            reason,
        };
        let claims = Api::<PersistentVolumeClaim>::namespaced(
            self.client.clone(),
            workspace.runtime.namespace(),
        );
        let claim = claims
            .get_opt(&binding.claim_name)
            .await?
            .ok_or_else(|| invalid("claim does not exist"))?;
        if claim.metadata.uid.as_deref() != Some(binding.claim_uid.as_str()) {
            return Err(invalid("claim UID does not match the persisted binding"));
        }
        if claim.metadata.deletion_timestamp.is_some() {
            return Err(invalid("claim is terminating"));
        }
        let status = claim
            .status
            .as_ref()
            .ok_or_else(|| invalid("claim has no status"))?;
        if status.phase.as_deref() != Some("Bound") {
            return Err(invalid("claim is not Bound"));
        }
        let capacity = status
            .capacity
            .as_ref()
            .and_then(|values| values.get("storage"));
        if !capacity
            .is_some_and(|quantity| capacity_is_sufficient(&quantity.0, binding.capacity_gib))
        {
            return Err(invalid(
                "claim capacity is missing, invalid, or smaller than the persisted binding",
            ));
        }
        Ok(())
    }
}

fn capacity_is_sufficient(quantity: &str, capacity_gib: u64) -> bool {
    capacity_gib.checked_mul(1 << 30).is_some_and(|required| {
        home_volume_capacity_bytes(quantity).is_some_and(|actual| actual >= required)
    })
}

pub fn home_volume_capacity_bytes(quantity: &str) -> Option<u64> {
    let (numerator, denominator) = quantity_ratio(quantity)?;
    if numerator % denominator != 0 {
        return None;
    }
    let bytes = u64::try_from(numerator / denominator).ok()?;
    (bytes <= i64::MAX as u64).then_some(bytes)
}

fn quantity_ratio(quantity: &str) -> Option<(u128, u128)> {
    let quantity = quantity.strip_prefix('+').unwrap_or(quantity);
    let split = quantity
        .find(|character: char| character.is_ascii_alphabetic())
        .unwrap_or(quantity.len());
    let (number, suffix) = quantity.split_at(split);
    let (whole, fraction) = number.split_once('.').unwrap_or((number, ""));
    if whole.is_empty() && fraction.is_empty()
        || !whole
            .bytes()
            .chain(fraction.bytes())
            .all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    let mut numerator = format!("{whole}{fraction}").parse::<u128>().ok()?;
    let mut denominator = 10_u128.checked_pow(u32::try_from(fraction.len()).ok()?)?;
    let decimal_exponent = match suffix {
        "Ki" | "Mi" | "Gi" | "Ti" | "Pi" | "Ei" => {
            let exponent = match suffix {
                "Ki" => 10,
                "Mi" => 20,
                "Gi" => 30,
                "Ti" => 40,
                "Pi" => 50,
                _ => 60,
            };
            numerator = numerator.checked_mul(1_u128 << exponent)?;
            0
        }
        "" => 0,
        "n" => -9,
        "u" => -6,
        "m" => -3,
        "k" => 3,
        "M" => 6,
        "G" => 9,
        "T" => 12,
        "P" => 15,
        "E" => 18,
        exponent if exponent.starts_with(['e', 'E']) => exponent[1..].parse::<i32>().ok()?,
        _ => return None,
    };
    let multiplier = 10_u128.checked_pow(decimal_exponent.unsigned_abs())?;
    if decimal_exponent >= 0 {
        numerator = numerator.checked_mul(multiplier)?;
    } else {
        denominator = denominator.checked_mul(multiplier)?;
    }
    Some((numerator, denominator))
}

#[cfg(test)]
mod tests {
    use super::{capacity_is_sufficient, home_volume_capacity_bytes};

    #[test]
    fn shared_capacity_parser_requires_exact_bytes_in_the_kubernetes_range() {
        for quantity in [
            "20Gi",
            "20480Mi",
            "21474836480",
            "2.147483648e10",
            "20.0Gi",
            "+20Gi",
        ] {
            assert_eq!(
                home_volume_capacity_bytes(quantity),
                Some(20 * (1 << 30)),
                "{quantity}"
            );
        }
        assert_eq!(home_volume_capacity_bytes("1000m"), Some(1));
        assert_eq!(home_volume_capacity_bytes("0"), Some(0));
        for quantity in [
            "1m",
            "9223372036854775808",
            "8Ei",
            "1e-999",
            "1e999",
            "1.2.3Gi",
            "-1Gi",
            " 20Gi",
        ] {
            assert_eq!(home_volume_capacity_bytes(quantity), None, "{quantity}");
        }
    }

    #[test]
    fn capacity_uses_exact_quantities_and_rejects_insufficient_or_invalid_values() {
        for quantity in [
            "20Gi",
            "20480Mi",
            "21474836480",
            "2.147483648e10",
            "20.0Gi",
            "+20Gi",
            "1Ti",
        ] {
            assert!(capacity_is_sufficient(quantity, 20), "{quantity}");
        }
        for quantity in [
            "19Gi",
            "20G",
            "21474836479",
            "19.999999999Gi",
            "20m",
            "",
            "NaN",
            "inf",
            "-20Gi",
            "20GiB",
            "1e999",
            "1.2.3Gi",
        ] {
            assert!(!capacity_is_sufficient(quantity, 20), "{quantity}");
        }
        assert!(!capacity_is_sufficient(
            "340282366920938463463374607431768211455Gi",
            u64::MAX
        ));
    }
}
