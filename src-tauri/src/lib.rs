mod battery;
mod history;
mod macos;
mod metrics;
mod processes;
mod tool;

use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tauri::{Manager, State};

use battery::{profiler::ProfilerBattery, BatteryInfo, BatteryReader, ChargeState};
use history::{BatterySample, DischargeEstimate, History};
use metrics::{Monitor, SystemSnapshot};

const SAMPLE_INTERVAL: Duration = Duration::from_secs(60);
const RETENTION_SECS: i64 = 30 * 24 * 3600;
const PRUNE_EVERY_SECS: i64 = 24 * 3600;
const MAX_HISTORY_HOURS: u32 = 30 * 24;

struct AppState {
    monitor: Arc<Mutex<Monitor>>,
    battery: Arc<Mutex<BatteryReader>>,
    history: Arc<Mutex<History>>,
    history_persistent: bool,
    /// Adapter state at the last battery read, to notice it being connected or removed.
    last_external_power: Arc<Mutex<Option<bool>>>,
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

/// Runs work that blocks (system calls, short waits, child processes) off the async runtime.
async fn blocking<T: Send + 'static>(work: impl FnOnce() -> Result<T, String> + Send + 'static) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(|e| format!("worker failed: {e}"))?
}

#[tauri::command]
async fn get_system_snapshot(state: State<'_, AppState>) -> Result<SystemSnapshot, String> {
    let monitor = state.monitor.clone();
    blocking(move || Ok(monitor.lock().map_err(|_| "monitor lock poisoned")?.snapshot())).await
}

#[tauri::command]
async fn get_battery(state: State<'_, AppState>) -> Result<BatteryInfo, String> {
    let (reader, history, last) = (state.battery.clone(), state.history.clone(), state.last_external_power.clone());
    blocking(move || {
        let info = reader.lock().map_err(|_| "battery lock poisoned")?.read()?;
        // Recorded now rather than at the sampler's next minute, so history and
        // the drain rate follow the adapter being connected or removed at once.
        let changed = last.lock().is_ok_and(|mut last| adapter_changed(&mut last, info.external_power));
        if changed {
            if let (Some(sample), Ok(h)) = (sample_from(&info), history.lock()) {
                store(&h, &sample);
            }
        }
        Ok(info)
    })
    .await
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

/// True when the adapter state differs from the previous known one, which is then updated.
fn adapter_changed(last: &mut Option<bool>, now: Option<bool>) -> bool {
    let Some(now) = now else { return false };
    last.replace(now).is_some_and(|before| before != now)
}

fn sample_from(info: &BatteryInfo) -> Option<BatterySample> {
    let percent = info.percent.filter(|_| info.present)?;
    Some(BatterySample {
        ts: now_secs(),
        percent,
        charging: matches!(info.state, ChargeState::Charging | ChargeState::FinishingCharge),
        external_power: info.external_power.unwrap_or(info.power_source.as_deref() == Some("AC Power")),
        charge_mah: info.current_charge_mah,
        full_charge_mah: info.full_charge_capacity_mah,
        amperage_ma: info.amperage_ma,
        voltage_mv: info.voltage_mv,
    })
}

fn store(history: &History, sample: &BatterySample) {
    if let Err(e) = history.insert(sample) {
        eprintln!("battery history insert failed: {e}");
    }
}

fn prune(history: &History) {
    if let Err(e) = history.prune_before(now_secs() - RETENTION_SECS) {
        eprintln!("battery history prune failed: {e}");
    }
}

/// Records one battery sample a minute and prunes old samples once a day. Runs
/// while the app is open; nothing is sampled while the Mac sleeps or MacPilot
/// is closed.
fn spawn_battery_sampler(history: Arc<Mutex<History>>) {
    let mut last_prune = now_secs();
    std::thread::spawn(move || loop {
        match BatteryReader::read_gauge() {
            Err(e) => eprintln!("battery sample skipped: {e}"),
            Ok((pm, io)) => {
                if let (Some(sample), Ok(h)) = (sample_from(&battery::combine(&pm, &io, &ProfilerBattery::default())), history.lock()) {
                    store(&h, &sample);
                    if sample.ts - last_prune >= PRUNE_EVERY_SECS {
                        prune(&h);
                        last_prune = sample.ts;
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
            prune(&h);
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
        .setup(|app| {
            let (history, history_persistent) = open_history(app);
            let history = Arc::new(Mutex::new(history));
            spawn_battery_sampler(history.clone());
            app.manage(AppState {
                monitor: Arc::new(Mutex::new(Monitor::new())),
                battery: Arc::new(Mutex::new(BatteryReader::default())),
                history,
                history_persistent,
                last_external_power: Arc::new(Mutex::new(None)),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![get_system_snapshot, get_battery, get_battery_history])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adapter_change_is_noticed_only_between_two_known_states() {
        let mut last = None;
        assert!(!adapter_changed(&mut last, Some(true)), "first reading only sets the baseline");
        assert!(!adapter_changed(&mut last, Some(true)));
        assert!(!adapter_changed(&mut last, None), "an unknown reading changes nothing");
        assert_eq!(last, Some(true));
        assert!(adapter_changed(&mut last, Some(false)));
        assert!(adapter_changed(&mut last, Some(true)));
    }
}
