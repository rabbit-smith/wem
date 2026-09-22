//! Immutable values passed between scheduling, analysis and packet layers.
//!
//! Mirrors Python `wwise_wem/analysis/model.py`.

use crate::preprocessing::windowing::WindowedFrame;

/// Channel-major transform rows (Python `FloatRows`).
pub type FloatRows = Vec<Vec<f64>>;

/// Transform-domain values and peak state for one windowed frame
/// (Python `SpectrumFrame`).
#[derive(Debug, Clone, PartialEq)]
pub struct SpectrumFrame {
    pub window: WindowedFrame,
    pub coefficients: FloatRows,
    pub raw_mdct: FloatRows,
    pub fft: FloatRows,
    pub channel_specmax: Vec<f64>,
    pub global_specmax: f64,
}

/// Psychoacoustic surfaces paired with their source spectrum
/// (Python `PsyFrame`).
#[derive(Debug, Clone, PartialEq)]
pub struct PsyFrame {
    pub spectrum: SpectrumFrame,
    pub remap: FloatRows,
    pub seed: FloatRows,
    pub post: FloatRows,
    pub side: FloatRows,
    pub coupling_peak: FloatRows,
}

impl PsyFrame {
    /// The source windowed frame (Python `window` property).
    pub fn window(&self) -> &WindowedFrame {
        &self.spectrum.window
    }
    /// Consume the frame and hand back its source windowed frame, so a
    /// caller that materializes one frame at a time can lend those row
    /// buffers to the next frame instead of allocating new ones (the
    /// scratch hand-back of
    /// [`PlannedWindowSource::materialize`](crate::preprocessing::windowing::PlannedWindowSource::materialize)).
    pub fn into_window(self) -> WindowedFrame {
        self.spectrum.window
    }
    /// The MDCT coefficients (Python `coefficients` property).
    pub fn coefficients(&self) -> &FloatRows {
        &self.spectrum.coefficients
    }
    /// The log-domain MDCT curve (Python `raw_mdct` property).
    pub fn raw_mdct(&self) -> &FloatRows {
        &self.spectrum.raw_mdct
    }
    /// The log-domain FFT curve (Python `fft` property).
    pub fn fft(&self) -> &FloatRows {
        &self.spectrum.fft
    }
    /// Per-channel spectrum maxima (Python `channel_specmax` property).
    pub fn channel_specmax(&self) -> &[f64] {
        &self.spectrum.channel_specmax
    }
    /// The global spectrum maximum (Python `global_specmax` property).
    pub fn global_specmax(&self) -> f64 {
        self.spectrum.global_specmax
    }
}
