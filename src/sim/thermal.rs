use crate::project::HeatedTank;

use super::fluid::{Flow, Storage};

// Density is 1 kg/L in this model, so this is also the heat capacity of one litre.
const WATER_HEAT_CAPACITY_J_PER_L_K: f64 = 4180.0;

pub(super) struct Tank {
    pub(super) config: HeatedTank,
    volume_l: f64,
    temperature_c: f64,
    dirt_mg_l: f64,
}

pub(super) struct Output {
    pub temperature_c: f32,
    pub heater_w: f32,
    pub heater_blocked: bool,
    pub level_pct: f32,
}

impl Tank {
    pub(super) fn new(config: HeatedTank) -> Self {
        let volume_l = f64::from(config.starting_volume_l());
        let temperature_c = if volume_l > 0.0 {
            config.initial_temperature_c
        } else {
            config.ambient_temperature_c
        };
        Self {
            config,
            volume_l,
            temperature_c: f64::from(temperature_c),
            dirt_mg_l: f64::from(config.initial_dirt_mg_l),
        }
    }

    pub(super) fn storage(&self) -> Storage {
        Storage {
            volume_l: self.volume_l,
            capacity_l: f64::from(self.config.volume_l),
            temperature_c: self.temperature_c,
            dirt_mg_l: self.dirt_mg_l,
        }
    }

    pub(super) fn step(
        &mut self,
        dt: f64,
        mixer_on: bool,
        heater_pct: f32,
        inlet: Flow,
        outflow_l_s: f64,
    ) -> Output {
        let capacity_l = f64::from(self.config.volume_l);
        let start = self.volume_l;
        let change = (inlet.rate_l_s - outflow_l_s) * dt;
        self.volume_l = (start + change).clamp(0.0, capacity_l);

        let heater_allowed = mixer_on && start > 0.0 && self.volume_l > 0.0;
        let heater_w = if heater_allowed {
            self.config.heater_max_w * (heater_pct / 100.0)
        } else {
            0.0
        };

        let ambient = f64::from(self.config.ambient_temperature_c);
        if dt > 0.0 {
            if self.volume_l == 0.0 {
                self.dirt_mg_l = inlet.dirt_mg_l;
                self.temperature_c = if inlet.rate_l_s > 0.0 {
                    inlet.temperature_c
                } else {
                    ambient
                };
            } else if start == 0.0 {
                self.dirt_mg_l = inlet.dirt_mg_l;
                self.temperature_c = inlet.temperature_c;
            } else {
                let feed = inlet.rate_l_s * WATER_HEAT_CAPACITY_J_PER_L_K;
                let loss = f64::from(self.config.heat_loss_w_per_k);
                let net_power = f64::from(heater_w)
                    + feed * (inlet.temperature_c - self.temperature_c)
                    + loss * (ambient - self.temperature_c);

                // For fixed rates the tank volume is linear during the step. Integrate
                // 1 / volume first to apply the same thermal solution at varying levels.
                let delta = self.volume_l - start;
                let relative = delta / start;
                let inverse_volume_time = if relative == 0.0 {
                    dt / start
                } else if relative > -0.5 {
                    dt / start * (relative.ln_1p() / relative)
                } else {
                    dt * (self.volume_l / start).ln() / delta
                };
                let exchanged = -(-inlet.rate_l_s * inverse_volume_time).exp_m1();
                self.dirt_mg_l += (inlet.dirt_mg_l - self.dirt_mg_l) * exchanged;
                let gain = inverse_volume_time / WATER_HEAT_CAPACITY_J_PER_L_K;
                let decay = (feed + loss) * gain;
                let factor = if decay == 0.0 {
                    1.0
                } else {
                    -(-decay).exp_m1() / decay
                };
                self.temperature_c += net_power * gain * factor;
            }
        }

        Output {
            temperature_c: self.temperature_c as f32,
            heater_w,
            heater_blocked: !heater_allowed && heater_pct > 0.0,
            level_pct: (self.volume_l / capacity_l * 100.0) as f32,
        }
    }
}
