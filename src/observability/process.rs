#[derive(Clone, Copy, Debug, Default, serde::Serialize)]
pub struct ProcessSnapshot {
    pub resident_bytes: u64,
    pub virtual_bytes: u64,
    pub threads: u64,
}

#[derive(Clone, Copy, Debug, serde::Serialize)]
pub struct AllocatorSnapshot {
    pub allocated_bytes: u64,
    pub active_bytes: u64,
    pub resident_bytes: u64,
    pub mapped_bytes: u64,
    pub metadata_bytes: u64,
    pub retained_bytes: u64,
}

#[cfg(target_os = "linux")]
pub(super) fn process_snapshot() -> ProcessSnapshot {
    let Ok(status) = std::fs::read_to_string("/proc/self/status") else {
        return ProcessSnapshot::default();
    };
    let value = |name: &str| {
        status.lines().find_map(|line| {
            let (key, value) = line.split_once(':')?;
            (key == name)
                .then(|| value.split_whitespace().next()?.parse::<u64>().ok())
                .flatten()
        })
    };
    ProcessSnapshot {
        resident_bytes: value("VmRSS").unwrap_or(0).saturating_mul(1024),
        virtual_bytes: value("VmSize").unwrap_or(0).saturating_mul(1024),
        threads: value("Threads").unwrap_or(0),
    }
}

#[cfg(not(target_os = "linux"))]
pub(super) fn process_snapshot() -> ProcessSnapshot {
    ProcessSnapshot::default()
}

#[cfg(target_os = "linux")]
pub(super) fn allocator_snapshot() -> Option<AllocatorSnapshot> {
    use tikv_jemalloc_ctl::{epoch, stats};

    epoch::advance().ok()?;
    Some(AllocatorSnapshot {
        allocated_bytes: u64::try_from(stats::allocated::read().ok()?).ok()?,
        active_bytes: u64::try_from(stats::active::read().ok()?).ok()?,
        resident_bytes: u64::try_from(stats::resident::read().ok()?).ok()?,
        mapped_bytes: u64::try_from(stats::mapped::read().ok()?).ok()?,
        metadata_bytes: u64::try_from(stats::metadata::read().ok()?).ok()?,
        retained_bytes: u64::try_from(stats::retained::read().ok()?).ok()?,
    })
}

#[cfg(not(target_os = "linux"))]
pub(super) fn allocator_snapshot() -> Option<AllocatorSnapshot> {
    None
}
