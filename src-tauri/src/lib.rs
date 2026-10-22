mod battery;
mod history;
mod metrics;

use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tauri::{Manager, State};

use battery::{BatteryInfo, BatteryReader, ChargeState};
use history::{BatterySample, DischargeEstimate, History};
use metrics::{Monitor, SystemSnapshot};

const SAMPLE_INTERVAL: Duration = Duration::from_secs(60);
const RETENTION_SECS: i64 = 30 * 24 * 3600;
const MAX_HISTORY_HOURS: u32 = 30 * 24;

struct AppState {
    monitor: Mutex<Monitor>,
    battery: Mutex<BatteryReader>,
    history: Arc<Mutex<History>>,
    history_persistent: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct BatteryHistory {
    samples: Vec<BatterySample>,
    discharge: Option<DischargeEstimate>,
    discharge_unavailable_reason: Option<String>,
    sample_interval_secs: u64,
    max_gap_secs: i64,
    persistent: bool,
}

fn now_secs() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

#[tauri::command]
async fn get_system_snapshot(state: State<'_, AppState>) -> Result<SystemSnapshot, String> {
    let mut monitor = state.monitor.lock().map_err(|_| "monitor lock poisoned")?;
    Ok(monitor.snapshot())
}

#[tauri::command]
async fn get_battery(state: State<'_, AppState>) -> Result<BatteryInfo, String> {
    let mut reader = state.battery.lock().map_err(|_| "battery lock poisoned")?;
    reader.read()
}

#[tauri::command]
async fn get_battery_history(hours: u32, state: State<'_, AppState>) -> Result<BatteryHistory, String> {
    if !(1..=MAX_HISTORY_HOURS).contains(&hours) {
        return Err(format!("hours must be between 1 and {MAX_HISTORY_HOURS}"));
    }
    let history = state.history.lock().map_err(|_| "history lock poisoned")?;
    let samples = history
        .since(now_secs() - i64::from(hours) * 3600)
        .map_err(|e| format!("could not read battery history: {e}"))?;
    let (discharge, discharge_unavailable_reason) = match history::estimate_discharge(&samples) {
        Ok(e) => (Some(e), None),
        Err(reason) => (None, Some(reason)),
    };
    Ok(BatteryHistory {
        samples,
        discharge,
        discharge_unavailable_reason,
        sample_interval_secs: SAMPLE_INTERVAL.as_secs(),
        max_gap_secs: history::MAX_SAMPLE_GAP_SECS,
        persistent: state.history_persistent,
    })
}

/// Records one battery sample a minute. Runs while the app is open; nothing is
/// sampled while the Mac sleeps or MacPilot is closed.
fn spawn_battery_sampler(history: Arc<Mutex<History>>) {
    std::thread::spawn(move || loop {
        if let Ok((pm, io)) = BatteryReader::read_gauge() {
            if let (true, Some(percent)) = (pm.present, pm.percent) {
                let sample = BatterySample {
                    ts: now_secs(),
                    percent,
                    charging: matches!(pm.state, ChargeState::Charging | ChargeState::FinishingCharge),
                    external_power: io.external_connected.unwrap_or(pm.power_source.as_deref() == Some("AC Power")),
                    charge_mah: io.current_charge_mah,
                    amperage_ma: io.amperage_ma,
                    voltage_mv: io.voltage_mv,
                };
                if let Ok(h) = history.lock() {
                    if let Err(e) = h.insert(&sample) {
                        eprintln!("battery history insert failed: {e}");
                    }
                }
            }
        }
        std::thread::sleep(SAMPLE_INTERVAL);
    });
}

fn open_history(app: &tauri::App) -> (History, bool) {
    let opened = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())
        .and_then(|dir| {
            std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            History::open(&dir.join("battery_history.sqlite")).map_err(|e| e.to_string())
        });
    match opened {
        Ok(h) => {
            if let Err(e) = h.prune_before(now_secs() - RETENTION_SECS) {
                eprintln!("battery history prune failed: {e}");
            }
            (h, true)
        }
        Err(e) => {
            eprintln!("battery history not persisted ({e}); using memory only");
            (History::open_in_memory().expect("in-memory sqlite"), false)
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let (history, history_persistent) = open_history(app);
            let history = Arc::new(Mutex::new(history));
            spawn_battery_sampler(history.clone());
            app.manage(AppState {
                monitor: Mutex::new(Monitor::new()),
                battery: Mutex::new(BatteryReader::default()),
                history,
                history_persistent,
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![get_system_snapshot, get_battery, get_battery_history])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
