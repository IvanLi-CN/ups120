use defmt::info;
use embassy_sync::{
    blocking_mutex::raw::NoopRawMutex,
    mutex::Mutex,
};
use embassy_time::{Duration, Timer};
use static_cell::StaticCell;

use crate::{fan_control, power::PowerStateMutex, tsens};

/// Aggregated thermal and fan-control state exposed to UI and power tasks.
///
/// Temperatures are represented as degrees Celsius. All fields are optional so
/// that callers can distinguish “no data yet” from a valid reading.
#[derive(Clone, Copy)]
pub struct ThermalState {
    /// UPS temperature derived from SC8815 ADIN (°C), if available.
    pub ups_temp_c: Option<f32>,
    /// Smart-battery pack temperature (°C), if available.
    pub sb_pack_temp_c: Option<f32>,
    /// Smart-battery charger/FET temperature (°C), if available.
    pub sb_charger_temp_c: Option<f32>,
    /// Latest fan-control status snapshot.
    pub fan: fan_control::FanStatus,
}

impl Default for ThermalState {
    fn default() -> Self {
        Self {
            ups_temp_c: None,
            sb_pack_temp_c: None,
            sb_charger_temp_c: None,
            fan: fan_control::FanStatus::default(),
        }
    }
}

pub type ThermalStateMutex = Mutex<NoopRawMutex, ThermalState>;

static THERMAL_STATE: StaticCell<ThermalStateMutex> = StaticCell::new();

/// Initialise the global thermal-state mutex.
pub fn init_thermal_state() -> &'static ThermalStateMutex {
    THERMAL_STATE.init(Mutex::new(ThermalState::default()))
}

/// Asynchronous thermal-management task responsible for:
///   * Periodic TSENS sampling.
///   * Combining TSENS and smart-battery temperatures for fan control.
///   * Publishing a compact [`ThermalState`] snapshot for UI/power tasks.
#[embassy_executor::task]
pub async fn thermal_task(
    mut controller: fan_control::FanController<'static>,
    power_state: &'static PowerStateMutex,
    thermal_state: &'static ThermalStateMutex,
) {
    info!("thermal: task started (period={} ms)", fan_control::SAMPLE_PERIOD_MS);

    loop {
        // TSENS sampling (async, non-blocking for other Embassy tasks).
        let reading = tsens::read_celsius_async().await;

        // Snapshot power-layer temperatures and VIN state for this control step.
        let (sb_temps, ups_temp_c, vin_present) = {
            let state = power_state.lock().await;
            (state.smart_batt_temps, state.adin_temp_c, state.ac_present)
        };

        controller.set_vin_present(vin_present);
        controller.tick(reading, sb_temps);

        let fan_status = controller.status();
        let (pack_c, charger_c) = match sb_temps {
            Some(t) => (t.pack_c, t.charger_c),
            None => (None, None),
        };

        // Publish current snapshot for UI and power tasks.
        {
            let mut state = thermal_state.lock().await;
            *state = ThermalState {
                ups_temp_c,
                sb_pack_temp_c: pack_c,
                sb_charger_temp_c: charger_c,
                fan: fan_status,
            };
        }

        // Maintain the legacy 20 ms control-loop cadence.
        Timer::after(Duration::from_millis(fan_control::SAMPLE_PERIOD_MS.into())).await;
    }
}
