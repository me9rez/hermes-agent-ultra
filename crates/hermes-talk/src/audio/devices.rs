use cpal::traits::{DeviceTrait, HostTrait};
use tracing::warn;

use crate::config::AudioConfig;
use crate::error::{DemoError, Result};

const NO_INPUT_HINT: &str = "no input device available — connect or enable a microphone \
(Windows: Settings > System > Sound > Input; enable disabled devices in Sound Control Panel), \
then run `hermes talk list-devices` and set [audio].input_device if needed";

const NO_OUTPUT_HINT: &str = "no output device available — connect speakers or headphones, \
then run `hermes talk list-devices` and set [audio].output_device if needed";

pub fn pick_input_device(host: &cpal::Host, cfg: &AudioConfig) -> Result<cpal::Device> {
    if !cfg.input_device.is_empty() {
        return find_named_input(host, &cfg.input_device);
    }
    if let Some(dev) = host.default_input_device() {
        return Ok(dev);
    }
    let devices: Vec<_> = host
        .input_devices()
        .map_err(|e| DemoError::Audio(e.to_string()))?
        .collect();
    if let Some(dev) = devices.into_iter().next() {
        let name = dev.name().unwrap_or_else(|_| "?".into());
        warn!(device = %name, "no default input device; using first available");
        return Ok(dev);
    }
    Err(DemoError::Audio(NO_INPUT_HINT.into()))
}

pub fn pick_output_device(host: &cpal::Host, cfg: &AudioConfig) -> Result<cpal::Device> {
    if !cfg.output_device.is_empty() {
        return find_named_output(host, &cfg.output_device);
    }
    if let Some(dev) = host.default_output_device() {
        return Ok(dev);
    }
    let devices: Vec<_> = host
        .output_devices()
        .map_err(|e| DemoError::Audio(e.to_string()))?
        .collect();
    if let Some(dev) = devices.into_iter().next() {
        let name = dev.name().unwrap_or_else(|_| "?".into());
        warn!(device = %name, "no default output device; using first available");
        return Ok(dev);
    }
    Err(DemoError::Audio(NO_OUTPUT_HINT.into()))
}

fn find_named_input(host: &cpal::Host, name: &str) -> Result<cpal::Device> {
    host.input_devices()
        .map_err(|e| DemoError::Audio(e.to_string()))?
        .find(|d| d.name().map(|n| n == name).unwrap_or(false))
        .ok_or_else(|| DemoError::Audio(format!("input device not found: {name}")))
}

fn find_named_output(host: &cpal::Host, name: &str) -> Result<cpal::Device> {
    host.output_devices()
        .map_err(|e| DemoError::Audio(e.to_string()))?
        .find(|d| d.name().map(|n| n == name).unwrap_or(false))
        .ok_or_else(|| DemoError::Audio(format!("output device not found: {name}")))
}
