#[cfg(target_os = "linux")]
#[global_allocator]
static GLOBAL_ALLOCATOR: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

#[cfg(target_os = "linux")]
#[allow(non_upper_case_globals)]
#[unsafe(export_name = "_rjem_malloc_conf")]
pub static malloc_conf: &[u8] = b"prof:true,prof_active:false,lg_prof_sample:19\0";

pub mod admin;
pub mod api;
pub mod auth;
pub mod config;
pub mod crypto;
pub mod events;
pub mod injections;
pub mod jobs;
pub mod kubernetes;
pub mod observability;
pub(crate) mod plugin_distribution;
pub mod plugins;
pub mod quota;
pub mod storage;
pub mod templates;
pub mod workspaces;
