//! WakeManager - runtime configuration store (NVS backed)
//! 对应 C 版 `components/config_store/config_store.c`
//!
//! NVS blob 布局与 C 版 `wm_config_t`（repr(C)，260 字节）逐字节兼容：
//! `u16 port @0`、`bool enable @2`、`uri[128] @3`、`user[64] @131`、
//! `pass[64] @195`、尾部 1 字节 padding → sizeof = 260。

use std::sync::{Mutex, OnceLock};

use esp_idf_svc::nvs::{EspDefaultNvsPartition, EspNvs, NvsDefault};
use esp_idf_svc::sys::{EspError, ESP_ERR_NVS_NOT_FOUND};

use crate::app_config as ac;

const NVS_NAMESPACE: &str = "wm_cfg";
const NVS_KEY: &str = "cfg";

/// C 版 `sizeof(wm_config_t)` = 259（字段）+ 1（align-2 padding）
const CFG_BLOB_LEN: usize = 260;

const OFF_PORT: usize = 0;
const OFF_ENABLE: usize = 2;
const OFF_URI: usize = 3;
const OFF_USER: usize = OFF_URI + ac::WM_CFG_HOST_LEN; // 131
const OFF_PASS: usize = OFF_USER + ac::WM_CFG_USER_LEN; // 195

#[derive(Clone, Debug)]
pub struct WmConfig {
    pub wol_udp_port: u16,
    pub mqtt_enable: bool,
    pub mqtt_uri: String,
    pub mqtt_username: String,
    pub mqtt_password: String,
}

impl WmConfig {
    /// 对应 C 版 `config_store_defaults()`（值来自 app_config.h）
    pub fn defaults() -> Self {
        Self {
            wol_udp_port: ac::WOL_UDP_PORT,
            mqtt_enable: ac::WOL_MQTT_ENABLE,
            mqtt_uri: ac::WOL_MQTT_BROKER_URI.to_string(),
            mqtt_username: ac::WOL_MQTT_USERNAME.to_string(),
            mqtt_password: ac::WOL_MQTT_PASSWORD.to_string(),
        }
    }

    /// 编码为 C 版 blob（定长数组 + NUL 截断，等价 strlcpy）
    fn encode(&self) -> [u8; CFG_BLOB_LEN] {
        let mut buf = [0u8; CFG_BLOB_LEN];
        buf[OFF_PORT..OFF_PORT + 2].copy_from_slice(&self.wol_udp_port.to_le_bytes());
        buf[OFF_ENABLE] = self.mqtt_enable as u8;
        write_str(&mut buf[OFF_URI..], &self.mqtt_uri, ac::WM_CFG_HOST_LEN);
        write_str(&mut buf[OFF_USER..], &self.mqtt_username, ac::WM_CFG_USER_LEN);
        write_str(&mut buf[OFF_PASS..], &self.mqtt_password, ac::WM_CFG_PASS_LEN);
        buf
    }

    /// 从 C 版 blob 解码（长度必须精确匹配，与 C 的 `len == sizeof(stored)` 一致）
    fn decode(buf: &[u8]) -> Option<Self> {
        if buf.len() != CFG_BLOB_LEN {
            return None;
        }
        Some(Self {
            wol_udp_port: u16::from_le_bytes([buf[OFF_PORT], buf[OFF_PORT + 1]]),
            mqtt_enable: buf[OFF_ENABLE] != 0,
            mqtt_uri: read_str(&buf[OFF_URI..], ac::WM_CFG_HOST_LEN),
            mqtt_username: read_str(&buf[OFF_USER..], ac::WM_CFG_USER_LEN),
            mqtt_password: read_str(&buf[OFF_PASS..], ac::WM_CFG_PASS_LEN),
        })
    }
}

/// 等价 strlcpy：最多 cap-1 字节 + NUL
fn write_str(dst: &mut [u8], src: &str, cap: usize) {
    let n = src.as_bytes().iter().take(cap - 1).count();
    dst[..n].copy_from_slice(&src.as_bytes()[..n]);
    dst[n] = 0;
}

/// 按 NUL 终止读取（最多 cap-1 字节）
fn read_str(src: &[u8], cap: usize) -> String {
    let end = src[..cap - 1]
        .iter()
        .position(|&b| b == 0)
        .unwrap_or(cap - 1);
    String::from_utf8_lossy(&src[..end]).into_owned()
}

static PARTITION: OnceLock<EspDefaultNvsPartition> = OnceLock::new();
static CFG: OnceLock<Mutex<WmConfig>> = OnceLock::new();
/// 可写 NVS 句柄缓存：首次保存时打开一次，之后所有写操作复用
/// （`EspNvs: Send`，句柄可跨线程共享）。
static NVS_RW: OnceLock<Mutex<EspNvs<NvsDefault>>> = OnceLock::new();

/// 打开（或复用）可写句柄。打开失败不缓存，下次保存时重试。
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

/// 对应 C 版 `config_store_init()`：加载 NVS blob，失败则用编译期默认值
pub fn init(partition: EspDefaultNvsPartition) -> Result<(), EspError> {
    let mut cfg = WmConfig::defaults();

    // 只读打开；全新 flash 上命名空间尚不存在（NVS_NOT_FOUND）→ 用默认值
    // （此路径由 Wokwi 模拟发现：原实现直接 `?` 上抛导致全新设备 abort 循环）
    let loaded = match EspNvs::new(partition.clone(), NVS_NAMESPACE, false) {
        Ok(nvs) => {
            let mut buf = [0u8; CFG_BLOB_LEN];
            match nvs.get_blob(NVS_KEY, &mut buf) {
                Ok(Some(data)) if data.len() == CFG_BLOB_LEN => WmConfig::decode(data),
                _ => None,
            }
        }
        Err(e) if e.code() == ESP_ERR_NVS_NOT_FOUND as i32 => None,
        Err(e) => return Err(e),
    };

    match loaded {
        Some(c) => {
            cfg = c;
            log::info!("Config loaded from NVS");
        }
        None => log::info!("No stored config, using defaults"),
    }

    let _ = PARTITION.set(partition);
    let _ = CFG.set(Mutex::new(cfg));
    Ok(())
}

fn with<T>(f: impl FnOnce(&WmConfig) -> T) -> T {
    let guard = CFG
        .get()
        .expect("config_store::init not called")
        .lock()
        .unwrap();
    f(&guard)
}

/// 对应 C 版 `config_store_get()`（线程安全拷贝）
pub fn get() -> WmConfig {
    with(|c| c.clone())
}

/// 对应 C 版 `config_store_save()`
pub fn save(cfg: &WmConfig) -> Result<(), EspError> {
    let part = PARTITION.get().expect("config_store::init not called");
    let nvs = writable_nvs(part)?;

    let blob = cfg.encode();
    nvs.lock().unwrap().set_blob(NVS_KEY, &blob)?;

    *CFG
        .get()
        .expect("config_store::init not called")
        .lock()
        .unwrap() = cfg.clone();

    log::info!("Config saved");
    Ok(())
}
