//! WakeManager - MAC/IP address book (NVS backed)
//! 对应 C 版 `components/mac_store/mac_store.c`
//!
//! NVS blob 布局与 C 版 `wm_mac_table_t` 逐字节兼容：
//! `u8 count @0` + 10 × { `mac[18]`, `ip[16]` } → 341 字节（全 char/u8，无 padding）。

use std::sync::{Mutex, OnceLock};

use esp_idf_svc::nvs::{EspDefaultNvsPartition, EspNvs, NvsDefault};
use esp_idf_svc::sys::EspError;

use crate::app_config as ac;
use crate::wol;

const NVS_NAMESPACE: &str = "wm_mac";
const NVS_KEY: &str = "list";
const ENTRY_LEN: usize = ac::WM_CFG_MAC_LEN + ac::WM_MAC_IP_LEN; // 34
const BLOB_LEN: usize = 1 + ac::WM_MAC_STORE_MAX * ENTRY_LEN; // 341

#[derive(Clone, Debug, Default)]
pub struct MacEntry {
    /// normalised to "AA:BB:CC:DD:EE:FF"
    pub mac: String,
    /// 可为空（空 = 使用默认广播地址）
    pub ip: String,
}

#[derive(Clone, Debug, Default)]
pub struct MacTable {
    pub entries: Vec<MacEntry>,
}

#[derive(Debug, PartialEq, Eq)]
#[allow(dead_code)] // Storage 字段随 Debug 输出使用
pub enum MacError {
    Invalid,
    NotFound,
    Full,
    Storage(EspError),
}

impl From<EspError> for MacError {
    fn from(e: EspError) -> Self {
        MacError::Storage(e)
    }
}

static PARTITION: OnceLock<EspDefaultNvsPartition> = OnceLock::new();
static TABLE: OnceLock<Mutex<MacTable>> = OnceLock::new();
/// 可写 NVS 句柄缓存：首次写盘时打开一次，之后所有写操作复用
static NVS_RW: OnceLock<Mutex<EspNvs<NvsDefault>>> = OnceLock::new();

/// 打开（或复用）可写句柄。打开失败不缓存，下次写盘时重试。
fn writable_nvs(
    part: &EspDefaultNvsPartition,
) -> Result<&'static Mutex<EspNvs<NvsDefault>>, EspError> {
    if let Some(nvs) = NVS_RW.get() {
        return Ok(nvs);
    }
    let nvs = EspNvs::new(part.clone(), NVS_NAMESPACE, true)?;
    let _ = NVS_RW.set(Mutex::new(nvs));
    Ok(NVS_RW.get().expect("writable NVS handle just set"))
}

/// 对应 C 版 `mac_store_init()`
pub fn init(partition: EspDefaultNvsPartition) -> Result<(), EspError> {
    let mut table = MacTable::default();

    let nvs = EspNvs::new(partition.clone(), NVS_NAMESPACE, false)?;
    let mut buf = [0u8; BLOB_LEN];
    match nvs.get_blob(NVS_KEY, &mut buf) {
        Ok(Some(data)) if data.len() == BLOB_LEN => {
            table = decode(data);
            log::info!(
                "Address book loaded: {} entr{}",
                table.entries.len(),
                if table.entries.len() == 1 { "y" } else { "ies" }
            );
        }
        _ => log::info!("No stored address book, starting empty"),
    }

    let _ = PARTITION.set(partition);
    let _ = TABLE.set(Mutex::new(table));
    Ok(())
}

fn encode(table: &MacTable) -> [u8; BLOB_LEN] {
    let mut buf = [0u8; BLOB_LEN];
    buf[0] = table.entries.len() as u8;
    for (i, e) in table.entries.iter().enumerate().take(ac::WM_MAC_STORE_MAX) {
        let off = 1 + i * ENTRY_LEN;
        let n = e.mac.as_bytes().iter().take(ac::WM_CFG_MAC_LEN - 1).count();
        buf[off..off + n].copy_from_slice(&e.mac.as_bytes()[..n]);
        let off = off + ac::WM_CFG_MAC_LEN;
        let n = e.ip.as_bytes().iter().take(ac::WM_MAC_IP_LEN - 1).count();
        buf[off..off + n].copy_from_slice(&e.ip.as_bytes()[..n]);
    }
    buf
}

