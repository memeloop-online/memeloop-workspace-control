use serde::{Deserialize, Serialize};
use thiserror::Error;
use utoipa::ToSchema;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct Resources {
    pub cpu_millis: u64,
    pub memory_mib: u64,
    pub gpu_count: u32,
    pub disk_gib: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct QuotaResources {
    pub cpu_millis: u64,
    pub memory_mib: u64,
    pub gpu_count: u32,
    pub disk_gib: u64,
    pub temporary_storage_gib: u64,
}

impl Resources {
    pub fn valid_workspace_request(self) -> bool {
        self.cpu_millis > 0 && self.memory_mib > 0 && self.disk_gib > 0
    }

    pub fn checked_add(self, other: Self) -> Result<Self, QuotaError> {
        Ok(Self {
            cpu_millis: self
                .cpu_millis
                .checked_add(other.cpu_millis)
                .ok_or(QuotaError::Overflow)?,
            memory_mib: self
                .memory_mib
                .checked_add(other.memory_mib)
                .ok_or(QuotaError::Overflow)?,
            gpu_count: self
                .gpu_count
                .checked_add(other.gpu_count)
                .ok_or(QuotaError::Overflow)?,
            disk_gib: self
                .disk_gib
                .checked_add(other.disk_gib)
                .ok_or(QuotaError::Overflow)?,
        })
    }
}

impl QuotaResources {
    pub fn for_workspace(resources: Resources, temporary_storage_gib: u64) -> Self {
        Self {
            cpu_millis: resources.cpu_millis,
            memory_mib: resources.memory_mib,
            gpu_count: resources.gpu_count,
            disk_gib: resources.disk_gib,
            temporary_storage_gib,
        }
    }

    pub fn checked_add(self, other: Self) -> Result<Self, QuotaError> {
        Ok(Self {
            cpu_millis: self
                .cpu_millis
                .checked_add(other.cpu_millis)
                .ok_or(QuotaError::Overflow)?,
            memory_mib: self
                .memory_mib
                .checked_add(other.memory_mib)
                .ok_or(QuotaError::Overflow)?,
            gpu_count: self
                .gpu_count
                .checked_add(other.gpu_count)
                .ok_or(QuotaError::Overflow)?,
            disk_gib: self
                .disk_gib
                .checked_add(other.disk_gib)
                .ok_or(QuotaError::Overflow)?,
            temporary_storage_gib: self
                .temporary_storage_gib
                .checked_add(other.temporary_storage_gib)
                .ok_or(QuotaError::Overflow)?,
        })
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ResourceQuota {
    pub limit: QuotaResources,
}

impl ResourceQuota {
    pub fn admit(
        &self,
        usage: QuotaResources,
        request: QuotaResources,
    ) -> Result<QuotaResources, QuotaError> {
        let proposed = usage.checked_add(request)?;
        check_limit("cpu_millis", proposed.cpu_millis, self.limit.cpu_millis)?;
        check_limit("memory_mib", proposed.memory_mib, self.limit.memory_mib)?;
        check_limit(
            "gpu_count",
            u64::from(proposed.gpu_count),
            u64::from(self.limit.gpu_count),
        )?;
        check_limit("disk_gib", proposed.disk_gib, self.limit.disk_gib)?;
        check_limit(
            "temporary_storage_gib",
            proposed.temporary_storage_gib,
            self.limit.temporary_storage_gib,
        )?;
        Ok(proposed)
    }
}

fn check_limit(resource: &'static str, requested: u64, limit: u64) -> Result<(), QuotaError> {
    if requested > limit {
        return Err(QuotaError::Exceeded {
            resource,
            requested,
            limit,
        });
    }
    Ok(())
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum QuotaError {
    #[error("resource total overflowed")]
    Overflow,
    #[error("quota exceeded for {resource}: requested {requested}, limit {limit}")]
    Exceeded {
        resource: &'static str,
        requested: u64,
        limit: u64,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn admits_request_within_aggregate_limit() {
        let quota = ResourceQuota {
            limit: QuotaResources {
                cpu_millis: 4_000,
                memory_mib: 8_192,
                gpu_count: 1,
                disk_gib: 100,
                temporary_storage_gib: 30,
            },
        };
        let proposed = quota
            .admit(
                QuotaResources {
                    cpu_millis: 1_000,
                    ..QuotaResources::default()
                },
                QuotaResources {
                    cpu_millis: 2_000,
                    memory_mib: 2_048,
                    temporary_storage_gib: 10,
                    ..QuotaResources::default()
                },
            )
            .unwrap();
        assert_eq!(proposed.cpu_millis, 3_000);
        assert_eq!(proposed.temporary_storage_gib, 10);
    }

    #[test]
    fn reports_exact_exceeded_resource() {
        let quota = ResourceQuota {
            limit: QuotaResources {
                temporary_storage_gib: 10,
                ..QuotaResources::default()
            },
        };
        assert_eq!(
            quota.admit(
                QuotaResources::default(),
                QuotaResources {
                    temporary_storage_gib: 11,
                    ..QuotaResources::default()
                }
            ),
            Err(QuotaError::Exceeded {
                resource: "temporary_storage_gib",
                requested: 11,
                limit: 10,
            })
        );
    }
}