fn cstr(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

fn decode(buf: &[u8]) -> MacTable {
    let count = (buf[0] as usize).min(ac::WM_MAC_STORE_MAX);
    let mut entries = Vec::with_capacity(count);
    for i in 0..count {
        let off = 1 + i * ENTRY_LEN;
        entries.push(MacEntry {
            mac: cstr(&buf[off..off + ac::WM_CFG_MAC_LEN]),
            ip: cstr(&buf[off + ac::WM_CFG_MAC_LEN..off + ENTRY_LEN]),
        });
    }
    MacTable { entries }
}

fn persist(table: &MacTable) -> Result<(), EspError> {
    let part = PARTITION.get().expect("mac_store::init not called");
    let nvs = writable_nvs(part)?;
    let blob = encode(table);
    nvs.lock().unwrap().set_blob(NVS_KEY, &blob)?;
    Ok(())
}

fn with<T>(f: impl FnOnce(&MacTable) -> T) -> T {
    f(&TABLE.get().expect("mac_store::init not called").lock().unwrap())
}

/// 对应 C 版 `mac_store_count()`
pub fn count() -> usize {
    with(|t| t.entries.len())
}

/// 对应 C 版 `mac_store_get()`
pub fn get() -> MacTable {
    with(|t| t.clone())
}

/// 对应 C 版 `normalise_mac()`：解析后规范化为大写 "AA:BB:CC:DD:EE:FF"
fn normalise_mac(mac: &str) -> Option<String> {
    wol::parse_mac(mac).map(|b| wol::mac_to_str(&b))
}

/// 空 IP 合法（= 使用默认广播）
fn ip_is_valid(ip: &str) -> bool {
    ip.is_empty() || wol::parse_ipv4(ip).is_some()
}

/// 对应 C 版 `mac_store_get_ip()`
pub fn get_ip(mac: &str) -> Option<String> {
    let norm = normalise_mac(mac)?;
    with(|t| {
        t.entries
            .iter()
            .find(|e| e.mac.eq_ignore_ascii_case(&norm))
            .map(|e| e.ip.clone())
    })
}

/// 对应 C 版 `mac_store_set()`
pub fn set(mac: &str, ip: &str) -> Result<(), MacError> {
    let norm = normalise_mac(mac).ok_or(MacError::Invalid)?;
    if !ip_is_valid(ip) {
        return Err(MacError::Invalid);
    }

    let mut table = TABLE.get().expect("mac_store::init not called").lock().unwrap();
    let idx = table
        .entries
        .iter()
        .position(|e| e.mac.eq_ignore_ascii_case(&norm));
    match idx {
        Some(i) => table.entries[i].ip = ip.to_string(),
        None => {
            if table.entries.len() >= ac::WM_MAC_STORE_MAX {
                log::warn!("Address book full ({} entries)", ac::WM_MAC_STORE_MAX);
                return Err(MacError::Full);
            }
            table.entries.push(MacEntry {
                mac: norm.clone(),
                ip: ip.to_string(),
            });
        }
    }
    persist(&table)?;

    log::info!(
        "Stored {} -> {}",
        norm,
        if ip.is_empty() { "(default)" } else { ip }
    );
    Ok(())
}

/// 对应 C 版 `mac_store_remove()`
pub fn remove(mac: &str) -> Result<(), MacError> {
    let norm = normalise_mac(mac).ok_or(MacError::Invalid)?;
    let mut table = TABLE.get().expect("mac_store::init not called").lock().unwrap();
    let idx = table
        .entries
        .iter()
        .position(|e| e.mac.eq_ignore_ascii_case(&norm))
        .ok_or(MacError::NotFound)?;
    table.entries.remove(idx);
    persist(&table)?;
    log::info!("Removed {norm}");
    Ok(())
}

/// 对应 C 版 `mac_store_clear()`
pub fn clear() -> Result<(), MacError> {
    let mut table = TABLE.get().expect("mac_store::init not called").lock().unwrap();
    table.entries.clear();
    persist(&table)?;
    log::info!("Address book cleared");
    Ok(())
}
